use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::cli::StartTab;
use crate::mise::MiseClient;
use crate::mise::schema::MiseTask;
use crate::model::task_tree::{TaskRow, build_task_rows};
use crate::model::template::{WorkspaceTemplate, discover_templates};
use crate::model::{EnvRow, ToolRow};
use crate::runner::{BackgroundRunner, RunStatus, RunnerEvent, TaskRunRecord, copy_to_clipboard};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Tasks,
    Tools,
    Env,
    Templates,
}

impl From<StartTab> for Tab {
    fn from(v: StartTab) -> Self {
        match v {
            StartTab::Tasks => Tab::Tasks,
            StartTab::Tools => Tab::Tools,
            StartTab::Env => Tab::Env,
            StartTab::Templates => Tab::Templates,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPane {
    List,
    Preview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskPreviewMode {
    Spec,
    Output,
}

#[derive(Debug, Clone)]
pub enum PendingAction {
    MiseInstall,
    MiseUpgrade(String),
    MisePrune,
    ScaffoldTemplate { template: String, name: String },
}

#[derive(Debug, Clone)]
pub enum Overlay {
    None,
    Help,
    Filter,
    Confirm {
        title: String,
        command_preview: String,
        action: PendingAction,
    },
    TemplateNameInput {
        template: String,
        input: String,
    },
}

#[derive(Debug, Clone)]
pub enum ForegroundRequest {
    RunTask(String),
    WatchTask(String),
    EditFile(PathBuf),
    RunCommand {
        program: String,
        args: Vec<String>,
        wait_for_enter: bool,
        reload_after: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingChord {
    None,
    G,
    Z,
}

pub struct App {
    pub client: MiseClient,
    pub tab: Tab,
    pub focus: FocusPane,
    pub task_preview_mode: TaskPreviewMode,
    pub overlay: Overlay,
    pub filter: String,
    pub should_quit: bool,

    // Raw & derived data
    pub tasks: Vec<MiseTask>,
    pub collapsed_groups: BTreeSet<String>,
    pub task_rows: Vec<TaskRow>,
    pub tools: Vec<ToolRow>,
    pub env_rows: Vec<EnvRow>,
    pub templates: Vec<WorkspaceTemplate>,

    // Selection & scroll state
    pub task_idx: usize,
    pub tool_idx: usize,
    pub env_idx: usize,
    pub template_idx: usize,
    pub preview_scroll: u16,

    // Runtime state
    pub revealed_env_keys: BTreeSet<String>,
    pub run_records: HashMap<String, TaskRunRecord>,
    pub last_task: Option<String>,
    pub runner: BackgroundRunner,
    pub status_msg: String,
    pub pending_fg: Option<ForegroundRequest>,
    chord: PendingChord,
}

impl App {
    pub fn new(workdir: PathBuf, start_tab: StartTab) -> Self {
        let client = MiseClient::new(workdir);
        let mut app = Self {
            client,
            tab: start_tab.into(),
            focus: FocusPane::List,
            task_preview_mode: TaskPreviewMode::Spec,
            overlay: Overlay::None,
            filter: String::new(),
            should_quit: false,
            tasks: Vec::new(),
            collapsed_groups: BTreeSet::new(),
            task_rows: Vec::new(),
            tools: Vec::new(),
            env_rows: Vec::new(),
            templates: Vec::new(),
            task_idx: 0,
            tool_idx: 0,
            env_idx: 0,
            template_idx: 0,
            preview_scroll: 0,
            revealed_env_keys: BTreeSet::new(),
            run_records: HashMap::new(),
            last_task: None,
            runner: BackgroundRunner::new(),
            status_msg: String::new(),
            pending_fg: None,
            chord: PendingChord::None,
        };
        app.reload_all(true);
        app
    }

    pub fn reload_all(&mut self, initial: bool) {
        let mut notes = Vec::new();

        match self.client.list_tasks() {
            Ok(tasks) => {
                if initial {
                    let has_local = tasks.iter().any(|t| !t.global);
                    if has_local {
                        self.collapsed_groups.insert("[global]".to_string());
                    }
                }
                self.tasks = tasks;
                self.rebuild_task_rows();
                if initial {
                    // Focus first concrete task rather than group header
                    if let Some(pos) = self.task_rows.iter().position(|r| r.as_task().is_some()) {
                        self.task_idx = pos;
                    }
                }
            }
            Err(err) => notes.push(format!("tasks: {err}")),
        }

        match self.client.list_tools() {
            Ok(map) => {
                let mut rows = Vec::new();
                for (name, entries) in map {
                    for entry in entries {
                        rows.push(ToolRow {
                            name: name.clone(),
                            entry,
                        });
                    }
                }
                // Active tools first, then alphabetical
                rows.sort_by(|a, b| {
                    b.entry
                        .active
                        .cmp(&a.entry.active)
                        .then_with(|| a.name.cmp(&b.name))
                });
                self.tools = rows;
                self.clamp_indices();
            }
            Err(err) => notes.push(format!("tools: {err}")),
        }

        match self.client.list_env() {
            Ok(map) => {
                self.env_rows = map
                    .into_iter()
                    .map(|(key, value)| {
                        let sensitive = EnvRow::is_sensitive_key(&key);
                        EnvRow {
                            key,
                            value,
                            sensitive,
                        }
                    })
                    .collect();
                self.clamp_indices();
            }
            Err(err) => notes.push(format!("env: {err}")),
        }

        self.templates = discover_templates();
        self.clamp_indices();

        if notes.is_empty() {
            self.status_msg = format!(
                "Loaded {} tasks, {} tools, {} env vars, {} templates",
                self.tasks.len(),
                self.tools.len(),
                self.env_rows.len(),
                self.templates.len()
            );
        } else {
            self.status_msg = format!("Partial load: {}", notes.join(" | "));
        }
    }

    pub fn rebuild_task_rows(&mut self) {
        self.task_rows = build_task_rows(&self.tasks, &self.collapsed_groups, &self.filter);
        self.clamp_indices();
    }

    pub fn filtered_tools(&self) -> Vec<&ToolRow> {
        let q = self.filter.trim().to_ascii_lowercase();
        if q.is_empty() {
            self.tools.iter().collect()
        } else {
            self.tools
                .iter()
                .filter(|r| {
                    r.name.to_ascii_lowercase().contains(&q)
                        || r.entry.version.to_ascii_lowercase().contains(&q)
                })
                .collect()
        }
    }

    pub fn filtered_env(&self) -> Vec<&EnvRow> {
        let q = self.filter.trim().to_ascii_lowercase();
        if q.is_empty() {
            self.env_rows.iter().collect()
        } else {
            self.env_rows
                .iter()
                .filter(|r| {
                    r.key.to_ascii_lowercase().contains(&q)
                        || (!r.sensitive && r.value.to_ascii_lowercase().contains(&q))
                })
                .collect()
        }
    }

    pub fn filtered_templates(&self) -> Vec<&WorkspaceTemplate> {
        let q = self.filter.trim().to_ascii_lowercase();
        if q.is_empty() {
            self.templates.iter().collect()
        } else {
            self.templates
                .iter()
                .filter(|t| t.name.to_ascii_lowercase().contains(&q))
                .collect()
        }
    }

    pub fn selected_task(&self) -> Option<&MiseTask> {
        self.task_rows.get(self.task_idx).and_then(TaskRow::as_task)
    }

    pub fn selected_task_script(&self) -> String {
        let Some(task) = self.selected_task() else {
            return String::new();
        };
        if !task.run.is_empty() {
            return task.run.join("\n");
        }
        let path = task.file.as_deref().unwrap_or(&task.source);
        if !path.is_empty()
            && !path.ends_with(".toml")
            && let Ok(content) = fs::read_to_string(path)
        {
            return content;
        }
        "(inline or external task definition)".to_string()
    }

    pub fn poll_runner(&mut self) {
        for ev in self.runner.drain_events() {
            match ev {
                RunnerEvent::Line { task, line } => {
                    let rec = self
                        .run_records
                        .entry(task)
                        .or_insert_with(|| TaskRunRecord {
                            status: RunStatus::Running,
                            lines: Vec::new(),
                        });
                    rec.push_line(line);
                }
                RunnerEvent::Finished {
                    task,
                    exit_code,
                    duration_ms,
                } => {
                    let status = if exit_code == 0 {
                        RunStatus::Success { duration_ms }
                    } else {
                        RunStatus::Failed {
                            code: exit_code,
                            duration_ms,
                        }
                    };
                    let rec = self
                        .run_records
                        .entry(task.clone())
                        .or_insert_with(|| TaskRunRecord {
                            status: status.clone(),
                            lines: Vec::new(),
                        });
                    rec.status = status;
                    self.status_msg = format!(
                        "Task `{task}` finished (exit {exit_code}, {:.2}s)",
                        duration_ms as f64 / 1000.0
                    );
                }
            }
        }
    }

    pub fn record_foreground_result(&mut self, task: &str, code: i32, duration_ms: u128) {
        let status = if code == 0 {
            RunStatus::Success { duration_ms }
        } else {
            RunStatus::Failed { code, duration_ms }
        };
        self.run_records
            .entry(task.to_string())
            .and_modify(|r| r.status = status.clone())
            .or_insert_with(|| TaskRunRecord {
                status,
                lines: vec![format!(
                    "(ran in foreground TTY; exit {code}, {:.2}s)",
                    duration_ms as f64 / 1000.0
                )],
            });
        self.status_msg = format!(
            "Task `{task}` exited {code} ({:.2}s)",
            duration_ms as f64 / 1000.0
        );
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        // Ctrl-C always exits cleanly
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        match self.overlay.clone() {
            Overlay::Help => {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter
                ) {
                    self.overlay = Overlay::None;
                }
                return;
            }
            Overlay::Filter => {
                self.handle_filter_key(key);
                return;
            }
            Overlay::Confirm { action, .. } => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        self.overlay = Overlay::None;
                        self.execute_confirmed(action);
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.overlay = Overlay::None;
                        self.status_msg = "Action cancelled".to_string();
                    }
                    _ => {}
                }
                return;
            }
            Overlay::TemplateNameInput { template, mut input } => {
                match key.code {
                    KeyCode::Esc => {
                        self.overlay = Overlay::None;
                    }
                    KeyCode::Backspace => {
                        input.pop();
                        self.overlay = Overlay::TemplateNameInput { template, input };
                    }
                    KeyCode::Enter => {
                        let name = input.trim().to_string();
                        if name.is_empty() || name.contains('/') || name.starts_with('.') {
                            self.status_msg =
                                "Project name must be a non-empty single directory name"
                                    .to_string();
                            self.overlay = Overlay::TemplateNameInput { template, input };
                        } else {
                            let cmd = format!("mise run new {template} {name}");
                            self.overlay = Overlay::Confirm {
                                title: format!("Scaffold new `{template}` workspace"),
                                command_preview: cmd,
                                action: PendingAction::ScaffoldTemplate { template, name },
                            };
                        }
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        input.push(c);
                        self.overlay = Overlay::TemplateNameInput { template, input };
                    }
                    _ => {}
                }
                return;
            }
            Overlay::None => {}
        }

        // Handle two-key Vim chords (gg, za)
        if self.chord != PendingChord::None {
            let chord = self.chord;
            self.chord = PendingChord::None;
            match (chord, key.code) {
                (PendingChord::G, KeyCode::Char('g')) => {
                    self.move_top();
                    return;
                }
                (PendingChord::Z, KeyCode::Char('a')) => {
                    if self.tab == Tab::Tasks {
                        self.toggle_current_group_fold();
                    }
                    return;
                }
                _ => {}
            }
        }

        // Ctrl-d / Ctrl-u half-page scrolling
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('d') => {
                    self.move_delta(10);
                    return;
                }
                KeyCode::Char('u') => {
                    self.move_delta(-10);
                    return;
                }
                _ => {}
            }
        }

        // Global navigation & actions
        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('?') => {
                self.overlay = Overlay::Help;
                return;
            }
            KeyCode::Char('1') => {
                self.switch_tab(Tab::Tasks);
                return;
            }
            KeyCode::Char('2') => {
                self.switch_tab(Tab::Tools);
                return;
            }
            KeyCode::Char('3') => {
                self.switch_tab(Tab::Env);
                return;
            }
            KeyCode::Char('4') => {
                self.switch_tab(Tab::Templates);
                return;
            }
            KeyCode::Tab => {
                self.focus = match self.focus {
                    FocusPane::List => FocusPane::Preview,
                    FocusPane::Preview => FocusPane::List,
                };
                return;
            }
            KeyCode::Char('h') | KeyCode::Left => {
                self.focus = FocusPane::List;
                return;
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.focus = FocusPane::Preview;
                return;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_delta(1);
                return;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_delta(-1);
                return;
            }
            KeyCode::Char('g') => {
                self.chord = PendingChord::G;
                return;
            }
            KeyCode::Char('z') => {
                self.chord = PendingChord::Z;
                return;
            }
            KeyCode::Char('G') => {
                self.move_bottom();
                return;
            }
            KeyCode::Char('/') => {
                self.overlay = Overlay::Filter;
                return;
            }
            KeyCode::Esc => {
                if !self.filter.is_empty() {
                    self.filter.clear();
                    self.rebuild_task_rows();
                    self.clamp_indices();
                    self.status_msg = "Filter cleared".to_string();
                }
                return;
            }
            KeyCode::Char('R') => {
                self.reload_all(false);
                return;
            }
            _ => {}
        }

        // Tab-specific actions
        match self.tab {
            Tab::Tasks => self.handle_tasks_key(key.code),
            Tab::Tools => self.handle_tools_key(key.code),
            Tab::Env => self.handle_env_key(key.code),
            Tab::Templates => self.handle_templates_key(key.code),
        }
    }

    fn handle_filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                self.overlay = Overlay::None;
            }
            KeyCode::Esc => {
                self.filter.clear();
                self.rebuild_task_rows();
                self.clamp_indices();
                self.overlay = Overlay::None;
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.rebuild_task_rows();
                self.clamp_indices();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.filter.push(c);
                self.rebuild_task_rows();
                self.clamp_indices();
            }
            _ => {}
        }
    }

    fn handle_tasks_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char(' ') => self.toggle_current_group_fold(),
            KeyCode::Char('p') => {
                self.task_preview_mode = match self.task_preview_mode {
                    TaskPreviewMode::Spec => TaskPreviewMode::Output,
                    TaskPreviewMode::Output => TaskPreviewMode::Spec,
                };
                self.preview_scroll = 0;
            }
            KeyCode::Enter => {
                if let Some(row) = self.task_rows.get(self.task_idx).cloned() {
                    match row {
                        TaskRow::Group { .. } => self.toggle_current_group_fold(),
                        TaskRow::Task(task) => {
                            self.last_task = Some(task.name.clone());
                            self.pending_fg = Some(ForegroundRequest::RunTask(task.name));
                        }
                    }
                }
            }
            KeyCode::Char('x') => {
                if let Some(task) = self.selected_task().cloned() {
                    self.start_background_task(task.name);
                }
            }
            KeyCode::Char('r') => {
                if let Some(name) = self.last_task.clone() {
                    self.pending_fg = Some(ForegroundRequest::RunTask(name));
                } else {
                    self.status_msg = "No previous task to re-run".to_string();
                }
            }
            KeyCode::Char('w') => {
                if let Some(task) = self.selected_task().cloned() {
                    self.last_task = Some(task.name.clone());
                    self.pending_fg = Some(ForegroundRequest::WatchTask(task.name));
                }
            }
            KeyCode::Char('e') => {
                if let Some(task) = self.selected_task().cloned() {
                    let target = task.file.unwrap_or(task.source);
                    if target.is_empty() {
                        self.status_msg = "Task has no editable source file".to_string();
                    } else {
                        self.pending_fg = Some(ForegroundRequest::EditFile(PathBuf::from(target)));
                    }
                }
            }
            KeyCode::Char('y') => {
                if let Some(task) = self.selected_task() {
                    let cmd = format!("mise run {}", task.name);
                    match copy_to_clipboard(&cmd) {
                        Ok(()) => self.status_msg = format!("Copied: {cmd}"),
                        Err(e) => self.status_msg = format!("Clipboard error: {e}"),
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_tools_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('i') => {
                self.overlay = Overlay::Confirm {
                    title: "Install configured mise tools".to_string(),
                    command_preview: "mise install".to_string(),
                    action: PendingAction::MiseInstall,
                };
            }
            KeyCode::Char('u') => {
                if let Some(tool) = self.filtered_tools().get(self.tool_idx) {
                    let name = tool.name.clone();
                    self.overlay = Overlay::Confirm {
                        title: format!("Upgrade tool `{name}`"),
                        command_preview: format!("mise upgrade {name}"),
                        action: PendingAction::MiseUpgrade(name),
                    };
                }
            }
            KeyCode::Char('p') => {
                self.overlay = Overlay::Confirm {
                    title: "Prune unused mise tool versions".to_string(),
                    command_preview: "mise prune".to_string(),
                    action: PendingAction::MisePrune,
                };
            }
            _ => {}
        }
    }

    fn handle_env_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('S') => {
                if let Some(row) = self.filtered_env().get(self.env_idx) {
                    let key = row.key.clone();
                    if self.revealed_env_keys.contains(&key) {
                        self.revealed_env_keys.remove(&key);
                    } else {
                        self.revealed_env_keys.insert(key);
                    }
                }
            }
            KeyCode::Char('y') => {
                if let Some(row) = self.filtered_env().get(self.env_idx) {
                    let val = row.value.clone();
                    let key = row.key.clone();
                    match copy_to_clipboard(&val) {
                        Ok(()) => self.status_msg = format!("Copied value of {key}"),
                        Err(e) => self.status_msg = format!("Clipboard error: {e}"),
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_templates_key(&mut self, code: KeyCode) {
        if matches!(code, KeyCode::Enter | KeyCode::Char('n'))
            && let Some(tpl) = self.filtered_templates().get(self.template_idx)
        {
            self.overlay = Overlay::TemplateNameInput {
                template: tpl.name.clone(),
                input: String::new(),
            };
        }
    }

    fn execute_confirmed(&mut self, action: PendingAction) {
        let req = match action {
            PendingAction::MiseInstall => ForegroundRequest::RunCommand {
                program: "mise".to_string(),
                args: vec!["install".to_string()],
                wait_for_enter: true,
                reload_after: true,
            },
            PendingAction::MiseUpgrade(name) => ForegroundRequest::RunCommand {
                program: "mise".to_string(),
                args: vec!["upgrade".to_string(), name],
                wait_for_enter: true,
                reload_after: true,
            },
            PendingAction::MisePrune => ForegroundRequest::RunCommand {
                program: "mise".to_string(),
                args: vec!["prune".to_string()],
                wait_for_enter: true,
                reload_after: true,
            },
            PendingAction::ScaffoldTemplate { template, name } => ForegroundRequest::RunCommand {
                program: "mise".to_string(),
                args: vec!["run".to_string(), "new".to_string(), template, name],
                wait_for_enter: true,
                reload_after: false,
            },
        };
        self.pending_fg = Some(req);
    }

    fn start_background_task(&mut self, task_name: String) {
        self.last_task = Some(task_name.clone());
        self.task_preview_mode = TaskPreviewMode::Output;
        self.preview_scroll = 0;
        self.run_records.insert(
            task_name.clone(),
            TaskRunRecord {
                status: RunStatus::Running,
                lines: vec![format!("$ mise run {task_name}")],
            },
        );
        self.status_msg = format!("Running `{task_name}` in background...");
        self.runner
            .spawn_mise_task(self.client.workdir().to_path_buf(), task_name);
    }

    fn toggle_current_group_fold(&mut self) {
        let key = match self.task_rows.get(self.task_idx) {
            Some(TaskRow::Group { key, .. }) => Some(key.clone()),
            Some(TaskRow::Task(t)) => Some(t.group_key()),
            None => None,
        };
        if let Some(k) = key {
            if self.collapsed_groups.contains(&k) {
                self.collapsed_groups.remove(&k);
            } else {
                self.collapsed_groups.insert(k.clone());
            }
            self.rebuild_task_rows();
            if let Some(pos) = self
                .task_rows
                .iter()
                .position(|r| r.group_key() == Some(k.as_str()))
            {
                self.task_idx = pos;
            }
        }
    }

    fn switch_tab(&mut self, tab: Tab) {
        if self.tab != tab {
            self.tab = tab;
            self.focus = FocusPane::List;
            self.preview_scroll = 0;
            self.clamp_indices();
        }
    }

    fn current_list_len(&self) -> usize {
        match self.tab {
            Tab::Tasks => self.task_rows.len(),
            Tab::Tools => self.filtered_tools().len(),
            Tab::Env => self.filtered_env().len(),
            Tab::Templates => self.filtered_templates().len(),
        }
    }

    fn current_idx_mut(&mut self) -> &mut usize {
        match self.tab {
            Tab::Tasks => &mut self.task_idx,
            Tab::Tools => &mut self.tool_idx,
            Tab::Env => &mut self.env_idx,
            Tab::Templates => &mut self.template_idx,
        }
    }

    fn clamp_indices(&mut self) {
        let clamp = |idx: &mut usize, len: usize| {
            if len == 0 {
                *idx = 0;
            } else if *idx >= len {
                *idx = len - 1;
            }
        };
        clamp(&mut self.task_idx, self.task_rows.len());
        let t_len = self.filtered_tools().len();
        clamp(&mut self.tool_idx, t_len);
        let e_len = self.filtered_env().len();
        clamp(&mut self.env_idx, e_len);
        let tpl_len = self.filtered_templates().len();
        clamp(&mut self.template_idx, tpl_len);
    }

    fn move_delta(&mut self, delta: isize) {
        match self.focus {
            FocusPane::Preview => {
                if delta > 0 {
                    self.preview_scroll = self.preview_scroll.saturating_add(delta as u16);
                } else {
                    self.preview_scroll = self.preview_scroll.saturating_sub((-delta) as u16);
                }
            }
            FocusPane::List => {
                let len = self.current_list_len();
                if len == 0 {
                    return;
                }
                let idx = self.current_idx_mut();
                let next = (*idx as isize + delta).clamp(0, (len - 1) as isize) as usize;
                *idx = next;
                self.preview_scroll = 0;
            }
        }
    }

    fn move_top(&mut self) {
        match self.focus {
            FocusPane::Preview => self.preview_scroll = 0,
            FocusPane::List => {
                *self.current_idx_mut() = 0;
                self.preview_scroll = 0;
            }
        }
    }

    fn move_bottom(&mut self) {
        match self.focus {
            FocusPane::Preview => self.preview_scroll = self.preview_scroll.saturating_add(200),
            FocusPane::List => {
                let len = self.current_list_len();
                if len > 0 {
                    *self.current_idx_mut() = len - 1;
                    self.preview_scroll = 0;
                }
            }
        }
    }
}
