use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use pretty_assertions::assert_eq;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::json;
use unicode_width::UnicodeWidthStr;

use lazymise::action::{
    ConfigSubView, Domain, EnvFilter, EnvSubView, FocusArea, LogPanelMode, Panel, ScreenMode,
    TaskSubView, ToastLevel, ToolScope, ToolSubView,
};
use lazymise::app::keymap::PendingChord;
use lazymise::app::state::{Overlay, RegistryStep};
use lazymise::app::{App, ForegroundRequest};
use lazymise::cli::Cli;
use lazymise::events::{
    AppEvent, CommandLogEntry, CommandSource, JobId, JobOutputLine, JobRecord, JobStatus,
    MiseDataEvent, MisePayload, RunnerEvent,
};
use lazymise::mise::schema::{
    AuditReport, ConfigExplainItem, ConfigFileEntry, DaemonEntry, DepEntry, DoctorReport,
    DoctorShell, EnvExtendedEntry, MiseTask, MiseToolEntry, MiseToolSource, OutdatedEntry,
    RegistryEntry, SettingEntry, TaskCacheConfig, TaskHistoryEntry, TaskHistorySummary,
    TaskUsageSpec, ToolDetail, TrustEntry, UsageArg, UsageCmd, UsageFlag, Vulnerability,
};
use lazymise::model::template::WorkspaceTemplate;
use lazymise::ui;
use lazymise::ui::layout::LayoutMap;
use lazymise::ui::theme::{ColorCapability, Palette, detect_color_capability_from};

fn make_offline_app(workdir: PathBuf, initial_panel: Panel) -> (App, mpsc::Receiver<AppEvent>) {
    let (tx, rx) = mpsc::channel();
    let mut app = App::new(workdir, None, initial_panel, tx);
    // Advance loader generation cleanly so startup probes on nonexistent dir are dropped
    // while keeping `app.state.generation` synchronized with `app.loader`.
    let next = app.loader.next_generation();
    app.state.generation = next;
    app.state.loading_domains.clear();
    app.state.toasts.clear();
    (app, rx)
}

fn key(code: KeyCode) -> AppEvent {
    AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn ctrl(c: char) -> AppEvent {
    AppEvent::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

fn send_str(app: &mut App, s: &str) {
    for ch in s.chars() {
        app.handle_event(key(KeyCode::Char(ch)));
    }
}

fn click(col: u16, row: u16) -> AppEvent {
    AppEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: col,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn wheel_down(col: u16, row: u16) -> AppEvent {
    AppEvent::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: col,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn render_frame(app: &mut App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| ui::render(frame, &mut app.state, &mut app.layout_map))
        .unwrap();
    let buf = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..height {
        let mut x = 0u16;
        while x < width {
            let sym = buf[(x, y)].symbol();
            out.push_str(sym);
            let step = UnicodeWidthStr::width(sym).max(1) as u16;
            x = x.saturating_add(step);
        }
        out.push('\n');
    }
    out
}

fn seed_full_workspace(app: &mut App) {
    let gen_id = app.state.generation;
    let workdir = app.state.workdir.clone();

    // 1. Tasks (including Chinese/Emoji, hidden, monorepo, and usage_spec)
    let tasks = vec![
        MiseTask {
            name: "check".into(),
            description: "Run tidy, tests, and clippy".into(),
            tags: vec!["ci".into()],
            cache: Some(TaskCacheConfig::Bool(true)),
            depends: vec![json!("tidy"), json!("test")],
            run: vec!["cargo test && cargo clippy".into()],
            history: Some(TaskHistorySummary {
                task: "check".into(),
                runs: 8,
                failures: 0,
                median_ms: Some(920),
                last_ms: Some(850),
                last_status: Some("ok".into()),
                last_started_at: Some("2026-11-26T10:00:00Z".into()),
            }),
            ..Default::default()
        },
        MiseTask {
            name: "deploy".into(),
            description: "Deploy service with required target".into(),
            run: vec!["echo deploying".into()],
            usage: "arg \"<env>\" help=\"Target env\"".into(),
            usage_spec: Some(TaskUsageSpec {
                name: "deploy".into(),
                bin: "deploy".into(),
                cmd: UsageCmd {
                    args: vec![UsageArg {
                        name: "env".into(),
                        usage: "<env>".into(),
                        help: Some("Target environment".into()),
                        required: true,
                        var: false,
                        default: None,
                    }],
                    flags: vec![UsageFlag {
                        name: "dry-run".into(),
                        usage: "--dry-run".into(),
                        help: Some("Dry run mode".into()),
                        long: vec!["dry-run".into()],
                        short: vec!["n".into()],
                        required: false,
                        arg: None,
                        default: None,
                    }],
                    help: None,
                    examples: vec![],
                },
            }),
            ..Default::default()
        },
        MiseTask {
            name: "exp:中文构建🚀".into(),
            description: "宽字符测试任务".into(),
            run: vec!["echo 你好世界".into()],
            ..Default::default()
        },
        MiseTask {
            name: "_secret_clean".into(),
            description: "Hidden cleanup task".into(),
            hide: true,
            run: vec!["rm -rf tmp".into()],
            ..Default::default()
        },
        MiseTask {
            name: "//packages/web:dev".into(),
            description: "Monorepo web dev server".into(),
            run: vec!["npm run dev".into()],
            ..Default::default()
        },
    ];
    app.handle_event(AppEvent::MiseData(MiseDataEvent {
        generation: gen_id,
        domain: Domain::Tasks,
        payload: MisePayload::Tasks(tasks),
        log_entry: Some(CommandLogEntry {
            id: 1,
            timestamp: std::time::SystemTime::now(),
            source: CommandSource::Internal,
            command: "mise tasks ls --json".into(),
            job_id: None,
            status: JobStatus::Success { duration_ms: 18 },
            summary: Some("5 tasks".into()),
        }),
    }));

    app.handle_event(AppEvent::MiseData(MiseDataEvent {
        generation: gen_id,
        domain: Domain::TaskDeps,
        payload: MisePayload::TaskDeps {
            task_name: "check".into(),
            tree_lines: vec!["check".into(), "├─ tidy".into(), "└─ test".into()],
        },
        log_entry: None,
    }));
    app.handle_event(AppEvent::MiseData(MiseDataEvent {
        generation: gen_id,
        domain: Domain::TaskHistory,
        payload: MisePayload::TaskHistory {
            task_name: "check".into(),
            entries: vec![TaskHistoryEntry {
                task: "check".into(),
                args: vec![],
                started_at: "2026-11-26 10:00:00".into(),
                duration_ms: 850,
                status: "ok".into(),
                exit_code: Some(0),
            }],
        },
        log_entry: None,
    }));

    // 2. Tools + Outdated + Audit + Registry + LockDryRun + ToolDetail
    let mut raw_tools = BTreeMap::new();
    raw_tools.insert(
        "rust".into(),
        vec![MiseToolEntry {
            version: "1.90.0".into(),
            requested_version: Some("stable".into()),
            source: Some(MiseToolSource {
                kind: Some("mise.toml".into()),
                path: Some(workdir.join("mise.toml").display().to_string()),
            }),
            installed: true,
            active: true,
            ..Default::default()
        }],
    );
    raw_tools.insert(
        "node".into(),
        vec![MiseToolEntry {
            version: "22.1.0".into(),
            requested_version: Some("22".into()),
            source: Some(MiseToolSource {
                kind: Some("mise.toml".into()),
                path: Some(workdir.join("mise.toml").display().to_string()),
            }),
            installed: true,
            active: true,
            ..Default::default()
        }],
    );
    app.handle_event(AppEvent::MiseData(MiseDataEvent {
        generation: gen_id,
        domain: Domain::Tools,
        payload: MisePayload::Tools(raw_tools),
        log_entry: None,
    }));

    let mut outdated = BTreeMap::new();
    outdated.insert(
        "node".into(),
        OutdatedEntry {
            name: "node".into(),
            requested: Some("22".into()),
            current: Some("22.1.0".into()),
            latest: "22.14.0".into(),
            release_url: Some("https://nodejs.org/en/blog".into()),
            ..Default::default()
        },
    );
    app.handle_event(AppEvent::MiseData(MiseDataEvent {
        generation: gen_id,
        domain: Domain::Outdated,
        payload: MisePayload::Outdated(outdated),
        log_entry: None,
    }));

    app.handle_event(AppEvent::MiseData(MiseDataEvent {
        generation: gen_id,
        domain: Domain::Audit,
        payload: MisePayload::Audit(AuditReport {
            vulnerabilities: vec![Vulnerability {
                tool: "node".into(),
                version: "22.1.0".into(),
                id: "CVE-2026-9999".into(),
                severity: Some("HIGH".into()),
                summary: "Demo vulnerability".into(),
                fixed: vec!["22.14.0".into()],
                ..Default::default()
            }],
            unchecked: vec![],
        }),
        log_entry: None,
    }));

    app.handle_event(AppEvent::MiseData(MiseDataEvent {
        generation: gen_id,
        domain: Domain::Registry,
        payload: MisePayload::Registry(vec![
            RegistryEntry {
                short: "ripgrep".into(),
                backends: vec!["aqua:BurntSushi/ripgrep".into()],
                bins: vec!["rg".into()],
                description: Some("Fast search tool".into()),
                aliases: vec!["rg".into()],
            },
            RegistryEntry {
                short: "fd".into(),
                backends: vec!["aqua:sharkdp/fd".into()],
                bins: vec!["fd".into()],
                description: Some("Simple fast alternative to find".into()),
                aliases: vec![],
            },
        ]),
        log_entry: None,
    }));

    app.handle_event(AppEvent::MiseData(MiseDataEvent {
        generation: gen_id,
        domain: Domain::LockDryRun,
        payload: MisePayload::LockDryRun("+ [tools.node]\n+ version = \"22.1.0\"".into()),
        log_entry: None,
    }));
    app.handle_event(AppEvent::MiseData(MiseDataEvent {
        generation: gen_id,
        domain: Domain::ToolDetail,
        payload: MisePayload::ToolDetail {
            tool_name: "node".into(),
            detail: Box::new(ToolDetail {
                backend: Some("core:node".into()),
                description: Some("Node.js JavaScript runtime".into()),
                url: Some("https://nodejs.org".into()),
                ..Default::default()
            }),
        },
        log_entry: None,
    }));

    // 3. Env (normal + sensitive + officially redacted + PATH + vars)
    let mut env_ext = BTreeMap::new();
    env_ext.insert(
        "OUT_DIR".into(),
        EnvExtendedEntry {
            value: "/tmp/outputs".into(),
            source: Some(workdir.join("mise.toml").display().to_string()),
            tool: None,
        },
    );
    env_ext.insert(
        "API_TOKEN".into(),
        EnvExtendedEntry {
            value: "secret-token-xyz".into(),
            source: Some(workdir.join("mise.local.toml").display().to_string()),
            tool: None,
        },
    );
    let mut redacted = BTreeSet::new();
    redacted.insert("API_TOKEN".into());
    let mut vars = BTreeMap::new();
    vars.insert("project".into(), json!("lazymise"));
    app.handle_event(AppEvent::MiseData(MiseDataEvent {
        generation: gen_id,
        domain: Domain::Env,
        payload: MisePayload::Env {
            extended: env_ext,
            redacted_keys: redacted,
            paths: vec!["/usr/bin".into(), "/nonexistent-lazymise-path".into()],
            vars,
        },
        log_entry: None,
    }));

    // 4. Config + Trust + Explain + Doctor + Settings + Daemons + Deps + Templates
    let cfg_path = workdir.join("mise.toml").display().to_string();
    app.handle_event(AppEvent::MiseData(MiseDataEvent {
        generation: gen_id,
        domain: Domain::Config,
        payload: MisePayload::Config {
            files: vec![ConfigFileEntry {
                path: cfg_path.clone(),
                tools: vec!["rust".into(), "node".into()],
                is_override: false,
            }],
            trust: vec![TrustEntry {
                path: cfg_path,
                trusted: true,
                raw_status: "trusted".into(),
            }],
        },
        log_entry: None,
    }));
    app.handle_event(AppEvent::MiseData(MiseDataEvent {
        generation: gen_id,
        domain: Domain::ConfigExplain,
        payload: MisePayload::ConfigExplain(vec![ConfigExplainItem {
            key: "legacy_flag".into(),
            line: 2,
            deprecated: true,
            known: true,
            suggestion: Some("tools".into()),
            ..Default::default()
        }]),
        log_entry: None,
    }));
    app.handle_event(AppEvent::MiseData(MiseDataEvent {
        generation: gen_id,
        domain: Domain::Doctor,
        payload: MisePayload::Doctor(Box::new(DoctorReport {
            activated: true,
            version: "2026.11.13".into(),
            shell: Some(DoctorShell {
                name: "zsh".into(),
                version: "5.9".into(),
            }),
            warnings: vec!["Sample warning".into()],
            errors: vec![],
            shims_on_path: true,
            ..Default::default()
        })),
        log_entry: None,
    }));
    app.handle_event(AppEvent::MiseData(MiseDataEvent {
        generation: gen_id,
        domain: Domain::Settings,
        payload: MisePayload::Settings(vec![SettingEntry {
            key: "experimental".into(),
            value: json!(true),
            kind: "bool".into(),
            description: "Enable experimental features".into(),
            source: None,
        }]),
        log_entry: None,
    }));
    app.handle_event(AppEvent::MiseData(MiseDataEvent {
        generation: gen_id,
        domain: Domain::Daemons,
        payload: MisePayload::Daemons(vec![DaemonEntry {
            name: "postgres".into(),
            status: "running".into(),
            pid: Some(12345),
            url: Some("postgres://localhost:5432".into()),
            ..Default::default()
        }]),
        log_entry: None,
    }));
    app.handle_event(AppEvent::MiseData(MiseDataEvent {
        generation: gen_id,
        domain: Domain::Deps,
        payload: MisePayload::Deps(vec![DepEntry {
            name: "cargo".into(),
            status: "fresh".into(),
            detail: "Cargo.lock synced".into(),
        }]),
        log_entry: None,
    }));
    app.handle_event(AppEvent::MiseData(MiseDataEvent {
        generation: gen_id,
        domain: Domain::Templates,
        payload: MisePayload::Templates(vec![WorkspaceTemplate {
            name: "rust-cli".into(),
            path: PathBuf::from("/tmp/templates/rust-cli"),
            files: vec!["mise.toml".into(), "Cargo.toml".into()],
            mise_toml_preview: "[tools]\nrust = \"stable\"\n".into(),
        }]),
        log_entry: None,
    }));

    // Clear startup toasts from Audit so subsequent assertions are deterministic
    app.state.toasts.clear();
}

#[test]
fn e2e_five_panels_subviews_zoom_narrow_and_cjk_render() {
    let (mut app, _rx) = make_offline_app(PathBuf::from("/tmp/lazymise-e2e"), Panel::Tasks);
    seed_full_workspace(&mut app);

    // Wide 140x40 render on Tasks
    let frame_tasks = render_frame(&mut app, 140, 40);
    assert!(frame_tasks.contains("lazymise"));
    assert!(frame_tasks.contains("[1] TASKS"));
    assert!(frame_tasks.contains("[2] TOOLS"));
    assert!(frame_tasks.contains("[3] ENV"));
    assert!(frame_tasks.contains("[4] CONFIG & HEALTH"));
    assert!(frame_tasks.contains("[5] COMMAND LOG & JOBS"));
    assert!(frame_tasks.contains("中文构建"));
    assert!(!app.layout_map.is_narrow);

    // Switch through Tasks sub-views via `]` / `[`
    app.handle_event(key(KeyCode::Char(']')));
    assert_eq!(app.state.task_subview, TaskSubView::Output);
    app.handle_event(key(KeyCode::Char(']')));
    assert_eq!(app.state.task_subview, TaskSubView::Deps);
    let deps_frame = render_frame(&mut app, 140, 40);
    assert!(deps_frame.contains("tidy"));
    app.handle_event(key(KeyCode::Char(']')));
    assert_eq!(app.state.task_subview, TaskSubView::History);
    let hist_frame = render_frame(&mut app, 140, 40);
    assert!(hist_frame.contains("RECENT EXECUTIONS"));

    // Tools panel (`2`): Scope cycle + Outdated/Audit/Lock subviews
    app.handle_event(key(KeyCode::Char('2')));
    assert_eq!(app.state.active_panel, Panel::Tools);
    let tools_frame = render_frame(&mut app, 140, 40);
    assert!(tools_frame.contains("node"));
    assert!(tools_frame.contains("22.14.0"));
    app.handle_event(key(KeyCode::Char(']')));
    assert_eq!(app.state.tool_subview, ToolSubView::Outdated);
    app.handle_event(key(KeyCode::Char(']')));
    assert_eq!(app.state.tool_subview, ToolSubView::Audit);
    let audit_frame = render_frame(&mut app, 140, 40);
    assert!(audit_frame.contains("CVE-2026-9999"));
    app.handle_event(key(KeyCode::Char(']')));
    assert_eq!(app.state.tool_subview, ToolSubView::Lock);
    let lock_frame = render_frame(&mut app, 140, 40);
    assert!(lock_frame.contains("[tools.node]"));
    app.handle_event(key(KeyCode::Char('f')));
    assert_eq!(app.state.tool_scope, ToolScope::Active);

    // Env panel (`3`): secret masked by default, `S` reveals with 30s countdown, `PATH` and `Vars`
    app.handle_event(key(KeyCode::Char('3')));
    assert_eq!(app.state.active_panel, Panel::Env);
    // Select API_TOKEN (sorted alphabetically first)
    let env_masked = render_frame(&mut app, 140, 40);
    assert!(env_masked.contains("API_TOKEN"));
    assert!(!env_masked.contains("secret-token-xyz"));
    app.handle_event(key(KeyCode::Char('S')));
    let env_revealed = render_frame(&mut app, 140, 40);
    assert!(env_revealed.contains("secret-token-xyz"));
    assert!(env_revealed.contains("REVEALED"));
    // Expire reveal via Tick -> re-masks automatically
    app.state
        .revealed_env_keys
        .insert("API_TOKEN".into(), Instant::now() - Duration::from_secs(1));
    app.handle_event(AppEvent::Tick);
    assert!(!app.state.is_env_revealed("API_TOKEN"));
    app.handle_event(key(KeyCode::Char(']')));
    assert_eq!(app.state.env_subview, EnvSubView::Paths);
    let paths_frame = render_frame(&mut app, 140, 40);
    assert!(paths_frame.contains("MISSING"));
    app.handle_event(key(KeyCode::Char(']')));
    assert_eq!(app.state.env_subview, EnvSubView::Vars);
    let vars_frame = render_frame(&mut app, 140, 40);
    assert!(vars_frame.contains("lazymise"));
    app.handle_event(key(KeyCode::Char('f')));
    assert_eq!(app.state.env_filter, EnvFilter::Project);

    // Config panel (`4`): File & Explain, Doctor, Settings, Daemons & Deps
    app.handle_event(key(KeyCode::Char('4')));
    assert_eq!(app.state.active_panel, Panel::Config);
    let cfg_frame = render_frame(&mut app, 140, 40);
    assert!(cfg_frame.contains("DEPRECATED"));
    app.handle_event(key(KeyCode::Char(']')));
    assert_eq!(app.state.config_subview, ConfigSubView::Doctor);
    let doc_frame = render_frame(&mut app, 140, 40);
    assert!(doc_frame.contains("Sample warning"));
    app.handle_event(key(KeyCode::Char(']')));
    assert_eq!(app.state.config_subview, ConfigSubView::Settings);
    let set_frame = render_frame(&mut app, 140, 40);
    assert!(set_frame.contains("experimental"));
    app.handle_event(key(KeyCode::Char(']')));
    assert_eq!(app.state.config_subview, ConfigSubView::Services);
    let svc_frame = render_frame(&mut app, 140, 40);
    assert!(svc_frame.contains("postgres"));
    assert!(svc_frame.contains("cargo"));

    // Zoom cycle `+` / `_` and Log drawer `@`
    app.handle_event(key(KeyCode::Char('+')));
    assert_eq!(app.state.screen_mode, ScreenMode::Half);
    assert!(render_frame(&mut app, 140, 40).contains("ZOOM:HALF"));
    app.handle_event(key(KeyCode::Char('+')));
    assert_eq!(app.state.screen_mode, ScreenMode::Full);
    assert!(render_frame(&mut app, 140, 40).contains("ZOOM:FULL"));
    app.handle_event(key(KeyCode::Char('_')));
    app.handle_event(key(KeyCode::Char('_')));
    assert_eq!(app.state.screen_mode, ScreenMode::Normal);

    app.handle_event(key(KeyCode::Char('@')));
    assert_eq!(app.state.log_mode, LogPanelMode::Expanded);
    app.handle_event(key(KeyCode::Char('@')));
    assert_eq!(app.state.log_mode, LogPanelMode::Hidden);
    app.handle_event(key(KeyCode::Char('@')));
    assert_eq!(app.state.log_mode, LogPanelMode::Compact);

    // Narrow 80x24 degradation and tiny 40x10 safety
    let narrow = render_frame(&mut app, 80, 24);
    assert!(app.layout_map.is_narrow);
    assert!(narrow.contains("LIST ←→ detail"));
    assert_eq!(app.layout_map.header_panels.len(), 5);

    let tiny = render_frame(&mut app, 40, 10);
    assert!(tiny.contains("[1]"));
}

#[test]
fn e2e_overlays_help_context_palette_registry_args_env_and_generate() {
    let (mut app, _rx) = make_offline_app(PathBuf::from("/tmp/lazymise-e2e"), Panel::Tasks);
    seed_full_workspace(&mut app);

    // 1. `?` Help search + Enter executes action directly
    app.handle_event(key(KeyCode::Char('?')));
    send_str(&mut app, "zoom");
    let help_frame = render_frame(&mut app, 140, 40);
    assert!(help_frame.contains("Keybindings Help"));
    app.handle_event(key(KeyCode::Enter));
    assert!(matches!(app.state.overlay, Overlay::None));
    assert_eq!(app.state.screen_mode, ScreenMode::Half);
    app.handle_event(key(KeyCode::Char('_')));

    // 2. `x` Context Menu -> shortcut key or Enter
    app.handle_event(key(KeyCode::Char('x')));
    let menu_frame = render_frame(&mut app, 140, 40);
    assert!(menu_frame.contains("TASKS ACTIONS"));
    app.handle_event(key(KeyCode::Char('H')));
    assert!(app.state.show_hidden_tasks);

    // 3. `:` / `Ctrl-p` Command Palette
    app.handle_event(ctrl('p'));
    send_str(&mut app, "doctor");
    let pal_frame = render_frame(&mut app, 140, 40);
    assert!(pal_frame.contains("Command Palette"));
    app.handle_event(key(KeyCode::Enter));
    assert_eq!(app.state.active_panel, Panel::Config);
    assert_eq!(app.state.config_subview, ConfigSubView::Doctor);

    // 4. `E` MISE_ENV Profile Switcher
    app.handle_event(key(KeyCode::Char('E')));
    send_str(&mut app, "staging,local");
    let prof_frame = render_frame(&mut app, 140, 40);
    assert!(prof_frame.contains("staging,local"));
    app.handle_event(key(KeyCode::Enter));
    assert_eq!(app.state.profile.as_deref(), Some("staging,local"));
    // Re-seed under the newly bumped generation
    seed_full_workspace(&mut app);

    // 5. Tools `a` Registry 3-step search -> version -> scope -> Confirm command preview
    app.handle_event(key(KeyCode::Char('2')));
    app.handle_event(key(KeyCode::Char('a')));
    send_str(&mut app, "rg");
    let reg_frame = render_frame(&mut app, 140, 40);
    assert!(reg_frame.contains("ripgrep"));
    app.handle_event(key(KeyCode::Enter));
    assert!(matches!(
        app.state.overlay,
        Overlay::RegistryPicker {
            step: RegistryStep::PickVersion { .. },
            ..
        }
    ));
    // Simulate async `mise ls-remote ripgrep` arrival
    let gen_id = app.state.generation;
    app.handle_event(AppEvent::MiseData(MiseDataEvent {
        generation: gen_id,
        domain: Domain::ToolVersions,
        payload: MisePayload::ToolVersions {
            tool_name: "ripgrep".into(),
            versions: vec!["14.0.0".into(), "14.1.1".into()],
        },
        log_entry: None,
    }));
    let ver_frame = render_frame(&mut app, 140, 40);
    assert!(ver_frame.contains("14.1.1"));
    app.handle_event(key(KeyCode::Down)); // pick 14.1.1
    app.handle_event(key(KeyCode::Enter)); // -> PickScope
    app.handle_event(key(KeyCode::Down)); // pick Global (-g)
    app.handle_event(key(KeyCode::Enter)); // -> Confirm
    let confirm_frame = render_frame(&mut app, 140, 40);
    assert!(confirm_frame.contains("mise -E staging,local use -g ripgrep@14.1.1"));
    app.handle_event(key(KeyCode::Esc));

    // 6. Tasks `A` usage_spec form validation & command building
    app.handle_event(key(KeyCode::Char('1')));
    // Select `deploy` task
    let deploy_idx = app
        .state
        .task_rows
        .iter()
        .position(|r| r.as_task().is_some_and(|t| t.name == "deploy"))
        .expect("deploy task row exists");
    app.state.list_states.tasks.select(Some(deploy_idx));
    // Pressing Enter on task with required usage_spec auto-opens TaskArgsForm
    app.handle_event(key(KeyCode::Enter));
    assert!(matches!(app.state.overlay, Overlay::TaskArgsForm { .. }));
    let args_frame = render_frame(&mut app, 140, 40);
    assert!(args_frame.contains("Run Task `deploy`"));
    // Pressing Enter with empty required field warns & stays open
    app.handle_event(key(KeyCode::Enter));
    assert!(matches!(app.state.overlay, Overlay::TaskArgsForm { .. }));
    assert_eq!(
        app.state.active_toast().map(|t| t.level),
        Some(ToastLevel::Warning)
    );
    // Fill required arg `env = prod`, toggle `--force`, submit -> pending_fg
    send_str(&mut app, "prod");
    app.handle_event(ctrl('f'));
    app.handle_event(key(KeyCode::Enter));
    assert!(matches!(app.state.overlay, Overlay::None));
    assert!(matches!(
        app.state.pending_fg,
        Some(ForegroundRequest::RunTask {
            ref name,
            ref args,
            force: true,
        }) if name == "deploy" && args == &vec!["prod".to_string()]
    ));
    app.state.pending_fg = None;

    // 7. Config `g` Generate & Templates modal (official target + custom template name input)
    app.handle_event(key(KeyCode::Char('4')));
    app.handle_event(key(KeyCode::Char('g')));
    let gen_frame = render_frame(&mut app, 140, 40);
    assert!(gen_frame.contains("mise generate config"));
    assert!(gen_frame.contains("template: rust-cli"));
    // Move to last item (`template: rust-cli`) and scaffold
    app.handle_event(key(KeyCode::Char('k'))); // wraps to last
    app.handle_event(key(KeyCode::Enter));
    send_str(&mut app, "my-app");
    app.handle_event(key(KeyCode::Enter));
    assert!(matches!(
        app.state.overlay,
        Overlay::Confirm { ref command_preview, .. } if command_preview.contains("run new rust-cli my-app")
    ));
    app.handle_event(key(KeyCode::Esc));
}

#[test]
fn e2e_mouse_layout_map_clicks_hover_wheel_and_overlay_dismiss() {
    let (mut app, _rx) = make_offline_app(PathBuf::from("/tmp/lazymise-e2e"), Panel::Tasks);
    seed_full_workspace(&mut app);

    // Render wide frame so LayoutMap holds real coordinates
    render_frame(&mut app, 140, 40);

    // 1. Click header profile badge -> opens ProfileSwitcher
    let prof_rect = app.layout_map.header_profile;
    assert!(prof_rect.width > 0);
    app.handle_event(click(prof_rect.x + 1, prof_rect.y));
    assert!(matches!(app.state.overlay, Overlay::ProfileSwitcher { .. }));

    // Render overlay so overlay_area is populated, then click outside (0,0) -> closes overlay
    render_frame(&mut app, 140, 40);
    assert!(app.layout_map.overlay_area.is_some());
    app.handle_event(click(0, 0));
    assert!(matches!(app.state.overlay, Overlay::None));

    // 2. Click collapsed `[2] TOOLS` accordion card -> focuses Tools panel
    render_frame(&mut app, 140, 40);
    let tools_outer = app
        .layout_map
        .sidebar_panels
        .iter()
        .find(|(p, _, _)| *p == Panel::Tools)
        .map(|(_, outer, _)| *outer)
        .expect("Tools accordion card registered");
    app.handle_event(click(tools_outer.x + 2, tools_outer.y + 1));
    assert_eq!(app.state.active_panel, Panel::Tools);
    assert_eq!(app.state.focus, FocusArea::Sidebar);

    // 3. Click sub-tab 2 (`Audit`) in Main pane -> switches ToolSubView::Audit & focuses Main
    render_frame(&mut app, 140, 40);
    let audit_tab = app
        .layout_map
        .main_subtabs
        .iter()
        .find(|(idx, _)| *idx == 2)
        .map(|(_, r)| *r)
        .expect("Audit subtab registered");
    app.handle_event(click(audit_tab.x + 1, audit_tab.y));
    assert_eq!(app.state.tool_subview, ToolSubView::Audit);
    assert_eq!(app.state.focus, FocusArea::Main);

    // 4. Hover wheel scroll over Main while Sidebar is focused scrolls Main without stealing focus
    app.handle_event(key(KeyCode::Char('1')));
    app.state.task_subview = TaskSubView::Spec;
    render_frame(&mut app, 140, 40);
    app.state.detail_scroll.content_len = 80;
    app.state.detail_scroll.viewport_height = 15;
    let main_rect = app.layout_map.main_area;
    assert_eq!(app.state.focus, FocusArea::Sidebar);
    app.handle_event(wheel_down(main_rect.x + 5, main_rect.y + 5));
    assert_eq!(app.state.detail_scroll.offset, 3);
    assert_eq!(app.state.focus, FocusArea::Sidebar);

    // 5. Narrow screen header `[3:ENV]` badge click switches directly to Env
    render_frame(&mut app, 80, 24);
    let env_badge = app
        .layout_map
        .header_panels
        .iter()
        .find(|(p, _)| *p == Panel::Env)
        .map(|(_, r)| *r)
        .expect("narrow header Env badge registered");
    app.handle_event(click(env_badge.x + 1, env_badge.y));
    assert_eq!(app.state.active_panel, Panel::Env);
}

#[test]
fn e2e_runner_ansi_streaming_job_isolation_internal_toggle_and_quit_guard() {
    let (mut app, _rx) = make_offline_app(PathBuf::from("/tmp/lazymise-e2e"), Panel::Tasks);
    seed_full_workspace(&mut app);

    // Register two concurrent jobs with distinct JobIds
    for (id_num, name) in [(10u64, "check"), (11u64, "deploy")] {
        let jid = JobId(id_num);
        app.state.jobs.insert(
            jid,
            JobRecord {
                id: jid,
                label: format!("task:{name}"),
                task_name: Some(name.into()),
                command_display: format!("mise run {name}"),
                pid: None,
                started_at: Instant::now(),
                status: JobStatus::Running,
                lines: vec![],
            },
        );
        app.state.latest_task_job.insert(name.into(), jid);
        app.state.command_log.record(
            CommandSource::User,
            format!("mise run {name}"),
            Some(jid),
            JobStatus::Running,
            None,
        );
    }
    app.state.selected_job_id = Some(JobId(10));
    app.state.task_subview = TaskSubView::Output;

    // Stream ANSI + `\r` progress overwrite lines into Job #10 and separate line into Job #11
    app.handle_event(AppEvent::Runner(RunnerEvent::Started {
        job_id: JobId(10),
        pid: Some(7777),
    }));
    app.handle_event(AppEvent::Runner(RunnerEvent::Output {
        job_id: JobId(10),
        line: JobOutputLine {
            text: "\x1b[33m50% building...\x1b[0m".into(),
            is_stderr: false,
        },
        overwrite_last: false,
    }));
    app.handle_event(AppEvent::Runner(RunnerEvent::Output {
        job_id: JobId(10),
        line: JobOutputLine {
            text: "\x1b[1;32m100% compiled ok\x1b[0m".into(),
            is_stderr: false,
        },
        overwrite_last: true,
    }));
    app.handle_event(AppEvent::Runner(RunnerEvent::Output {
        job_id: JobId(11),
        line: JobOutputLine {
            text: "deploy-only-line".into(),
            is_stderr: false,
        },
        overwrite_last: false,
    }));

    assert_eq!(app.state.jobs[&JobId(10)].pid, Some(7777));
    assert_eq!(app.state.jobs[&JobId(10)].lines.len(), 1);
    assert!(app.state.jobs[&JobId(10)].lines[0].text.contains("100% compiled ok"));
    assert_eq!(app.state.jobs[&JobId(11)].lines.len(), 1);

    // Render Jobs panel (`5`) and verify ANSI text without raw ESC leakage
    app.handle_event(key(KeyCode::Char('5')));
    app.state.list_states.jobs.select(Some(1)); // job #10 (newest-first: #11 is idx 0, #10 is idx 1)
    let jobs_frame = render_frame(&mut app, 140, 40);
    assert!(jobs_frame.contains("100% compiled ok"));
    assert!(!jobs_frame.contains("\x1b["));

    // Toggle internal queries with `i`
    let user_only_count = app.state.filtered_command_log().len();
    assert_eq!(user_only_count, 2);
    app.handle_event(key(KeyCode::Char('i')));
    assert!(app.state.show_internal_commands);
    assert_eq!(app.state.filtered_command_log().len(), 3); // includes `mise tasks ls --json`

    // Finish job #10, leave #11 running -> `q` triggers QuitWithRunningJobs confirm
    app.handle_event(AppEvent::Runner(RunnerEvent::Finished {
        job_id: JobId(10),
        exit_code: 0,
        duration_ms: 420,
        killed: false,
    }));
    assert_eq!(app.state.running_jobs_count(), 1);
    app.handle_event(key(KeyCode::Char('q')));
    assert!(!app.state.should_quit);
    assert!(matches!(app.state.overlay, Overlay::Confirm { .. }));
    app.handle_event(key(KeyCode::Esc));

    // Double Ctrl-C forces quit cleanly
    app.handle_event(ctrl('c'));
    assert!(!app.state.should_quit);
    app.handle_event(ctrl('c'));
    assert!(app.state.should_quit);

    // Confirm PendingChord & LayoutMap exports remain accessible
    let _ = PendingChord::None;
    let _ = LayoutMap::default();
}

#[test]
fn e2e_cli_compatibility_and_no_color_palette() {
    let cli = Cli::try_parse_from([
        "lazymise",
        "-C",
        "/tmp/demo",
        "-E",
        "dev",
        "--tab",
        "templates",
    ])
    .unwrap();
    assert_eq!(cli.dir, Some(PathBuf::from("/tmp/demo")));
    assert_eq!(cli.env.as_deref(), Some("dev"));
    assert_eq!(Panel::from(cli.tab), Panel::Config);

    let cap = detect_color_capability_from(Some("1"), None, Some("truecolor"), Some("xterm-256color"));
    assert_eq!(cap, ColorCapability::NoColor);
    let pal = Palette::for_capability(cap);
    assert!(pal.is_no_color());
    assert_eq!(pal.cyan, None);
    assert_eq!(pal.red, None);
}

#[test]
fn e2e_verifier_regressions_palette_jump_secret_guard_daemon_and_mouse_offset() {
    let (mut app, _rx) = make_offline_app(PathBuf::from("/tmp/lazymise-e2e"), Panel::Tools);
    seed_full_workspace(&mut app);

    // 1. Command Palette task jump from Tools panel (`:` -> "deploy" -> Enter)
    app.handle_event(key(KeyCode::Char(':')));
    send_str(&mut app, "deploy");
    let pal_frame = render_frame(&mut app, 140, 40);
    assert!(pal_frame.contains("Task: deploy"));
    app.handle_event(key(KeyCode::Enter));
    assert_eq!(app.state.active_panel, Panel::Tasks);
    assert_eq!(app.state.selected_task().map(|t| t.name.as_str()), Some("deploy"));

    // 2. Env editing a masked secret must NOT pre-fill plaintext or leak into Confirm preview
    app.handle_event(key(KeyCode::Char('3')));
    assert!(!app.state.is_env_revealed("API_TOKEN"));
    app.handle_event(key(KeyCode::Char('e')));
    let form_frame = render_frame(&mut app, 140, 40);
    assert!(!form_frame.contains("secret-token-xyz"));
    // Type new secret value -> submit -> Confirm preview must mask the secret value
    send_str(&mut app, "new-super-secret-999");
    app.handle_event(key(KeyCode::Enter));
    let confirm_frame = render_frame(&mut app, 140, 40);
    assert!(confirm_frame.contains("API_TOKEN=••••••••"));
    assert!(!confirm_frame.contains("new-super-secret-999"));
    app.handle_event(key(KeyCode::Esc));

    // 3. Config -> Services daemon navigation (`j`/`k` in Main focus)
    let gen_id = app.state.generation;
    app.handle_event(AppEvent::MiseData(MiseDataEvent {
        generation: gen_id,
        domain: Domain::Daemons,
        payload: MisePayload::Daemons(vec![
            DaemonEntry {
                name: "postgres".into(),
                status: "running".into(),
                pid: Some(100),
                ..Default::default()
            },
            DaemonEntry {
                name: "redis".into(),
                status: "started".into(),
                pid: Some(200),
                ..Default::default()
            },
        ]),
        log_entry: None,
    }));
    app.handle_event(key(KeyCode::Char('4')));
    app.state.config_subview = ConfigSubView::Services;
    app.state.focus = FocusArea::Main;
    assert_eq!(app.state.selected_daemon_idx, 0);
    app.handle_event(key(KeyCode::Char('j')));
    assert_eq!(app.state.selected_daemon_idx, 1);
    let svc_frame = render_frame(&mut app, 140, 40);
    assert!(svc_frame.contains("redis"));
    // Pressing `s` on `started` daemon offers `stop` (unified status check)
    app.handle_event(key(KeyCode::Char('s')));
    assert!(matches!(
        app.state.overlay,
        Overlay::Confirm { ref command_preview, .. } if command_preview.contains("daemons stop redis")
    ));
    app.handle_event(key(KeyCode::Esc));

    // 4. Mouse click on Tasks row while Jobs panel is active & scrolled uses Tasks offset, not Jobs offset
    app.handle_event(key(KeyCode::Char('5')));
    *app.state.list_states.jobs.offset_mut() = 10;
    *app.state.list_states.tasks.offset_mut() = 0;
    render_frame(&mut app, 140, 40);
    let tasks_inner = app
        .layout_map
        .sidebar_panels
        .iter()
        .find(|(p, _, _)| *p == Panel::Tasks)
        .map(|(_, _, inner)| *inner)
        .expect("Tasks sidebar inner registered while Jobs is active");
    assert!(tasks_inner.height >= 2);
    // Click row 1 of Tasks (index 1, first task under [root])
    app.handle_event(click(tasks_inner.x + 2, tasks_inner.y + 1));
    assert_eq!(app.state.active_panel, Panel::Tasks);
    assert_eq!(app.state.list_states.tasks.selected(), Some(1));

    // 5. TaskToggleHidden (`H`) increments generation so stale Tasks payloads are ignored
    let gen_before = app.state.generation;
    app.handle_event(key(KeyCode::Char('H')));
    assert!(app.state.generation > gen_before);
}
