use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::widgets::ListState;

use crate::action::{
    Action, ConfigSubView, Domain, EnvFilter, EnvSubView, FocusArea, LogPanelMode, Panel,
    ProfileSpec, ScreenMode, TaskSubView, ToastLevel, ToolScope, ToolSubView,
};
use crate::events::{CommandLogEntry, CommandSource, JobId, JobOutputLine, JobRecord, JobStatus};
use crate::mise::MiseClient;
use crate::mise::schema::{MiseTask, RegistryEntry, TaskHistoryEntry};
use crate::model::config::ConfigViewModel;
use crate::model::env::EnvViewModel;
use crate::model::fuzzy::fuzzy_match;
use crate::model::task_tree::{ScriptCache, TaskRow, TaskTree};
use crate::model::template::{ScaffoldKind, WorkspaceTemplate, build_scaffold_catalog};
use crate::model::tools::{RegistryMatch, ToolViewModel, filter_registry_entries};
use crate::runner::CommandLog;

/// Default auto-re-mask duration for revealed secrets in the Env panel.
pub const SECRET_REVEAL_TTL: Duration = Duration::from_secs(30);

/// Double-click detection threshold for mouse interactions.
pub const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(400);

/// Double Ctrl-C force-quit window when background jobs are still running.
pub const FORCE_QUIT_WINDOW: Duration = Duration::from_secs(3);

/// Transient notification shown in the footer / toast banner.
#[derive(Debug, Clone)]
pub struct Toast {
    pub level: ToastLevel,
    pub message: String,
    pub expires_at: Instant,
}

impl Toast {
    pub fn new(level: ToastLevel, message: impl Into<String>, ttl: Duration) -> Self {
        Self {
            level,
            message: message.into(),
            expires_at: Instant::now() + ttl,
        }
    }
}

/// Bounded viewport scroll state for the right detail pane.
#[derive(Debug, Default, Clone, Copy)]
pub struct ScrollState {
    pub offset: u16,
    pub content_len: u16,
    pub viewport_height: u16,
}

impl ScrollState {
    pub fn max_offset(&self) -> u16 {
        self.content_len.saturating_sub(self.viewport_height)
    }

    pub fn clamp(&mut self) {
        let max = self.max_offset();
        if self.offset > max {
            self.offset = max;
        }
    }

    pub fn scroll_by(&mut self, delta: isize) {
        if delta > 0 {
            self.offset = self
                .offset
                .saturating_add(delta as u16)
                .min(self.max_offset());
        } else {
            self.offset = self.offset.saturating_sub((-delta) as u16);
        }
    }

    pub fn scroll_top(&mut self) {
        self.offset = 0;
    }

    pub fn scroll_bottom(&mut self) {
        self.offset = self.max_offset();
    }

    pub fn half_page(&self) -> isize {
        (self.viewport_height / 2).max(1) as isize
    }
}

/// Editable text input buffer with cursor index (Unicode char offset).
#[derive(Debug, Default, Clone)]
pub struct TextInput {
    pub value: String,
    pub cursor: usize,
}

impl TextInput {
    pub fn new(initial: impl Into<String>) -> Self {
        let value = initial.into();
        let cursor = value.chars().count();
        Self { value, cursor }
    }

    pub fn insert_char(&mut self, c: char) {
        let byte_idx = self.byte_index_at(self.cursor);
        self.value.insert(byte_idx, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let start = self.byte_index_at(self.cursor - 1);
            let end = self.byte_index_at(self.cursor);
            self.value.replace_range(start..end, "");
            self.cursor -= 1;
        }
    }

    pub fn delete_char(&mut self) {
        let len = self.value.chars().count();
        if self.cursor < len {
            let start = self.byte_index_at(self.cursor);
            let end = self.byte_index_at(self.cursor + 1);
            self.value.replace_range(start..end, "");
        }
    }

    pub fn delete_word_back(&mut self) {
        while self.cursor > 0 {
            let ch = self.value.chars().nth(self.cursor - 1);
            if ch.is_some_and(char::is_whitespace) {
                self.backspace();
            } else {
                break;
            }
        }
        while self.cursor > 0 {
            let ch = self.value.chars().nth(self.cursor - 1);
            if ch.is_some_and(char::is_whitespace) {
                break;
            }
            self.backspace();
        }
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        let len = self.value.chars().count();
        self.cursor = (self.cursor + 1).min(len);
    }

    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor = self.value.chars().count();
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    fn byte_index_at(&self, char_idx: usize) -> usize {
        self.value
            .char_indices()
            .nth(char_idx)
            .map(|(i, _)| i)
            .unwrap_or(self.value.len())
    }
}

/// Item displayed inside a Context Menu (`x`) or Command Palette (`:` / `Ctrl-p`).
#[derive(Debug, Clone)]
pub struct MenuItem {
    pub key_hint: String,
    pub label: String,
    pub description: String,
    pub action: Action,
    pub command_preview: Option<String>,
}

/// Pending high-impact / mutating action awaiting explicit confirmation.
#[derive(Debug, Clone)]
pub enum ConfirmAction {
    MiseCommand {
        args: Vec<String>,
        foreground: bool,
        reload_after: bool,
        label: String,
    },
    QuitWithRunningJobs,
    ScaffoldTemplate {
        template: String,
        name: String,
    },
}

/// Interactive step inside the Registry fuzzy search & install modal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryStep {
    SearchTool,
    PickVersion {
        tool: String,
        versions: Vec<String>,
        loading: bool,
    },
    PickScope {
        tool: String,
        version: String,
    },
}

/// Structured input field for `mise tasks info` `usage_spec` arguments/flags.
#[derive(Debug, Clone)]
pub struct ArgField {
    pub name: String,
    pub flag: Option<String>,
    pub help: String,
    pub required: bool,
    pub input: TextInput,
}

/// Active modal overlay state.
#[derive(Debug, Clone, Default)]
pub enum Overlay {
    #[default]
    None,
    Help {
        query: TextInput,
        selected: usize,
    },
    Filter {
        panel: Panel,
        input: TextInput,
    },
    ContextMenu {
        title: String,
        items: Vec<MenuItem>,
        selected: usize,
    },
    CommandPalette {
        query: TextInput,
        selected: usize,
    },
    ProfileSwitcher {
        candidates: Vec<String>,
        input: TextInput,
        selected: usize,
    },
    Confirm {
        title: String,
        detail: String,
        command_preview: String,
        action: ConfirmAction,
    },
    TaskArgsForm {
        task_name: String,
        background: bool,
        force: bool,
        fields: Vec<ArgField>,
        extra_args: TextInput,
        focused_field: usize,
    },
    RegistryPicker {
        query: TextInput,
        selected: usize,
        step: RegistryStep,
    },
    EnvSetForm {
        key: TextInput,
        value: TextInput,
        target_files: Vec<String>,
        target_idx: usize,
        focused_field: usize, // 0=key, 1=value, 2=target_file
        is_edit: bool,
    },
    GenerateMenu {
        items: Vec<GenerateItem>,
        selected: usize,
        template_name_input: Option<(String, TextInput)>,
    },
}

#[derive(Debug, Clone)]
pub struct GenerateItem {
    pub label: String,
    pub description: String,
    pub command_preview: String,
    pub kind: GenerateKind,
}

#[derive(Debug, Clone)]
pub enum GenerateKind {
    MiseGenerate { args: Vec<String> },
    WorkspaceTemplate { template_name: String },
}

/// Foreground terminal handoff request executed by `main.rs` via `TerminalGuard`.
#[derive(Debug, Clone)]
pub enum ForegroundRequest {
    RunTask {
        name: String,
        args: Vec<String>,
        force: bool,
    },
    WatchTask {
        name: String,
    },
    EditFile {
        path: PathBuf,
    },
    RunMiseCommand {
        args: Vec<String>,
        wait_for_enter: bool,
        reload_after: bool,
        label: String,
    },
}

/// Persistent per-panel list scroll state (fixes list viewport jumping on every frame!).
#[derive(Debug, Default)]
pub struct PanelListStates {
    pub tasks: ListState,
    pub tools: ListState,
    pub env: ListState,
    pub config: ListState,
    pub jobs: ListState,
}

impl PanelListStates {
    pub fn get_mut(&mut self, panel: Panel) -> &mut ListState {
        match panel {
            Panel::Tasks => &mut self.tasks,
            Panel::Tools => &mut self.tools,
            Panel::Env => &mut self.env,
            Panel::Config => &mut self.config,
            Panel::Jobs => &mut self.jobs,
        }
    }

    pub fn get(&self, panel: Panel) -> &ListState {
        match panel {
            Panel::Tasks => &self.tasks,
            Panel::Tools => &self.tools,
            Panel::Env => &self.env,
            Panel::Config => &self.config,
            Panel::Jobs => &self.jobs,
        }
    }
}

/// Per-panel independent filter strings.
#[derive(Debug, Default, Clone)]
pub struct PanelFilters {
    pub tasks: String,
    pub tools: String,
    pub env: String,
    pub config: String,
    pub jobs: String,
}

impl PanelFilters {
    pub fn get(&self, panel: Panel) -> &str {
        match panel {
            Panel::Tasks => &self.tasks,
            Panel::Tools => &self.tools,
            Panel::Env => &self.env,
            Panel::Config => &self.config,
            Panel::Jobs => &self.jobs,
        }
    }

    pub fn get_mut(&mut self, panel: Panel) -> &mut String {
        match panel {
            Panel::Tasks => &mut self.tasks,
            Panel::Tools => &mut self.tools,
            Panel::Env => &mut self.env,
            Panel::Config => &mut self.config,
            Panel::Jobs => &mut self.jobs,
        }
    }
}

/// Central read/write state of lazymise 2.0.
/// `ui::render` reads fields and mutates ONLY `list_states` and `detail_scroll.viewport_height/content_len`.
pub struct AppState {
    pub workdir: PathBuf,
    pub profile: ProfileSpec,
    pub generation: u64,
    pub should_quit: bool,
    pub dirty: bool,
    pub tick_count: u64,

    // Layout & navigation
    pub active_panel: Panel,
    pub focus: FocusArea,
    pub screen_mode: ScreenMode,
    pub log_mode: LogPanelMode,
    pub task_subview: TaskSubView,
    pub tool_subview: ToolSubView,
    pub env_subview: EnvSubView,
    pub config_subview: ConfigSubView,
    pub overlay: Overlay,
    pub filters: PanelFilters,
    pub list_states: PanelListStates,
    pub detail_scroll: ScrollState,
    pub auto_follow: bool,

    // Async loading status & feedback
    pub loading_domains: BTreeSet<Domain>,
    pub toasts: Vec<Toast>,
    pub pending_fg: Option<ForegroundRequest>,
    pub ctrl_c_armed_at: Option<Instant>,
    pub last_click: Option<(Instant, u16, u16)>,
    pub requested_domains: BTreeSet<Domain>,

    // Panel 1: Tasks
    pub tasks: Vec<MiseTask>,
    pub show_hidden_tasks: bool,
    pub monorepo_all_tasks: bool,
    pub collapsed_groups: BTreeSet<String>,
    pub task_rows: Vec<TaskRow>,
    pub script_cache: ScriptCache,
    pub task_info_cache: HashMap<String, MiseTask>,
    pub task_deps_cache: HashMap<String, Vec<String>>,
    pub task_history_cache: HashMap<String, Vec<TaskHistoryEntry>>,
    pub last_task: Option<String>,

    // Panel 2: Tools
    pub tools_vm: ToolViewModel,
    pub tool_scope: ToolScope,
    pub registry: Vec<RegistryEntry>,

    // Panel 3: Env
    pub env_vm: EnvViewModel,
    pub env_filter: EnvFilter,
    pub revealed_env_keys: HashMap<String, Instant>,

    // Panel 4: Config & Health (+ legacy workspace templates)
    pub config_vm: ConfigViewModel,
    pub templates: Vec<WorkspaceTemplate>,
    pub selected_daemon_idx: usize,

    // Panel 5: Jobs & Command Log
    pub jobs: BTreeMap<JobId, JobRecord>,
    pub latest_task_job: HashMap<String, JobId>,
    pub command_log: CommandLog,
    pub show_internal_commands: bool,
    pub selected_job_id: Option<JobId>,
}

impl AppState {
    pub fn new(workdir: PathBuf, profile: ProfileSpec, initial_panel: Panel) -> Self {
        let mut collapsed_groups = BTreeSet::new();
        collapsed_groups.insert("[global]".to_string());
        Self {
            workdir,
            profile: profile.filter(|s| !s.trim().is_empty()),
            generation: 0,
            should_quit: false,
            dirty: true,
            tick_count: 0,
            active_panel: initial_panel,
            focus: if initial_panel == Panel::Jobs {
                FocusArea::Log
            } else {
                FocusArea::Sidebar
            },
            screen_mode: ScreenMode::Normal,
            log_mode: LogPanelMode::Compact,
            task_subview: TaskSubView::Spec,
            tool_subview: ToolSubView::Details,
            env_subview: EnvSubView::Detail,
            config_subview: ConfigSubView::File,
            overlay: Overlay::None,
            filters: PanelFilters::default(),
            list_states: PanelListStates::default(),
            detail_scroll: ScrollState::default(),
            auto_follow: true,
            loading_domains: BTreeSet::new(),
            toasts: Vec::new(),
            pending_fg: None,
            ctrl_c_armed_at: None,
            last_click: None,
            requested_domains: BTreeSet::new(),
            tasks: Vec::new(),
            show_hidden_tasks: false,
            monorepo_all_tasks: false,
            collapsed_groups,
            task_rows: Vec::new(),
            script_cache: ScriptCache::new(),
            task_info_cache: HashMap::new(),
            task_deps_cache: HashMap::new(),
            task_history_cache: HashMap::new(),
            last_task: None,
            tools_vm: ToolViewModel::default(),
            tool_scope: ToolScope::All,
            registry: Vec::new(),
            env_vm: EnvViewModel::default(),
            env_filter: EnvFilter::All,
            revealed_env_keys: HashMap::new(),
            config_vm: ConfigViewModel::default(),
            templates: Vec::new(),
            selected_daemon_idx: 0,
            jobs: BTreeMap::new(),
            latest_task_job: HashMap::new(),
            command_log: CommandLog::default(),
            show_internal_commands: false,
            selected_job_id: None,
        }
    }

    pub fn push_toast(&mut self, level: ToastLevel, msg: impl Into<String>) {
        let ttl = match level {
            ToastLevel::Error | ToastLevel::Warning => Duration::from_secs(5),
            ToastLevel::Info | ToastLevel::Success => Duration::from_secs(3),
        };
        self.toasts.push(Toast::new(level, msg, ttl));
        if self.toasts.len() > 5 {
            self.toasts.remove(0);
        }
        self.dirty = true;
    }

    pub fn prune_expired_toasts(&mut self) -> bool {
        let now = Instant::now();
        let before = self.toasts.len();
        self.toasts.retain(|t| t.expires_at > now);
        let expired_secrets_before = self.revealed_env_keys.len();
        self.revealed_env_keys.retain(|_, exp| *exp > now);
        let expired_ctrl_c = self
            .ctrl_c_armed_at
            .is_some_and(|at| now.saturating_duration_since(at) > FORCE_QUIT_WINDOW);
        if expired_ctrl_c {
            self.ctrl_c_armed_at = None;
        }
        self.toasts.len() != before
            || self.revealed_env_keys.len() != expired_secrets_before
            || expired_ctrl_c
    }

    pub fn active_toast(&self) -> Option<&Toast> {
        self.toasts.last()
    }

    pub fn running_jobs_count(&self) -> usize {
        self.jobs.values().filter(|j| j.status.is_running()).count()
    }

    pub fn has_active_animations(&self) -> bool {
        !self.loading_domains.is_empty()
            || self.running_jobs_count() > 0
            || !self.toasts.is_empty()
            || !self.revealed_env_keys.is_empty()
            || self.ctrl_c_armed_at.is_some()
    }

    pub fn format_mise_command(&self, args: &[String]) -> String {
        let mut parts = vec!["mise".to_string()];
        if let Some(env_name) = self.profile.as_deref().filter(|s| !s.trim().is_empty()) {
            parts.push("-E".to_string());
            parts.push(env_name.to_string());
        }
        parts.extend(args.iter().cloned());
        parts.join(" ")
    }

    /// Format a `mise` CLI command for display / Command Log while redacting sensitive
    /// `mise set KEY=VAL` values so secrets never leak into modals or logs.
    pub fn format_mise_command_safe(&self, args: &[String]) -> String {
        let is_set = args.first().map(String::as_str) == Some("set");
        if !is_set {
            return self.format_mise_command(args);
        }
        let safe_args: Vec<String> = args
            .iter()
            .map(|arg| {
                if let Some((k, _)) = arg.split_once('=')
                    && (crate::model::env::EnvRow::is_sensitive_key(k)
                        || self.env_vm.redacted_keys.contains(k))
                {
                    return format!("{k}=••••••••");
                }
                arg.clone()
            })
            .collect();
        self.format_mise_command(&safe_args)
    }

    pub fn rebuild_task_rows(&mut self) {
        let mut tree = TaskTree {
            collapsed: self.collapsed_groups.iter().cloned().collect(),
            show_hidden: self.show_hidden_tasks,
            monorepo_all: self.monorepo_all_tasks,
            rows: Vec::new(),
        };
        tree.rebuild(&self.tasks, &self.filters.tasks);
        self.task_rows = tree.rows;
        self.clamp_panel_selection(Panel::Tasks);
        self.preload_selected_content();
    }

    pub fn rebuild_tools(&mut self) {
        self.tools_vm
            .rebuild(&self.workdir, self.tool_scope, &self.filters.tools);
        self.clamp_panel_selection(Panel::Tools);
    }

    pub fn rebuild_env(&mut self) {
        self.env_vm
            .rebuild(&self.workdir, self.env_filter, &self.filters.env);
        self.clamp_panel_selection(Panel::Env);
    }

    pub fn rebuild_config(&mut self) {
        self.config_vm.rebuild(&self.filters.config);
        self.clamp_panel_selection(Panel::Config);
        self.preload_selected_content();
    }

    pub fn rebuild_panel(&mut self, panel: Panel) {
        match panel {
            Panel::Tasks => self.rebuild_task_rows(),
            Panel::Tools => self.rebuild_tools(),
            Panel::Env => self.rebuild_env(),
            Panel::Config => self.rebuild_config(),
            Panel::Jobs => self.clamp_panel_selection(Panel::Jobs),
        }
    }

    /// Preload the currently selected task script or config file into memory caches
    /// so `ui::render` never performs disk I/O.
    pub fn preload_selected_content(&mut self) {
        if let Some(task) = self.selected_task().cloned() {
            let _ = self.script_cache.preload_task(&task);
        }
        if let Some(cfg) = self.selected_config().cloned() {
            self.config_vm.preload_file(PathBuf::from(cfg.entry.path));
        }
    }

    pub fn filtered_command_log(&self) -> Vec<&CommandLogEntry> {
        let raw = self.command_log.filtered(self.show_internal_commands);
        let q = self.filters.jobs.trim();
        if q.is_empty() {
            return raw;
        }
        raw.into_iter()
            .filter(|entry| {
                fuzzy_match(q, &entry.command).is_some()
                    || entry
                        .summary
                        .as_deref()
                        .is_some_and(|s| fuzzy_match(q, s).is_some())
            })
            .collect()
    }

    pub fn current_list_len(&self, panel: Panel) -> usize {
        match panel {
            Panel::Tasks => self.task_rows.len(),
            Panel::Tools => self.tools_vm.rows.len(),
            Panel::Env => self.env_vm.rows.len(),
            Panel::Config => self.config_vm.rows.len(),
            Panel::Jobs => self.filtered_command_log().len(),
        }
    }

    pub fn clamp_panel_selection(&mut self, panel: Panel) {
        let len = self.current_list_len(panel);
        let ls = self.list_states.get_mut(panel);
        if len == 0 {
            ls.select(None);
        } else {
            let next = ls.selected().unwrap_or(0).min(len - 1);
            ls.select(Some(next));
        }
    }

    pub fn clamp_all_selections(&mut self) {
        for panel in Panel::ALL {
            self.clamp_panel_selection(panel);
        }
    }

    pub fn selected_task(&self) -> Option<&MiseTask> {
        let idx = self.list_states.tasks.selected()?;
        self.task_rows.get(idx).and_then(TaskRow::as_task)
    }

    pub fn selected_task_with_info(&self) -> Option<&MiseTask> {
        let task = self.selected_task()?;
        self.task_info_cache.get(&task.name).or(Some(task))
    }

    pub fn selected_task_job(&self) -> Option<&JobRecord> {
        let task = self.selected_task()?;
        let job_id = self.latest_task_job.get(&task.name)?;
        self.jobs.get(job_id)
    }

    pub fn selected_tool(&self) -> Option<&crate::model::tools::ToolRow> {
        let idx = self.list_states.tools.selected()?;
        self.tools_vm.rows.get(idx)
    }

    pub fn selected_env(&self) -> Option<&crate::model::env::EnvRow> {
        let idx = self.list_states.env.selected()?;
        self.env_vm.rows.get(idx)
    }

    pub fn selected_config(&self) -> Option<&crate::model::config::ConfigFileRow> {
        let idx = self.list_states.config.selected()?;
        self.config_vm.rows.get(idx)
    }

    pub fn selected_command_log_entry(&self) -> Option<&CommandLogEntry> {
        let entries = self.filtered_command_log();
        let idx = self.list_states.jobs.selected()?;
        entries.get(idx).copied()
    }

    pub fn selected_job_record(&self) -> Option<&JobRecord> {
        if let Some(id) = self.selected_job_id
            && let Some(rec) = self.jobs.get(&id)
        {
            return Some(rec);
        }
        if let Some(entry) = self.selected_command_log_entry()
            && let Some(job_id) = entry.job_id
            && let Some(rec) = self.jobs.get(&job_id)
        {
            return Some(rec);
        }
        self.selected_task_job()
    }

    pub fn is_env_revealed(&self, key: &str) -> bool {
        self.revealed_env_keys
            .get(key)
            .is_some_and(|exp| *exp > Instant::now())
    }

    pub fn env_reveal_remaining_secs(&self, key: &str) -> Option<u64> {
        let exp = self.revealed_env_keys.get(key)?;
        let now = Instant::now();
        (*exp > now).then(|| exp.saturating_duration_since(now).as_secs().max(1))
    }

    pub fn filtered_registry(&self, query: &str) -> Vec<RegistryMatch> {
        let src = if !self.tools_vm.registry.is_empty() {
            &self.tools_vm.registry
        } else {
            &self.registry
        };
        filter_registry_entries(src, query)
    }

    pub fn discover_profile_candidates(&self) -> Vec<String> {
        let mut out = vec!["default".to_string()];
        let discovered = MiseClient::new(&self.workdir).discover_profiles();
        for p in discovered {
            if !out.contains(&p) {
                out.push(p);
            }
        }
        if let Some(active) = self.profile.as_deref()
            && !out.iter().any(|p| p == active)
        {
            out.push(active.to_string());
        }
        out
    }

    pub fn build_generate_items(&self) -> Vec<GenerateItem> {
        build_scaffold_catalog(&self.templates)
            .into_iter()
            .map(|entry| GenerateItem {
                label: entry.label,
                description: entry.description,
                command_preview: entry.command_preview,
                kind: match entry.kind {
                    ScaffoldKind::MiseGenerate { args, .. } => GenerateKind::MiseGenerate { args },
                    ScaffoldKind::WorkspaceTemplate { template_name, .. } => {
                        GenerateKind::WorkspaceTemplate { template_name }
                    }
                },
            })
            .collect()
    }

    pub fn record_foreground_result(
        &mut self,
        req: &ForegroundRequest,
        code: i32,
        duration_ms: u128,
        job_id: JobId,
    ) {
        let status = if code == 0 {
            JobStatus::Success { duration_ms }
        } else {
            JobStatus::Failed { code, duration_ms }
        };
        let (label, task_name, command_display) = match req {
            ForegroundRequest::RunTask { name, args, force } => {
                let mut cmd = vec!["run".to_string()];
                if *force {
                    cmd.push("--force".to_string());
                }
                cmd.push(name.clone());
                if !args.is_empty() {
                    cmd.push("--".to_string());
                    cmd.extend(args.iter().cloned());
                }
                (
                    format!("task:{name}"),
                    Some(name.clone()),
                    self.format_mise_command(&cmd),
                )
            }
            ForegroundRequest::WatchTask { name } => (
                format!("watch:{name}"),
                Some(name.clone()),
                self.format_mise_command(&["watch".to_string(), "-t".to_string(), name.clone()]),
            ),
            ForegroundRequest::EditFile { path } => (
                format!("edit:{}", path.display()),
                None,
                format!("$EDITOR {}", path.display()),
            ),
            ForegroundRequest::RunMiseCommand { args, label, .. } => {
                (label.clone(), None, self.format_mise_command_safe(args))
            }
        };

        let summary = format!("exit {code} ({:.2}s)", duration_ms as f64 / 1000.0);
        let record = JobRecord {
            id: job_id,
            label: label.clone(),
            task_name: task_name.clone(),
            command_display: command_display.clone(),
            pid: None,
            started_at: Instant::now(),
            status: status.clone(),
            lines: vec![JobOutputLine {
                text: format!("(ran in foreground TTY: `{command_display}`, {summary})"),
                is_stderr: code != 0,
            }],
        };

        self.jobs.insert(job_id, record);
        if let Some(t) = task_name {
            self.latest_task_job.insert(t, job_id);
        }
        self.command_log.record(
            CommandSource::User,
            command_display,
            Some(job_id),
            status,
            Some(summary.clone()),
        );
        self.clamp_panel_selection(Panel::Jobs);

        let level = if code == 0 {
            ToastLevel::Success
        } else {
            ToastLevel::Error
        };
        self.push_toast(level, format!("{label}: {summary}"));
    }

    pub fn spinner_char(&self) -> char {
        const FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        FRAMES[(self.tick_count as usize) % FRAMES.len()]
    }
}
