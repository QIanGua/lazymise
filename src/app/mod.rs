pub mod keymap;
pub mod mouse;
pub mod overlay;
pub mod reducer;
pub mod state;

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use crate::action::{Panel, ProfileSpec};
use crate::app::keymap::{PendingChord, resolve};
use crate::app::state::AppState;
use crate::events::AppEvent;
use crate::mise::loader::MiseLoader;
use crate::runner::BackgroundRunner;
use crate::ui::layout::LayoutMap;

pub use state::{AppState as State, ForegroundRequest, Overlay};

/// Top-level application coordinator owning `AppState`, `MiseLoader`, `BackgroundRunner`, and `LayoutMap`.
pub struct App {
    pub state: AppState,
    pub loader: MiseLoader,
    pub runner: BackgroundRunner,
    pub layout_map: LayoutMap,
    chord: PendingChord,
}

impl App {
    /// Construct the application and immediately launch non-blocking Tier-1 hot reload
    /// (+ background Tier-2 warmup) so first frame renders in <10ms.
    pub fn new(
        workdir: PathBuf,
        profile: ProfileSpec,
        initial_panel: Panel,
        tx: Sender<AppEvent>,
    ) -> Self {
        let loader = MiseLoader::new(workdir.clone(), tx.clone());
        let runner = BackgroundRunner::new(tx);
        let mut state = AppState::new(workdir, profile, initial_panel);
        reducer::trigger_hot_reload(&mut state, &loader, true);
        Self {
            state,
            loader,
            runner,
            layout_map: LayoutMap::default(),
            chord: PendingChord::None,
        }
    }

    pub fn handle_event(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Key(key) => {
                if overlay::handle_overlay_key(&mut self.state, key, &self.loader, &self.runner) {
                    return;
                }
                if let Some(action) = resolve(key, &self.state, &mut self.chord) {
                    reducer::apply_action(&mut self.state, action, &self.loader, &self.runner);
                }
            }
            AppEvent::Mouse(m) => {
                mouse::handle_mouse(
                    &mut self.state,
                    m,
                    &self.layout_map,
                    &self.loader,
                    &self.runner,
                );
            }
            AppEvent::Resize(_, _) => {
                self.state.dirty = true;
            }
            AppEvent::Tick => {
                self.state.tick_count = self.state.tick_count.wrapping_add(1);
                if self.state.prune_expired_toasts() || self.state.has_active_animations() {
                    self.state.dirty = true;
                }
            }
            AppEvent::MiseData(ev) => {
                reducer::apply_mise_event(&mut self.state, ev);
            }
            AppEvent::Runner(ev) => {
                reducer::apply_runner_event(&mut self.state, ev, &self.loader);
            }
        }
    }

    pub fn record_foreground_result(
        &mut self,
        req: &ForegroundRequest,
        code: i32,
        duration_ms: u128,
        reload_after: bool,
    ) {
        let job_id = self.runner.allocate_job_id();
        self.state
            .record_foreground_result(req, code, duration_ms, job_id);
        if reload_after {
            reducer::trigger_hot_reload(&mut self.state, &self.loader, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use pretty_assertions::assert_eq;
    use ratatui::layout::Rect;

    use super::*;
    use crate::action::{
        Action, Domain, EnvFilter, FocusArea, LogPanelMode, ScreenMode, TaskSubView, ToastLevel,
        ToolScope,
    };
    use crate::app::state::RegistryStep;
    use crate::events::{JobId, JobRecord, JobStatus, MiseDataEvent, MisePayload};
    use crate::mise::schema::{EnvExtendedEntry, MiseTask, MiseToolEntry, RegistryEntry};

    fn test_app() -> (App, mpsc::Receiver<AppEvent>) {
        let (tx, rx) = mpsc::channel();
        let loader = MiseLoader::new(PathBuf::from("/tmp/lazymise-unit-test"), tx.clone());
        let runner = BackgroundRunner::new(tx);
        let mut state = AppState::new(
            PathBuf::from("/tmp/lazymise-unit-test"),
            None,
            Panel::Tasks,
        );
        state.generation = loader.next_generation();
        let app = App {
            state,
            loader,
            runner,
            layout_map: LayoutMap::default(),
            chord: PendingChord::None,
        };
        (app, rx)
    }

    fn key(code: KeyCode) -> AppEvent {
        AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ctrl(c: char) -> AppEvent {
        AppEvent::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    #[test]
    fn panel_switching_zoom_and_per_panel_filter_isolation() {
        let (mut app, _rx) = test_app();

        // Populate Tasks and Tools via MiseDataEvent
        app.handle_event(AppEvent::MiseData(MiseDataEvent {
            generation: 1,
            domain: Domain::Tasks,
            payload: MisePayload::Tasks(vec![
                MiseTask {
                    name: "build".into(),
                    ..Default::default()
                },
                MiseTask {
                    name: "test".into(),
                    ..Default::default()
                },
            ]),
            log_entry: None,
        }));

        let mut tools_map = BTreeMap::new();
        tools_map.insert(
            "rust".into(),
            vec![MiseToolEntry {
                version: "1.90.0".into(),
                installed: true,
                active: true,
                ..Default::default()
            }],
        );
        tools_map.insert(
            "node".into(),
            vec![MiseToolEntry {
                version: "22.1.0".into(),
                installed: true,
                active: true,
                ..Default::default()
            }],
        );
        app.handle_event(AppEvent::MiseData(MiseDataEvent {
            generation: 1,
            domain: Domain::Tools,
            payload: MisePayload::Tools(tools_map),
            log_entry: None,
        }));

        // Apply filter on Tasks panel (`/` -> "bu" -> Enter)
        app.handle_event(key(KeyCode::Char('/')));
        app.handle_event(key(KeyCode::Char('b')));
        app.handle_event(key(KeyCode::Char('u')));
        app.handle_event(key(KeyCode::Enter));
        assert_eq!(app.state.filters.tasks, "bu");
        assert_eq!(app.state.filters.tools, "");

        // Switch to Tools (`2`) — Tools filter must stay isolated & unfiltered
        app.handle_event(key(KeyCode::Char('2')));
        assert_eq!(app.state.active_panel, Panel::Tools);
        assert_eq!(app.state.tools_vm.rows.len(), 2);

        // Cycle zoom modes: Normal -> Half -> Full -> Half -> Normal
        assert_eq!(app.state.screen_mode, ScreenMode::Normal);
        app.handle_event(key(KeyCode::Char('+')));
        assert_eq!(app.state.screen_mode, ScreenMode::Half);
        app.handle_event(key(KeyCode::Char('+')));
        assert_eq!(app.state.screen_mode, ScreenMode::Full);
        app.handle_event(key(KeyCode::Char('_')));
        assert_eq!(app.state.screen_mode, ScreenMode::Half);

        // Toggle Command Log drawer (`@`): Compact -> Expanded -> Hidden
        assert_eq!(app.state.log_mode, LogPanelMode::Compact);
        app.handle_event(key(KeyCode::Char('@')));
        assert_eq!(app.state.log_mode, LogPanelMode::Expanded);
        app.handle_event(key(KeyCode::Char('@')));
        assert_eq!(app.state.log_mode, LogPanelMode::Hidden);
    }

    #[test]
    fn drops_stale_generation_mise_events_and_enforces_scroll_bounds() {
        let (mut app, _rx) = test_app();
        app.state.generation = 5;

        // Stale event (gen=4) must be dropped
        app.handle_event(AppEvent::MiseData(MiseDataEvent {
            generation: 4,
            domain: Domain::Tasks,
            payload: MisePayload::Tasks(vec![MiseTask {
                name: "stale".into(),
                ..Default::default()
            }]),
            log_entry: None,
        }));
        assert!(app.state.tasks.is_empty());

        // Bounded detail scroll never overflows past max_offset
        app.state.focus = FocusArea::Main;
        app.state.detail_scroll.content_len = 30;
        app.state.detail_scroll.viewport_height = 10;
        app.handle_event(key(KeyCode::Char('G')));
        assert_eq!(app.state.detail_scroll.offset, 20);
        app.handle_event(key(KeyCode::Char('j')));
        assert_eq!(app.state.detail_scroll.offset, 20);
        app.handle_event(key(KeyCode::Home));
        assert_eq!(app.state.detail_scroll.offset, 0);
    }

    #[test]
    fn env_secret_reveal_auto_remasks_after_ttl() {
        let (mut app, _rx) = test_app();
        let mut ext = BTreeMap::new();
        ext.insert(
            "API_TOKEN".into(),
            EnvExtendedEntry {
                value: "super-secret".into(),
                source: Some("/tmp/lazymise-unit-test/mise.toml".into()),
                tool: None,
            },
        );
        app.handle_event(AppEvent::MiseData(MiseDataEvent {
            generation: 1,
            domain: Domain::Env,
            payload: MisePayload::Env {
                extended: ext,
                redacted_keys: Default::default(),
                paths: vec![],
                vars: Default::default(),
            },
            log_entry: None,
        }));

        app.handle_event(key(KeyCode::Char('3')));
        assert!(!app.state.is_env_revealed("API_TOKEN"));
        app.handle_event(key(KeyCode::Char('S')));
        assert!(app.state.is_env_revealed("API_TOKEN"));

        // Simulate 30s expiration
        app.state
            .revealed_env_keys
            .insert("API_TOKEN".into(), Instant::now() - Duration::from_secs(1));
        app.handle_event(AppEvent::Tick);
        assert!(!app.state.is_env_revealed("API_TOKEN"));

        // Cycle env filter
        app.handle_event(key(KeyCode::Char('f')));
        assert_eq!(app.state.env_filter, EnvFilter::Project);
    }

    #[test]
    fn quit_guards_running_jobs_and_double_ctrl_c_forces_quit() {
        let (mut app, _rx) = test_app();
        app.state.jobs.insert(
            JobId(1),
            JobRecord {
                id: JobId(1),
                label: "task:dev".into(),
                task_name: Some("dev".into()),
                command_display: "mise run dev".into(),
                pid: None,
                started_at: Instant::now(),
                status: JobStatus::Running,
                lines: vec![],
            },
        );

        // `q` opens Confirm instead of quitting immediately
        app.handle_event(key(KeyCode::Char('q')));
        assert!(!app.state.should_quit);
        assert!(matches!(app.state.overlay, Overlay::Confirm { .. }));

        // Cancel confirm
        app.handle_event(key(KeyCode::Esc));
        assert!(matches!(app.state.overlay, Overlay::None));

        // First Ctrl-C arms force quit with warning toast
        app.handle_event(ctrl('c'));
        assert!(!app.state.should_quit);
        assert!(app.state.ctrl_c_armed_at.is_some());
        assert_eq!(
            app.state.active_toast().map(|t| t.level),
            Some(ToastLevel::Warning)
        );

        // Second Ctrl-C exits
        app.handle_event(ctrl('c'));
        assert!(app.state.should_quit);
    }

    #[test]
    fn overlays_help_registry_and_profile_switcher_flow() {
        let (mut app, _rx) = test_app();
        app.state.registry = vec![RegistryEntry {
            short: "ripgrep".into(),
            backends: vec!["aqua:BurntSushi/ripgrep".into()],
            bins: vec!["rg".into()],
            description: Some("Fast search".into()),
            aliases: vec!["rg".into()],
        }];

        // Open Help (`?`), filter for "zoom", press Enter -> triggers ZoomIn
        app.handle_event(key(KeyCode::Char('?')));
        for ch in "zoom".chars() {
            app.handle_event(key(KeyCode::Char(ch)));
        }
        app.handle_event(key(KeyCode::Enter));
        assert!(matches!(app.state.overlay, Overlay::None));
        assert_eq!(app.state.screen_mode, ScreenMode::Half);

        // Open Registry Picker on Tools (`2` -> `a`), search "rg", press Enter -> PickVersion
        app.handle_event(key(KeyCode::Char('2')));
        app.handle_event(key(KeyCode::Char('a')));
        app.handle_event(key(KeyCode::Char('r')));
        app.handle_event(key(KeyCode::Char('g')));
        app.handle_event(key(KeyCode::Enter));
        assert!(matches!(
            app.state.overlay,
            Overlay::RegistryPicker {
                step: RegistryStep::PickVersion { ref tool, .. },
                ..
            } if tool == "ripgrep"
        ));

        // Pick "latest" -> PickScope -> Confirm `mise use ripgrep@latest`
        app.handle_event(key(KeyCode::Enter));
        assert!(matches!(
            app.state.overlay,
            Overlay::RegistryPicker {
                step: RegistryStep::PickScope { .. },
                ..
            }
        ));
        app.handle_event(key(KeyCode::Enter));
        assert!(matches!(
            app.state.overlay,
            Overlay::Confirm { ref command_preview, .. } if command_preview.contains("mise use ripgrep@latest")
        ));
        app.handle_event(key(KeyCode::Esc));

        // Profile Switcher (`E`) -> type "prod" -> Enter
        app.handle_event(key(KeyCode::Char('E')));
        for ch in "prod".chars() {
            app.handle_event(key(KeyCode::Char(ch)));
        }
        app.handle_event(key(KeyCode::Enter));
        assert_eq!(app.state.profile.as_deref(), Some("prod"));
    }

    #[test]
    fn mouse_click_subtab_and_hover_wheel_scroll() {
        let (mut app, _rx) = test_app();
        app.state.detail_scroll.content_len = 50;
        app.state.detail_scroll.viewport_height = 10;
        app.layout_map.main_area = Rect::new(40, 2, 60, 20);
        app.layout_map.main_subtabs = vec![
            (0, Rect::new(41, 2, 8, 1)),
            (1, Rect::new(50, 2, 8, 1)),
        ];

        // Hover scroll over Main scrolls detail_scroll even while Sidebar is focused!
        assert_eq!(app.state.focus, FocusArea::Sidebar);
        app.handle_event(AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 55,
            row: 10,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(app.state.detail_scroll.offset, 3);
        assert_eq!(app.state.focus, FocusArea::Sidebar);

        // Clicking sub-tab 1 switches TaskSubView to Output and focuses Main
        app.handle_event(AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 52,
            row: 2,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(app.state.task_subview, TaskSubView::Output);
        assert_eq!(app.state.focus, FocusArea::Main);

        // Verify ToolScope cycling via direct action
        reducer::apply_action(&mut app.state, Action::ToolCycleScope, &app.loader, &app.runner);
        assert_eq!(app.state.tool_scope, ToolScope::Active);
    }

    #[test]
    fn unicode_text_input_env_set_form_and_command_palette_work() {
        let (mut app, _rx) = test_app();

        // Unicode TextInput editing (insert, move left, backspace, delete word)
        let mut ti = crate::app::state::TextInput::new("你好 world");
        ti.move_left();
        ti.insert_char('!');
        assert_eq!(ti.value, "你好 worl!d");
        ti.move_end();
        ti.delete_word_back();
        assert_eq!(ti.value, "你好 ");

        // Command Palette (`:`) -> search "doctor" -> Enter switches to Config Doctor subview
        app.handle_event(key(KeyCode::Char(':')));
        for ch in "doctor".chars() {
            app.handle_event(key(KeyCode::Char(ch)));
        }
        app.handle_event(key(KeyCode::Enter));
        assert!(matches!(app.state.overlay, Overlay::None));
        assert_eq!(app.state.config_subview, crate::action::ConfigSubView::Doctor);

        // EnvSetForm (`3` -> `a` -> type key, Tab, type val, Enter -> Confirm `mise set FOO=bar`)
        app.handle_event(key(KeyCode::Char('3')));
        app.handle_event(key(KeyCode::Char('a')));
        for ch in "FOO".chars() {
            app.handle_event(key(KeyCode::Char(ch)));
        }
        app.handle_event(key(KeyCode::Tab));
        for ch in "bar".chars() {
            app.handle_event(key(KeyCode::Char(ch)));
        }
        app.handle_event(key(KeyCode::Enter));
        assert!(matches!(
            app.state.overlay,
            Overlay::Confirm { ref command_preview, .. } if command_preview == "mise set FOO=bar"
        ));
    }

    #[test]
    fn runner_events_update_jobs_command_log_and_auto_follow() {
        let (mut app, _rx) = test_app();
        app.state.detail_scroll.viewport_height = 5;
        app.state.selected_job_id = Some(JobId(9));
        app.state.jobs.insert(
            JobId(9),
            JobRecord {
                id: JobId(9),
                label: "task:test".into(),
                task_name: Some("test".into()),
                command_display: "mise run test".into(),
                pid: None,
                started_at: Instant::now(),
                status: JobStatus::Running,
                lines: vec![],
            },
        );
        app.state.command_log.record(
            crate::events::CommandSource::User,
            "mise run test",
            Some(JobId(9)),
            JobStatus::Running,
            None,
        );

        for i in 0..10 {
            app.handle_event(AppEvent::Runner(crate::events::RunnerEvent::Output {
                job_id: JobId(9),
                line: crate::events::JobOutputLine {
                    text: format!("line {i}"),
                    is_stderr: false,
                },
                overwrite_last: false,
            }));
        }
        assert!(app.state.detail_scroll.offset > 0);

        app.handle_event(AppEvent::Runner(crate::events::RunnerEvent::Finished {
            job_id: JobId(9),
            exit_code: 0,
            duration_ms: 250,
            killed: false,
        }));
        assert_eq!(
            app.state.jobs[&JobId(9)].status,
            JobStatus::Success { duration_ms: 250 }
        );
        assert_eq!(
            app.state
                .command_log
                .find_by_job(JobId(9))
                .map(|e| &e.status),
            Some(&JobStatus::Success { duration_ms: 250 })
        );
    }

    #[test]
    fn verifier_fixes_mouse_offset_palette_task_jump_secret_masking_and_daemons() {
        let (mut app, _rx) = test_app();

        // 1. Mouse click on Tasks sidebar while active_panel == Panel::Jobs uses tasks offset, not jobs offset
        app.handle_event(AppEvent::MiseData(MiseDataEvent {
            generation: 1,
            domain: Domain::Tasks,
            payload: MisePayload::Tasks(vec![
                MiseTask {
                    name: "alpha".into(),
                    ..Default::default()
                },
                MiseTask {
                    name: "exp:deploy".into(),
                    description: "Deploy experiment".into(),
                    ..Default::default()
                },
            ]),
            log_entry: None,
        }));
        app.state.active_panel = Panel::Jobs;
        *app.state.list_states.jobs.offset_mut() = 7;
        *app.state.list_states.tasks.offset_mut() = 0;
        app.layout_map.sidebar_panels = vec![(
            Panel::Tasks,
            Rect::new(0, 1, 35, 10),
            Rect::new(1, 2, 33, 8),
        )];
        // Clicking row y=3 (inner offset 1) should select task_rows[1] ("alpha"), NOT clamped to end via jobs offset 7
        app.handle_event(AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 3,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(app.state.active_panel, Panel::Tasks);
        assert_eq!(app.state.list_states.tasks.selected(), Some(1));

        // 2. Command Palette jumps to a collapsed task from another panel
        app.state.collapsed_groups.insert("exp".into());
        app.state.rebuild_task_rows();
        app.handle_event(key(KeyCode::Char('2'))); // switch to Tools
        app.handle_event(key(KeyCode::Char(':')));
        for ch in "deploy".chars() {
            app.handle_event(key(KeyCode::Char(ch)));
        }
        app.handle_event(key(KeyCode::Enter));
        assert_eq!(app.state.active_panel, Panel::Tasks);
        assert_eq!(
            app.state.selected_task().map(|t| t.name.as_str()),
            Some("exp:deploy")
        );

        // 3. Env editing a masked secret does NOT prefill plaintext, and Confirm + safe command redact it
        let mut ext = BTreeMap::new();
        ext.insert(
            "API_TOKEN".into(),
            EnvExtendedEntry {
                value: "super-secret-value".into(),
                source: Some("/tmp/lazymise-unit-test/mise.toml".into()),
                tool: None,
            },
        );
        app.handle_event(AppEvent::MiseData(MiseDataEvent {
            generation: 1,
            domain: Domain::Env,
            payload: MisePayload::Env {
                extended: ext,
                redacted_keys: Default::default(),
                paths: vec![],
                vars: Default::default(),
            },
            log_entry: None,
        }));
        app.handle_event(key(KeyCode::Char('3')));
        app.handle_event(key(KeyCode::Char('e')));
        if let Overlay::EnvSetForm { ref value, .. } = app.state.overlay {
            assert_eq!(value.value, "");
        } else {
            panic!("expected EnvSetForm overlay");
        }
        for ch in "new-secret".chars() {
            app.handle_event(key(KeyCode::Char(ch)));
        }
        app.handle_event(key(KeyCode::Enter));
        if let Overlay::Confirm {
            ref command_preview,
            ..
        } = app.state.overlay
        {
            assert!(!command_preview.contains("new-secret"));
            assert!(command_preview.contains("API_TOKEN=••••••••"));
        } else {
            panic!("expected Confirm overlay");
        }
        app.handle_event(key(KeyCode::Esc));

        // 4. TaskToggleHidden bumps generation cleanly
        let gen_before = app.state.generation;
        app.handle_event(key(KeyCode::Char('1')));
        app.handle_event(key(KeyCode::Char('H')));
        assert!(app.state.generation > gen_before);

        // 5. Config -> Services moves selected_daemon_idx and treats "active" as running
        app.state.config_vm.daemons = vec![
            crate::mise::schema::DaemonEntry {
                name: "postgres".into(),
                status: "active".into(),
                ..Default::default()
            },
            crate::mise::schema::DaemonEntry {
                name: "redis".into(),
                status: "stopped".into(),
                ..Default::default()
            },
        ];
        app.state.active_panel = Panel::Config;
        app.state.config_subview = crate::action::ConfigSubView::Services;
        app.state.focus = FocusArea::Main;
        assert_eq!(app.state.selected_daemon_idx, 0);
        app.handle_event(key(KeyCode::Char('j')));
        assert_eq!(app.state.selected_daemon_idx, 1);
        app.handle_event(key(KeyCode::Char('k')));
        assert_eq!(app.state.selected_daemon_idx, 0);
        app.handle_event(key(KeyCode::Char('s')));
        if let Overlay::Confirm {
            ref command_preview,
            ..
        } = app.state.overlay
        {
            assert!(command_preview.contains("daemons stop postgres"));
        } else {
            panic!("expected Confirm overlay for active daemon stop");
        }
    }

    #[test]
    fn enter_runs_tasks_and_confirmed_commands_in_background_without_leaving_tui() {
        let (mut app, _rx) = test_app();
        app.handle_event(AppEvent::MiseData(MiseDataEvent {
            generation: 1,
            domain: Domain::Tasks,
            payload: MisePayload::Tasks(vec![MiseTask {
                name: "build".into(),
                ..Default::default()
            }]),
            log_entry: None,
        }));

        // Select "build" (row 1 after group header) and press Enter -> background job, no pending_fg
        app.state.list_states.tasks.select(Some(1));
        app.handle_event(key(KeyCode::Enter));
        assert!(app.state.pending_fg.is_none());
        assert_eq!(app.state.running_jobs_count(), 1);
        assert_eq!(app.state.task_subview, TaskSubView::Output);
        app.runner.kill_all();

        // Explicit `T` still allows interactive foreground TTY handoff
        app.handle_event(key(KeyCode::Char('T')));
        assert!(matches!(
            app.state.pending_fg,
            Some(ForegroundRequest::RunTask { ref name, .. }) if name == "build"
        ));
        app.state.pending_fg = None;

        // Confirming `mise install` on Tools panel runs in background (no pending_fg)
        app.handle_event(key(KeyCode::Char('2')));
        app.handle_event(key(KeyCode::Char('i')));
        assert!(matches!(app.state.overlay, Overlay::Confirm { .. }));
        app.handle_event(key(KeyCode::Enter));
        assert!(app.state.pending_fg.is_none());
        assert!(app.state.jobs.values().any(|j| j.label == "mise install"));
        app.runner.kill_all();
    }

    #[test]
    fn interactive_and_raw_tasks_hand_off_to_foreground_tty() {
        let (mut app, _rx) = test_app();
        app.handle_event(AppEvent::MiseData(MiseDataEvent {
            generation: 1,
            domain: Domain::Tasks,
            payload: MisePayload::Tasks(vec![
                MiseTask {
                    name: "dev".into(),
                    interactive: true,
                    ..Default::default()
                },
                MiseTask {
                    name: "repl".into(),
                    raw: true,
                    ..Default::default()
                },
            ]),
            log_entry: None,
        }));

        // Pressing Enter or `f` on an `interactive = true` task must hand off to foreground TTY
        app.state.list_states.tasks.select(Some(1));
        app.handle_event(key(KeyCode::Enter));
        assert!(matches!(
            app.state.pending_fg,
            Some(ForegroundRequest::RunTask { ref name, force: false, .. }) if name == "dev"
        ));
        assert_eq!(app.state.running_jobs_count(), 0);
        app.state.pending_fg = None;

        app.handle_event(key(KeyCode::Char('f')));
        assert!(matches!(
            app.state.pending_fg,
            Some(ForegroundRequest::RunTask { ref name, force: true, .. }) if name == "dev"
        ));
        assert_eq!(app.state.running_jobs_count(), 0);
        app.state.pending_fg = None;

        // `raw = true` tasks also hand off to foreground TTY instead of hanging in background
        app.state.list_states.tasks.select(Some(2));
        app.handle_event(key(KeyCode::Enter));
        assert!(matches!(
            app.state.pending_fg,
            Some(ForegroundRequest::RunTask { ref name, .. }) if name == "repl"
        ));
        assert_eq!(app.state.running_jobs_count(), 0);
    }
}
