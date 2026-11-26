use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::action::{
    Action, ConfigSubView, Domain, FocusArea, LogPanelMode, Panel, TaskSubView, ToastLevel,
    ToolSubView,
};
use crate::app::keymap::context_menu_items;
use crate::app::state::{
    AppState, ArgField, ConfirmAction, FORCE_QUIT_WINDOW, ForegroundRequest, Overlay,
    RegistryStep, SECRET_REVEAL_TTL, TextInput,
};
use crate::events::{
    CommandSource, JobId, JobOutputLine, JobRecord, JobStatus, MiseDataEvent, MisePayload,
    RunnerEvent,
};
use crate::mise::loader::MiseLoader;
use crate::mise::schema::MiseTask;
use crate::model::task_tree::{TaskRow, task_group_key};
use crate::runner::{
    BackgroundRunner, command_exists, copy_to_clipboard_detailed, split_shell_words,
};

/// Trigger Tier-1 hot-path parallel reload (and optional Tier-2 background warmup).
pub fn trigger_hot_reload(state: &mut AppState, loader: &MiseLoader, warmup_tier2: bool) {
    let generation = loader.next_generation();
    state.generation = generation;
    state.loading_domains.clear();
    for d in [
        Domain::Tasks,
        Domain::Tools,
        Domain::Env,
        Domain::Config,
        Domain::Templates,
    ] {
        state.loading_domains.insert(d);
    }

    loader.request_hot_reload(
        generation,
        state.profile.clone(),
        state.show_hidden_tasks,
        state.monorepo_all_tasks,
    );

    if warmup_tier2 {
        // Warm up fast 20ms registry + slow background diagnostics without blocking startup
        for d in [Domain::Registry, Domain::Outdated, Domain::Doctor] {
            state.loading_domains.insert(d);
            state.requested_domains.insert(d);
            loader.request_domain(generation, state.profile.clone(), d, None);
        }
    }

    state.dirty = true;
}

/// Ensure lazy/on-demand domains for the active panel, subview, and selection are fetched.
pub fn ensure_active_data_loaded(state: &mut AppState, loader: &MiseLoader) {
    let gen_id = state.generation;
    let profile = state.profile.clone();

    match state.active_panel {
        Panel::Tasks => {
            if let Some(task) = state.selected_task().cloned() {
                let _ = state.script_cache.preload_task(&task);
                match state.task_subview {
                    TaskSubView::Spec => {
                        if !state.task_info_cache.contains_key(&task.name)
                            && !task.usage.trim().is_empty()
                        {
                            state.loading_domains.insert(Domain::TaskInfo);
                            loader.request_domain(
                                gen_id,
                                profile.clone(),
                                Domain::TaskInfo,
                                Some(task.name.clone()),
                            );
                        }
                    }
                    TaskSubView::Deps => {
                        if !state.task_deps_cache.contains_key(&task.name) {
                            state.loading_domains.insert(Domain::TaskDeps);
                            loader.request_domain(
                                gen_id,
                                profile.clone(),
                                Domain::TaskDeps,
                                Some(task.name.clone()),
                            );
                        }
                    }
                    TaskSubView::History => {
                        if !state.task_history_cache.contains_key(&task.name) {
                            state.loading_domains.insert(Domain::TaskHistory);
                            loader.request_domain(
                                gen_id,
                                profile.clone(),
                                Domain::TaskHistory,
                                Some(task.name.clone()),
                            );
                        }
                    }
                    TaskSubView::Output => {}
                }
            }
        }
        Panel::Tools => {
            if let Some(tool) = state.selected_tool().cloned()
                && !state.tools_vm.details_cache.contains_key(&tool.name)
            {
                state.loading_domains.insert(Domain::ToolDetail);
                loader.request_domain(
                    gen_id,
                    profile.clone(),
                    Domain::ToolDetail,
                    Some(tool.name),
                );
            }
            if !state.requested_domains.contains(&Domain::Outdated) {
                state.requested_domains.insert(Domain::Outdated);
                state.loading_domains.insert(Domain::Outdated);
                loader.request_domain(gen_id, profile.clone(), Domain::Outdated, None);
            }
            if state.tool_subview == ToolSubView::Lock
                && state.tools_vm.lock_preview.is_none()
                && !state.requested_domains.contains(&Domain::LockDryRun)
            {
                state.requested_domains.insert(Domain::LockDryRun);
                state.loading_domains.insert(Domain::LockDryRun);
                loader.request_domain(gen_id, profile, Domain::LockDryRun, None);
            }
        }
        Panel::Config => {
            state.preload_selected_content();
            match state.config_subview {
                ConfigSubView::File => {
                    if !state.requested_domains.contains(&Domain::ConfigExplain) {
                        state.requested_domains.insert(Domain::ConfigExplain);
                        state.loading_domains.insert(Domain::ConfigExplain);
                        loader.request_domain(gen_id, profile, Domain::ConfigExplain, None);
                    }
                }
                ConfigSubView::Doctor => {
                    if !state.requested_domains.contains(&Domain::Doctor) {
                        state.requested_domains.insert(Domain::Doctor);
                        state.loading_domains.insert(Domain::Doctor);
                        loader.request_domain(gen_id, profile, Domain::Doctor, None);
                    }
                }
                ConfigSubView::Settings => {
                    if !state.requested_domains.contains(&Domain::Settings) {
                        state.requested_domains.insert(Domain::Settings);
                        state.loading_domains.insert(Domain::Settings);
                        loader.request_domain(gen_id, profile, Domain::Settings, None);
                    }
                }
                ConfigSubView::Services => {
                    if !state.requested_domains.contains(&Domain::Daemons) {
                        state.requested_domains.insert(Domain::Daemons);
                        state.loading_domains.insert(Domain::Daemons);
                        loader.request_domain(gen_id, profile.clone(), Domain::Daemons, None);
                    }
                    if !state.requested_domains.contains(&Domain::Deps) {
                        state.requested_domains.insert(Domain::Deps);
                        state.loading_domains.insert(Domain::Deps);
                        loader.request_domain(gen_id, profile, Domain::Deps, None);
                    }
                }
            }
        }
        Panel::Env | Panel::Jobs => {}
    }
}

/// Apply a semantic `Action` to `AppState` and dispatch async / runner / foreground side effects.
pub fn apply_action(
    state: &mut AppState,
    action: Action,
    loader: &MiseLoader,
    runner: &BackgroundRunner,
) {
    match action {
        Action::Quit => {
            let running = state.running_jobs_count();
            if running > 0 {
                state.overlay = Overlay::Confirm {
                    title: format!("Quit with {running} running background job(s)?"),
                    detail: "All active background process groups will be terminated cleanly."
                        .to_string(),
                    command_preview: "SIGTERM -> SIGKILL active job process groups".to_string(),
                    action: ConfirmAction::QuitWithRunningJobs,
                };
            } else {
                state.should_quit = true;
            }
        }
        Action::ForceQuit => {
            let running = state.running_jobs_count();
            let now = Instant::now();
            if running == 0 {
                state.should_quit = true;
            } else if state
                .ctrl_c_armed_at
                .is_some_and(|at| now.saturating_duration_since(at) <= FORCE_QUIT_WINDOW)
            {
                runner.kill_all();
                state.should_quit = true;
            } else {
                state.ctrl_c_armed_at = Some(now);
                state.push_toast(
                    ToastLevel::Warning,
                    format!(
                        "{running} job(s) running — press Ctrl-C again within 3s to force quit"
                    ),
                );
            }
        }
        Action::FocusPanel(panel) => {
            state.active_panel = panel;
            state.focus = if panel == Panel::Jobs {
                if state.log_mode == LogPanelMode::Hidden {
                    state.log_mode = LogPanelMode::Compact;
                }
                FocusArea::Log
            } else {
                FocusArea::Sidebar
            };
            state.detail_scroll.scroll_top();
            state.clamp_panel_selection(panel);
            state.preload_selected_content();
            ensure_active_data_loaded(state, loader);
        }
        Action::CycleFocusNext => {
            state.focus = match state.focus {
                FocusArea::Sidebar => FocusArea::Main,
                FocusArea::Main => {
                    if state.log_mode != LogPanelMode::Hidden {
                        FocusArea::Log
                    } else {
                        FocusArea::Sidebar
                    }
                }
                FocusArea::Log => FocusArea::Sidebar,
            };
        }
        Action::CycleFocusPrev => {
            state.focus = match state.focus {
                FocusArea::Sidebar => {
                    if state.log_mode != LogPanelMode::Hidden {
                        FocusArea::Log
                    } else {
                        FocusArea::Main
                    }
                }
                FocusArea::Main => FocusArea::Sidebar,
                FocusArea::Log => FocusArea::Main,
            };
        }
        Action::FocusLeft => {
            if state.focus == FocusArea::Sidebar
                && state.active_panel == Panel::Tasks
                && let Some(TaskRow::Group { key, collapsed, .. }) =
                    state.list_states.tasks.selected().and_then(|i| state.task_rows.get(i))
                && !collapsed
            {
                let k = key.clone();
                state.collapsed_groups.insert(k);
                state.rebuild_task_rows();
                state.dirty = true;
                return;
            }
            state.focus = FocusArea::Sidebar;
        }
        Action::FocusRight => {
            if state.focus == FocusArea::Sidebar
                && state.active_panel == Panel::Tasks
                && let Some(TaskRow::Group { key, collapsed, .. }) =
                    state.list_states.tasks.selected().and_then(|i| state.task_rows.get(i))
                && *collapsed
            {
                let k = key.clone();
                state.collapsed_groups.remove(&k);
                state.rebuild_task_rows();
                state.dirty = true;
                return;
            }
            state.focus = FocusArea::Main;
        }
        Action::NextSubView => {
            match state.active_panel {
                Panel::Tasks => state.task_subview = state.task_subview.next(),
                Panel::Tools => state.tool_subview = state.tool_subview.next(),
                Panel::Env => state.env_subview = state.env_subview.next(),
                Panel::Config => state.config_subview = state.config_subview.next(),
                Panel::Jobs => {}
            }
            state.detail_scroll.scroll_top();
            ensure_active_data_loaded(state, loader);
        }
        Action::PrevSubView => {
            match state.active_panel {
                Panel::Tasks => state.task_subview = state.task_subview.prev(),
                Panel::Tools => state.tool_subview = state.tool_subview.prev(),
                Panel::Env => state.env_subview = state.env_subview.prev(),
                Panel::Config => state.config_subview = state.config_subview.prev(),
                Panel::Jobs => {}
            }
            state.detail_scroll.scroll_top();
            ensure_active_data_loaded(state, loader);
        }
        Action::ZoomIn => {
            state.screen_mode = state.screen_mode.zoom_in();
            state.push_toast(
                ToastLevel::Info,
                format!("Screen mode: {}", state.screen_mode.label()),
            );
        }
        Action::ZoomOut => {
            state.screen_mode = state.screen_mode.zoom_out();
            state.push_toast(
                ToastLevel::Info,
                format!("Screen mode: {}", state.screen_mode.label()),
            );
        }
        Action::ToggleLogPanel => {
            state.log_mode = state.log_mode.cycle();
            if state.log_mode == LogPanelMode::Hidden && state.focus == FocusArea::Log {
                state.focus = FocusArea::Sidebar;
            }
        }
        Action::MoveUp => move_cursor_or_scroll(state, -1, loader),
        Action::MoveDown => move_cursor_or_scroll(state, 1, loader),
        Action::MoveTop => move_top(state, loader),
        Action::MoveBottom => move_bottom(state, loader),
        Action::HalfPageDown => {
            let step = state.detail_scroll.half_page();
            move_cursor_or_scroll(state, step, loader);
        }
        Action::HalfPageUp => {
            let step = -state.detail_scroll.half_page();
            move_cursor_or_scroll(state, step, loader);
        }
        Action::OpenFilter => {
            let panel = if state.focus == FocusArea::Log {
                Panel::Jobs
            } else {
                state.active_panel
            };
            let existing = state.filters.get(panel).to_string();
            state.overlay = Overlay::Filter {
                panel,
                input: TextInput::new(existing),
            };
        }
        Action::ClearFilter => {
            let panel = if state.focus == FocusArea::Log {
                Panel::Jobs
            } else {
                state.active_panel
            };
            if !state.filters.get(panel).is_empty() {
                state.filters.get_mut(panel).clear();
                state.rebuild_panel(panel);
                state.push_toast(ToastLevel::Info, "Filter cleared");
            }
        }
        Action::OpenHelp => {
            state.overlay = Overlay::Help {
                query: TextInput::default(),
                selected: 0,
            };
        }
        Action::OpenContextMenu => {
            let items = context_menu_items(state);
            let title = format!(" {} ACTIONS ", state.active_panel.title());
            state.overlay = Overlay::ContextMenu {
                title,
                items,
                selected: 0,
            };
        }
        Action::OpenCommandPalette => {
            state.overlay = Overlay::CommandPalette {
                query: TextInput::default(),
                selected: 0,
            };
        }
        Action::OpenProfileSwitcher => {
            let candidates = state.discover_profile_candidates();
            let active = state.profile.as_deref().unwrap_or("default");
            let selected = candidates
                .iter()
                .position(|c| c == active)
                .unwrap_or(0);
            state.overlay = Overlay::ProfileSwitcher {
                candidates,
                input: TextInput::default(),
                selected,
            };
        }
        Action::ReloadAll => {
            state.script_cache.clear();
            state.config_vm.file_cache.clear();
            state.task_info_cache.clear();
            state.task_deps_cache.clear();
            state.task_history_cache.clear();
            state.tools_vm.details_cache.clear();
            state.requested_domains.clear();
            state.push_toast(ToastLevel::Info, "Reloading mise state...");
            trigger_hot_reload(state, loader, true);
        }
        Action::RerunLastTask => {
            if let Some(name) = state.last_task.clone() {
                launch_task_execution(state, runner, name, Vec::new(), true, false);
            } else {
                state.push_toast(ToastLevel::Warning, "No previous task to re-run");
            }
        }

        // ---- Tasks panel ----
        Action::TaskRunForeground => {
            let Some(idx) = state.list_states.tasks.selected() else {
                return;
            };
            match state.task_rows.get(idx).cloned() {
                Some(TaskRow::Group { .. }) => toggle_current_task_group(state),
                Some(TaskRow::Task { task, .. }) => {
                    let info = state
                        .task_info_cache
                        .get(&task.name)
                        .cloned()
                        .unwrap_or(*task);
                    if info
                        .usage_spec
                        .as_ref()
                        .is_some_and(|u| u.has_required_inputs())
                    {
                        open_task_args_form(state, &info, false, false, loader);
                    } else {
                        launch_task_execution(state, runner, info.name, Vec::new(), false, false);
                    }
                }
                None => {}
            }
        }
        Action::TaskRunBackground => {
            let Some(idx) = state.list_states.tasks.selected() else {
                return;
            };
            match state.task_rows.get(idx).cloned() {
                Some(TaskRow::Group { .. }) => toggle_current_task_group(state),
                Some(TaskRow::Task { task, .. }) => {
                    let info = state
                        .task_info_cache
                        .get(&task.name)
                        .cloned()
                        .unwrap_or(*task);
                    if info
                        .usage_spec
                        .as_ref()
                        .is_some_and(|u| u.has_required_inputs())
                    {
                        open_task_args_form(state, &info, true, false, loader);
                    } else {
                        launch_task_execution(state, runner, info.name, Vec::new(), true, false);
                    }
                }
                None => {}
            }
        }
        Action::TaskRunWithArgs => {
            if let Some(task) = state.selected_task().cloned() {
                let info = state
                    .task_info_cache
                    .get(&task.name)
                    .cloned()
                    .unwrap_or(task);
                open_task_args_form(state, &info, true, false, loader);
            }
        }
        Action::TaskRunForce => {
            if let Some(task) = state.selected_task().cloned() {
                launch_task_execution(state, runner, task.name, Vec::new(), true, true);
            }
        }
        Action::TaskWatch => {
            if let Some(task) = state.selected_task().cloned() {
                if !command_exists("watchexec") {
                    state.push_toast(
                        ToastLevel::Warning,
                        "`watchexec` not found on PATH (required by `mise watch`). Install via `mise use -g watchexec`.",
                    );
                } else {
                    state.last_task = Some(task.name.clone());
                    let job_id = spawn_user_mise_job(
                        state,
                        runner,
                        vec!["watch".into(), "-t".into(), task.name.clone()],
                        format!("watch:{}", task.name),
                        Some(task.name.clone()),
                    );
                    state.latest_task_job.insert(task.name.clone(), job_id);
                    state.task_subview = TaskSubView::Output;
                    state.detail_scroll.scroll_top();
                    state.push_toast(
                        ToastLevel::Info,
                        format!("Watching `{}` in background (job #{})", task.name, job_id.0),
                    );
                }
            }
        }
        Action::TaskToggleFold => toggle_current_task_group(state),
        Action::TaskToggleHidden => {
            state.show_hidden_tasks = !state.show_hidden_tasks;
            state.rebuild_task_rows();
            trigger_hot_reload(state, loader, false);
            state.push_toast(
                ToastLevel::Info,
                if state.show_hidden_tasks {
                    "Showing hidden tasks (--hidden)"
                } else {
                    "Hiding hidden tasks"
                },
            );
        }
        Action::TaskToggleMonorepoAll => {
            state.monorepo_all_tasks = !state.monorepo_all_tasks;
            state.rebuild_task_rows();
            trigger_hot_reload(state, loader, false);
            state.push_toast(
                ToastLevel::Info,
                if state.monorepo_all_tasks {
                    "Monorepo mode: showing all subpackage tasks (--all)"
                } else {
                    "Showing local workspace tasks"
                },
            );
        }
        Action::TaskValidate => {
            spawn_user_mise_job(
                state,
                runner,
                vec!["tasks".into(), "validate".into()],
                "tasks:validate".into(),
                None,
            );
            state.push_toast(ToastLevel::Info, "Running `mise tasks validate`...");
        }
        Action::TaskEditSource => {
            if let Some(task) = state.selected_task() {
                if let Some(path) = task.editable_path() {
                    state.pending_fg = Some(ForegroundRequest::EditFile {
                        path: PathBuf::from(path),
                    });
                } else {
                    state.push_toast(ToastLevel::Warning, "Task has no editable source file");
                }
            }
        }
        Action::TaskCopyCommand => {
            if let Some(task) = state.selected_task() {
                let cmd = state.format_mise_command(&["run".into(), task.name.clone()]);
                copy_and_toast(state, &cmd, &format!("Copied `{cmd}`"), false);
            }
        }
        Action::TaskToggleAutoFollow | Action::JobToggleAutoFollow => {
            state.auto_follow = !state.auto_follow;
            if state.auto_follow {
                state.detail_scroll.scroll_bottom();
            }
            state.push_toast(
                ToastLevel::Info,
                if state.auto_follow {
                    "Output auto-follow ON"
                } else {
                    "Output auto-follow OFF"
                },
            );
        }

        // ---- Tools panel ----
        Action::ToolOpenRegistry => {
            if state.registry.is_empty() && state.tools_vm.registry.is_empty() {
                state.loading_domains.insert(Domain::Registry);
                loader.request_domain(state.generation, state.profile.clone(), Domain::Registry, None);
            }
            state.overlay = Overlay::RegistryPicker {
                query: TextInput::default(),
                selected: 0,
                step: RegistryStep::SearchTool,
            };
        }
        Action::ToolInstallAll => {
            let args = vec!["install".into()];
            let preview = state.format_mise_command(&args);
            state.overlay = Overlay::Confirm {
                title: "Install configured mise tools".into(),
                detail: format!("{} missing tool(s) detected.", state.tools_vm.missing_count()),
                command_preview: preview,
                action: ConfirmAction::MiseCommand {
                    args,
                    foreground: false,
                    reload_after: true,
                    label: "mise install".into(),
                },
            };
        }
        Action::ToolUpgradeSelected => {
            if let Some(tool) = state.selected_tool() {
                let name = tool.name.clone();
                let args = vec!["upgrade".into(), name.clone()];
                let preview = state.format_mise_command(&args);
                let detail = tool
                    .outdated
                    .as_ref()
                    .map(|o| {
                        format!(
                            "Upgrades `{name}`: {} -> {}",
                            o.current.as_deref().unwrap_or(&tool.entry.version),
                            o.latest
                        )
                    })
                    .unwrap_or_else(|| format!("Upgrades `{name}` to latest matching version."));
                state.overlay = Overlay::Confirm {
                    title: format!("Upgrade tool `{name}`"),
                    detail,
                    command_preview: preview,
                    action: ConfirmAction::MiseCommand {
                        args,
                        foreground: false,
                        reload_after: true,
                        label: format!("upgrade:{name}"),
                    },
                };
            }
        }
        Action::ToolUpgradeAll => {
            let args = vec!["upgrade".into()];
            let preview = state.format_mise_command(&args);
            state.overlay = Overlay::Confirm {
                title: "Upgrade all outdated tools".into(),
                detail: format!(
                    "{} outdated tool(s) currently reported.",
                    state.tools_vm.outdated_count()
                ),
                command_preview: preview,
                action: ConfirmAction::MiseCommand {
                    args,
                    foreground: false,
                    reload_after: true,
                    label: "mise upgrade".into(),
                },
            };
        }
        Action::ToolUpgradeVulnerable => {
            let args = vec!["upgrade".into(), "--vulnerable".into()];
            let preview = state.format_mise_command(&args);
            state.overlay = Overlay::Confirm {
                title: "Upgrade vulnerable tools".into(),
                detail: format!(
                    "{} CVE finding(s) reported by `mise audit`.",
                    state.tools_vm.vulnerability_count()
                ),
                command_preview: preview,
                action: ConfirmAction::MiseCommand {
                    args,
                    foreground: false,
                    reload_after: true,
                    label: "upgrade --vulnerable".into(),
                },
            };
        }
        Action::ToolRemoveSelected => {
            if let Some(tool) = state.selected_tool() {
                let name = tool.name.clone();
                if tool.should_uninstall_version_only() {
                    let spec = format!("{name}@{}", tool.entry.version);
                    let args = vec!["uninstall".into(), spec.clone()];
                    let preview = state.format_mise_command(&args);
                    state.overlay = Overlay::Confirm {
                        title: format!("Uninstall `{spec}`"),
                        detail: format!(
                            "Uninstalls only version `{}` of `{name}` without editing mise.toml.",
                            tool.entry.version
                        ),
                        command_preview: preview,
                        action: ConfirmAction::MiseCommand {
                            args,
                            foreground: false,
                            reload_after: true,
                            label: format!("uninstall:{spec}"),
                        },
                    };
                } else {
                    let args = vec!["unuse".into(), name.clone()];
                    let preview = state.format_mise_command(&args);
                    let source_hint = tool
                        .entry
                        .source
                        .as_ref()
                        .and_then(|s| s.path.as_deref())
                        .unwrap_or("mise.toml");
                    state.overlay = Overlay::Confirm {
                        title: format!("Unuse `{name}` from config"),
                        detail: format!(
                            "Removes `{name}` from `{source_hint}` and uninstalls it via `mise unuse`."
                        ),
                        command_preview: preview,
                        action: ConfirmAction::MiseCommand {
                            args,
                            foreground: false,
                            reload_after: true,
                            label: format!("unuse:{name}"),
                        },
                    };
                }
            }
        }
        Action::ToolPrune => {
            let args = vec!["prune".into()];
            let preview = state.format_mise_command(&args);
            state.overlay = Overlay::Confirm {
                title: "Prune unused tool versions".into(),
                detail: format!(
                    "{} version(s) currently scheduled for pruning.",
                    state.tools_vm.prunable_count()
                ),
                command_preview: preview,
                action: ConfirmAction::MiseCommand {
                    args,
                    foreground: false,
                    reload_after: true,
                    label: "mise prune".into(),
                },
            };
        }
        Action::ToolLockPreview => {
            state.active_panel = Panel::Tools;
            state.tool_subview = ToolSubView::Lock;
            state.loading_domains.insert(Domain::LockDryRun);
            state.requested_domains.insert(Domain::LockDryRun);
            loader.request_domain(
                state.generation,
                state.profile.clone(),
                Domain::LockDryRun,
                None,
            );
            state.push_toast(ToastLevel::Info, "Running `mise lock --dry-run`...");
        }
        Action::ToolLockApply => {
            let args = vec!["lock".into()];
            let preview = state.format_mise_command(&args);
            state.overlay = Overlay::Confirm {
                title: "Update mise.lock".into(),
                detail: "Locks resolved tool versions, URLs, and checksums in mise.lock.".into(),
                command_preview: preview,
                action: ConfirmAction::MiseCommand {
                    args,
                    foreground: false,
                    reload_after: true,
                    label: "mise lock".into(),
                },
            };
        }
        Action::ToolRunAudit => {
            state.active_panel = Panel::Tools;
            state.tool_subview = ToolSubView::Audit;
            state.loading_domains.insert(Domain::Audit);
            state.requested_domains.insert(Domain::Audit);
            loader.request_domain(state.generation, state.profile.clone(), Domain::Audit, None);
            state.push_toast(ToastLevel::Info, "Running OSV.dev security audit...");
        }
        Action::ToolRefreshOutdated => {
            state.loading_domains.insert(Domain::Outdated);
            state.requested_domains.insert(Domain::Outdated);
            loader.request_domain(
                state.generation,
                state.profile.clone(),
                Domain::Outdated,
                None,
            );
            state.push_toast(ToastLevel::Info, "Checking outdated tools...");
        }
        Action::ToolCycleScope => {
            state.tool_scope = state.tool_scope.next();
            state.rebuild_tools();
            state.push_toast(
                ToastLevel::Info,
                format!("Tools scope: {}", state.tool_scope.label()),
            );
        }
        Action::ToolOpenUrl => {
            let url = state.selected_tool().and_then(|tool| {
                tool.outdated
                    .as_ref()
                    .and_then(|o| o.release_url.clone())
                    .or_else(|| {
                        state
                            .tools_vm
                            .detail_for(&tool.name)
                            .and_then(|d| d.url.clone())
                    })
            });
            if let Some(url) = url {
                match open_url(&url) {
                    Ok(()) => state.push_toast(ToastLevel::Success, format!("Opened {url}")),
                    Err(e) => state.push_toast(ToastLevel::Error, format!("Open URL failed: {e}")),
                }
            } else {
                state.push_toast(ToastLevel::Warning, "No homepage or release URL for this tool");
            }
        }
        Action::ToolEditSource => {
            if let Some(path) = state
                .selected_tool()
                .and_then(|t| t.entry.source.as_ref())
                .and_then(|s| s.path.as_deref())
                .filter(|p| !p.is_empty())
            {
                state.pending_fg = Some(ForegroundRequest::EditFile {
                    path: PathBuf::from(path),
                });
            } else {
                state.push_toast(ToastLevel::Warning, "Tool has no editable config source");
            }
        }

        // ---- Env panel ----
        Action::EnvAddNew => {
            let target_files = build_env_target_files(state);
            state.overlay = Overlay::EnvSetForm {
                key: TextInput::default(),
                value: TextInput::default(),
                target_files,
                target_idx: 0,
                focused_field: 0,
                is_edit: false,
            };
        }
        Action::EnvAddOrEdit => {
            let target_files = build_env_target_files(state);
            if let Some(row) = state.selected_env().cloned() {
                let masked_secret = row.sensitive && !state.is_env_revealed(&row.key);
                let initial_val = if masked_secret {
                    state.push_toast(
                        ToastLevel::Info,
                        format!(
                            "`{}` is masked — enter a new value or press S first to reveal",
                            row.key
                        ),
                    );
                    TextInput::default()
                } else {
                    TextInput::new(row.value)
                };
                state.overlay = Overlay::EnvSetForm {
                    key: TextInput::new(row.key),
                    value: initial_val,
                    target_files,
                    target_idx: 0,
                    focused_field: 1,
                    is_edit: true,
                };
            } else {
                state.overlay = Overlay::EnvSetForm {
                    key: TextInput::default(),
                    value: TextInput::default(),
                    target_files,
                    target_idx: 0,
                    focused_field: 0,
                    is_edit: false,
                };
            }
        }
        Action::EnvUnset => {
            if let Some(row) = state.selected_env() {
                let key = row.key.clone();
                let args = vec!["unset".into(), key.clone()];
                let preview = state.format_mise_command(&args);
                state.overlay = Overlay::Confirm {
                    title: format!("Unset env variable `{key}`"),
                    detail: format!("Removes `{key}` from project mise.toml via `mise unset`."),
                    command_preview: preview,
                    action: ConfirmAction::MiseCommand {
                        args,
                        foreground: false,
                        reload_after: true,
                        label: format!("unset:{key}"),
                    },
                };
            }
        }
        Action::EnvToggleReveal => {
            if let Some(row) = state.selected_env() {
                let key = row.key.clone();
                if state.is_env_revealed(&key) {
                    state.revealed_env_keys.remove(&key);
                    state.push_toast(ToastLevel::Info, format!("Masked `{key}`"));
                } else {
                    state
                        .revealed_env_keys
                        .insert(key.clone(), Instant::now() + SECRET_REVEAL_TTL);
                    state.push_toast(
                        ToastLevel::Warning,
                        format!("Revealed `{key}` (auto-masks in 30s)"),
                    );
                }
            }
        }
        Action::EnvCopyValue => {
            if let Some(row) = state.selected_env().cloned() {
                let msg = format!("Copied value of `{}`", row.key);
                copy_and_toast(state, &row.value, &msg, row.sensitive);
            }
        }
        Action::EnvCopyPair => {
            if let Some(row) = state.selected_env().cloned() {
                let pair = format!("{}={}", row.key, row.value);
                let msg = format!("Copied `{}=...`", row.key);
                copy_and_toast(state, &pair, &msg, row.sensitive);
            }
        }
        Action::EnvCycleFilter => {
            state.env_filter = state.env_filter.next();
            state.rebuild_env();
            state.push_toast(
                ToastLevel::Info,
                format!("Env filter: {}", state.env_filter.label()),
            );
        }

        // ---- Config & Health panel ----
        Action::ConfigEditSelected => {
            if let Some(cfg) = state.selected_config() {
                state.pending_fg = Some(ForegroundRequest::EditFile {
                    path: PathBuf::from(&cfg.entry.path),
                });
            }
        }
        Action::ConfigToggleTrust => {
            if let Some(cfg) = state.selected_config() {
                let path = cfg.entry.path.clone();
                let is_trusted = cfg.trusted == Some(true);
                let (args, verb) = if is_trusted {
                    (vec!["trust".into(), "--untrust".into(), path.clone()], "Untrust")
                } else {
                    (vec!["trust".into(), path.clone()], "Trust")
                };
                let preview = state.format_mise_command(&args);
                state.overlay = Overlay::Confirm {
                    title: format!("{verb} config file"),
                    detail: format!("{verb} `{path}` in mise trust registry."),
                    command_preview: preview,
                    action: ConfirmAction::MiseCommand {
                        args,
                        foreground: false,
                        reload_after: true,
                        label: format!("trust:{verb}"),
                    },
                };
            }
        }
        Action::ConfigFormat => {
            let args = vec!["fmt".into()];
            let preview = state.format_mise_command(&args);
            state.overlay = Overlay::Confirm {
                title: "Format mise.toml (`mise fmt`)".into(),
                detail: "Formats project mise.toml in place.".into(),
                command_preview: preview,
                action: ConfirmAction::MiseCommand {
                    args,
                    foreground: false,
                    reload_after: true,
                    label: "mise fmt".into(),
                },
            };
        }
        Action::ConfigRefreshDoctor => {
            state.active_panel = Panel::Config;
            state.config_subview = ConfigSubView::Doctor;
            state.loading_domains.insert(Domain::Doctor);
            state.requested_domains.insert(Domain::Doctor);
            loader.request_domain(state.generation, state.profile.clone(), Domain::Doctor, None);
            state.push_toast(ToastLevel::Info, "Running `mise doctor --json`...");
        }
        Action::ConfigDoctorReshim => {
            let args = vec!["reshim".into()];
            let preview = state.format_mise_command(&args);
            state.overlay = Overlay::Confirm {
                title: "Rebuild mise shims (`mise reshim`)".into(),
                detail: "Re-generates shims for all installed tools.".into(),
                command_preview: preview,
                action: ConfirmAction::MiseCommand {
                    args,
                    foreground: false,
                    reload_after: true,
                    label: "mise reshim".into(),
                },
            };
        }
        Action::ConfigDoctorProject => {
            spawn_user_mise_job(
                state,
                runner,
                vec!["doctor".into(), "project".into()],
                "doctor:project".into(),
                None,
            );
            state.push_toast(ToastLevel::Info, "Running `mise doctor project`...");
        }
        Action::ConfigOpenGenerate => {
            let items = state.build_generate_items();
            state.overlay = Overlay::GenerateMenu {
                items,
                selected: 0,
                template_name_input: None,
            };
        }
        Action::DaemonToggleSelected => {
            if let Some(daemon) = state.config_vm.daemons.get(state.selected_daemon_idx) {
                let name = daemon.name.clone();
                let running = is_daemon_running(&daemon.status);
                let sub = if running { "stop" } else { "start" };
                let args = vec!["daemons".into(), sub.into(), name.clone()];
                let preview = state.format_mise_command(&args);
                state.overlay = Overlay::Confirm {
                    title: format!("{sub} daemon `{name}`"),
                    detail: format!("Current status: {}", daemon.status),
                    command_preview: preview,
                    action: ConfirmAction::MiseCommand {
                        args,
                        foreground: false,
                        reload_after: true,
                        label: format!("daemon:{sub}:{name}"),
                    },
                };
            } else {
                state.push_toast(ToastLevel::Info, "No [daemons] configured in this project");
            }
        }
        Action::DaemonRestartSelected => {
            if let Some(daemon) = state.config_vm.daemons.get(state.selected_daemon_idx) {
                let name = daemon.name.clone();
                let args = vec!["daemons".into(), "restart".into(), name.clone()];
                let preview = state.format_mise_command(&args);
                state.overlay = Overlay::Confirm {
                    title: format!("Restart daemon `{name}`"),
                    detail: "Restarts managed background daemon.".into(),
                    command_preview: preview,
                    action: ConfirmAction::MiseCommand {
                        args,
                        foreground: false,
                        reload_after: true,
                        label: format!("daemon:restart:{name}"),
                    },
                };
            }
        }
        Action::DepsSync => {
            let args = vec!["deps".into()];
            let preview = state.format_mise_command(&args);
            state.overlay = Overlay::Confirm {
                title: "Sync project dependencies (`mise deps`)".into(),
                detail: "Runs configured [deps] providers to sync project dependencies.".into(),
                command_preview: preview,
                action: ConfirmAction::MiseCommand {
                    args,
                    foreground: false,
                    reload_after: true,
                    label: "mise deps".into(),
                },
            };
        }

        // ---- Jobs & Command Log panel ----
        Action::JobInspect => {
            if let Some(entry) = state.selected_command_log_entry() {
                state.selected_job_id = entry.job_id;
                state.active_panel = Panel::Jobs;
                state.focus = FocusArea::Main;
                state.detail_scroll.scroll_top();
            }
        }
        Action::JobKillSelected => {
            let task_running_id = if state.active_panel == Panel::Tasks
                && state.focus != FocusArea::Log
            {
                state
                    .selected_task_job()
                    .filter(|j| j.status.is_running())
                    .map(|j| j.id)
            } else {
                None
            };
            let target_id = task_running_id
                .or_else(|| state.selected_command_log_entry().and_then(|e| e.job_id))
                .or(state.selected_job_id)
                .or_else(|| {
                    state
                        .jobs
                        .values()
                        .rev()
                        .find(|j| j.status.is_running())
                        .map(|j| j.id)
                });
            if let Some(id) = target_id {
                if runner.kill_job(id) {
                    state.push_toast(
                        ToastLevel::Warning,
                        format!("Sent SIGTERM to job #{}...", id.0),
                    );
                } else {
                    state.push_toast(ToastLevel::Info, format!("Job #{} is not running", id.0));
                }
            } else {
                state.push_toast(ToastLevel::Info, "No running background job to kill");
            }
        }
        Action::JobRerunSelected => {
            let Some(entry) = state.selected_command_log_entry().cloned() else {
                return;
            };
            if let Some(job_id) = entry.job_id
                && let Some(rec) = state.jobs.get(&job_id)
                && let Some(task_name) = rec.task_name.clone()
            {
                launch_task_execution(state, runner, task_name, Vec::new(), true, false);
                return;
            }
            if let Ok(words) = split_shell_words(&entry.command)
                && words.first().map(String::as_str) == Some("mise")
                && words.len() > 1
            {
                let cleaned = strip_leading_dir_and_env(&words[1..]);
                spawn_user_mise_job(
                    state,
                    runner,
                    cleaned,
                    "rerun".into(),
                    None,
                );
            } else {
                state.push_toast(ToastLevel::Warning, "Cannot re-run this log entry");
            }
        }
        Action::JobCopyCommand => {
            if let Some(entry) = state.selected_command_log_entry() {
                let cmd = entry.command.clone();
                copy_and_toast(state, &cmd, &format!("Copied `{cmd}`"), false);
            }
        }
        Action::JobClearFinished => {
            state.command_log.clear_completed();
            state.jobs.retain(|_, j| j.status.is_running());
            state.clamp_panel_selection(Panel::Jobs);
            state.push_toast(ToastLevel::Info, "Cleared completed jobs & log entries");
        }
        Action::ToggleInternalCommands => {
            state.show_internal_commands = !state.show_internal_commands;
            state.clamp_panel_selection(Panel::Jobs);
            state.push_toast(
                ToastLevel::Info,
                if state.show_internal_commands {
                    "Showing internal mise queries in Command Log"
                } else {
                    "Showing user commands only in Command Log"
                },
            );
        }
    }

    state.dirty = true;
}

/// Execute a confirmed `ConfirmAction`.
pub fn execute_confirmed(
    state: &mut AppState,
    action: ConfirmAction,
    _loader: &MiseLoader,
    runner: &BackgroundRunner,
) {
    match action {
        ConfirmAction::QuitWithRunningJobs => {
            runner.kill_all();
            state.should_quit = true;
        }
        ConfirmAction::MiseCommand {
            args,
            foreground,
            reload_after,
            label,
        } => {
            if foreground {
                state.pending_fg = Some(ForegroundRequest::RunMiseCommand {
                    args,
                    wait_for_enter: true,
                    reload_after,
                    label,
                });
            } else {
                spawn_user_mise_job(state, runner, args, label.clone(), None);
                state.push_toast(ToastLevel::Info, format!("Running `{label}` in background..."));
            }
        }
        ConfirmAction::ScaffoldTemplate { template, name } => {
            let label = format!("scaffold:{template}/{name}");
            spawn_user_mise_job(
                state,
                runner,
                vec!["run".into(), "new".into(), template, name],
                label.clone(),
                None,
            );
            state.push_toast(
                ToastLevel::Info,
                format!("Running `{label}` in background..."),
            );
        }
    }
    state.dirty = true;
}

/// Launch a task in either foreground TTY or background streaming job.
pub fn launch_task_execution(
    state: &mut AppState,
    runner: &BackgroundRunner,
    task_name: String,
    extra_args: Vec<String>,
    background: bool,
    force: bool,
) {
    state.last_task = Some(task_name.clone());
    let requires_tty = state
        .task_info_cache
        .get(&task_name)
        .or_else(|| state.tasks.iter().find(|t| t.name == task_name))
        .is_some_and(|t| t.interactive || t.raw);

    if !background || requires_tty {
        state.pending_fg = Some(ForegroundRequest::RunTask {
            name: task_name,
            args: extra_args,
            force,
        });
        state.dirty = true;
        return;
    }

    let mut cmd_args = vec!["run".to_string()];
    if force {
        cmd_args.push("--force".to_string());
    }
    cmd_args.push(task_name.clone());
    if !extra_args.is_empty() {
        cmd_args.push("--".to_string());
        cmd_args.extend(extra_args.iter().cloned());
    }

    let cmd_display = state.format_mise_command(&cmd_args);
    let job_id = runner.spawn_mise_task(
        state.workdir.clone(),
        state.profile.clone(),
        task_name.clone(),
        extra_args,
        force,
    );

    let record = JobRecord {
        id: job_id,
        label: format!("task:{task_name}"),
        task_name: Some(task_name.clone()),
        command_display: cmd_display.clone(),
        pid: None,
        started_at: Instant::now(),
        status: JobStatus::Running,
        lines: vec![JobOutputLine {
            text: format!("$ {cmd_display}"),
            is_stderr: false,
        }],
    };
    state.jobs.insert(job_id, record);
    state.latest_task_job.insert(task_name.clone(), job_id);
    state.selected_job_id = Some(job_id);
    state.command_log.record(
        CommandSource::User,
        cmd_display,
        Some(job_id),
        JobStatus::Running,
        Some(format!("job #{}", job_id.0)),
    );
    state.clamp_panel_selection(Panel::Jobs);
    state.task_subview = TaskSubView::Output;
    state.detail_scroll.scroll_top();
    state.push_toast(
        ToastLevel::Info,
        format!("Started `{task_name}` in background (job #{})", job_id.0),
    );
    state.dirty = true;
}

/// Apply an incoming asynchronous `MiseDataEvent` (dropping stale generations).
pub fn apply_mise_event(state: &mut AppState, ev: MiseDataEvent) {
    if ev.generation != state.generation {
        return;
    }
    state.loading_domains.remove(&ev.domain);
    if let Some(log) = ev.log_entry {
        state.command_log.push(log);
        state.clamp_panel_selection(Panel::Jobs);
    }

    match ev.payload {
        MisePayload::Tasks(tasks) => {
            let had_none = state.tasks.is_empty();
            if had_none && tasks.iter().any(|t| !t.global) {
                state.collapsed_groups.insert("[global]".to_string());
            }
            state.tasks = tasks;
            state.rebuild_task_rows();
            if had_none
                && let Some(pos) = state.task_rows.iter().position(|r| r.as_task().is_some())
            {
                state.list_states.tasks.select(Some(pos));
                state.preload_selected_content();
            }
        }
        MisePayload::TaskInfo { task_name, task } => {
            // If TaskArgsForm is currently open for this task with empty fields, hydrate usage_spec fields
            if let Overlay::TaskArgsForm {
                task_name: ref open_task,
                ref mut fields,
                ..
            } = state.overlay
                && open_task == &task_name
                && fields.is_empty()
                && let Some(spec) = &task.usage_spec
            {
                *fields = build_arg_fields(spec);
            }
            state.task_info_cache.insert(task_name, *task);
        }
        MisePayload::TaskDeps {
            task_name,
            tree_lines,
        } => {
            state.task_deps_cache.insert(task_name, tree_lines);
        }
        MisePayload::TaskHistory { task_name, entries } => {
            state.task_history_cache.insert(task_name, entries);
        }
        MisePayload::Tools(map) => {
            state.tools_vm.raw_tools = map;
            state.rebuild_tools();
        }
        MisePayload::ToolDetail { tool_name, detail } => {
            state.tools_vm.details_cache.insert(tool_name, *detail);
        }
        MisePayload::ToolVersions {
            tool_name,
            mut versions,
        } => {
            // Reverse so newest releases appear at the top after "latest"
            versions.reverse();
            let mut choices = Vec::with_capacity(versions.len() + 1);
            choices.push("latest".to_string());
            choices.extend(versions);
            if let Overlay::RegistryPicker {
                step:
                    RegistryStep::PickVersion {
                        ref tool,
                        versions: ref mut existing,
                        ref mut loading,
                    },
                ..
            } = state.overlay
                && tool == &tool_name
            {
                *existing = choices;
                *loading = false;
            }
        }
        MisePayload::Registry(entries) => {
            state.tools_vm.registry = entries.clone();
            state.registry = entries;
        }
        MisePayload::Outdated(map) => {
            state.tools_vm.outdated = map;
            state.rebuild_tools();
        }
        MisePayload::Audit(report) => {
            let count = report.vulnerabilities.len();
            state.tools_vm.audit = Some(report);
            state.rebuild_tools();
            if count > 0 {
                state.push_toast(
                    ToastLevel::Warning,
                    format!("Audit found {count} vulnerability(ies)"),
                );
            } else {
                state.push_toast(ToastLevel::Success, "Audit clean: 0 vulnerabilities");
            }
        }
        MisePayload::LockDryRun(text) => {
            state.tools_vm.lock_preview = Some(text);
        }
        MisePayload::Env {
            extended,
            redacted_keys,
            paths,
            vars,
        } => {
            state.env_vm.raw_extended = extended;
            state.env_vm.redacted_keys = redacted_keys;
            state.env_vm.paths = paths;
            state.env_vm.vars = vars;
            state.rebuild_env();
        }
        MisePayload::Config { files, trust } => {
            state.config_vm.raw_files = files;
            state.config_vm.trust_entries = trust;
            state.rebuild_config();
        }
        MisePayload::ConfigExplain(items) => {
            state.config_vm.explain_items = items;
            state.rebuild_config();
        }
        MisePayload::Doctor(report) => {
            state.config_vm.doctor = Some(*report);
        }
        MisePayload::Settings(entries) => {
            state.config_vm.settings = entries;
            state.rebuild_config();
        }
        MisePayload::Daemons(daemons) => {
            state.config_vm.daemons = daemons;
            if state.config_vm.daemons.is_empty() {
                state.selected_daemon_idx = 0;
            } else {
                state.selected_daemon_idx = state
                    .selected_daemon_idx
                    .min(state.config_vm.daemons.len() - 1);
            }
        }
        MisePayload::Deps(deps) => {
            state.config_vm.deps = deps;
        }
        MisePayload::Templates(templates) => {
            state.templates = templates;
        }
        MisePayload::Error { domain, message } => {
            // Silently ignore optional experimental probes if unsupported
            if !matches!(domain, Domain::Daemons | Domain::Deps) {
                state.push_toast(ToastLevel::Error, format!("{domain}: {message}"));
            }
        }
    }

    state.dirty = true;
}

/// Apply an incoming `RunnerEvent` to job records, command log, auto-follow, and reload hooks.
pub fn apply_runner_event(state: &mut AppState, ev: RunnerEvent, loader: &MiseLoader) {
    match ev {
        RunnerEvent::Started { job_id, pid } => {
            if let Some(rec) = state.jobs.get_mut(&job_id) {
                rec.pid = pid;
            }
        }
        RunnerEvent::Output {
            job_id,
            line,
            overwrite_last,
        } => {
            if let Some(rec) = state.jobs.get_mut(&job_id) {
                rec.push_line(line, overwrite_last);
                let line_count = rec.lines.len().min(u16::MAX as usize) as u16;
                let viewing_this_job = state.selected_job_id == Some(job_id)
                    || (state.active_panel == Panel::Tasks
                        && state.task_subview == TaskSubView::Output
                        && state
                            .selected_task()
                            .and_then(|t| state.latest_task_job.get(&t.name))
                            == Some(&job_id));
                if viewing_this_job && state.auto_follow {
                    state.detail_scroll.content_len = line_count.saturating_add(3);
                    state.detail_scroll.scroll_bottom();
                }
            }
        }
        RunnerEvent::Finished {
            job_id,
            exit_code,
            duration_ms,
            killed,
        } => {
            let status = if killed {
                JobStatus::Killed { duration_ms }
            } else if exit_code == 0 {
                JobStatus::Success { duration_ms }
            } else {
                JobStatus::Failed {
                    code: exit_code,
                    duration_ms,
                }
            };
            let mut label = format!("job #{}", job_id.0);
            let mut is_mutation = false;
            if let Some(rec) = state.jobs.get_mut(&job_id) {
                rec.status = status.clone();
                label = rec.label.clone();
                is_mutation = rec.task_name.is_none()
                    && !rec.label.starts_with("tasks:validate")
                    && !rec.label.starts_with("doctor:");
            }
            state.command_log.update_job_status(job_id, status.clone());
            let secs = duration_ms as f64 / 1000.0;
            match status {
                JobStatus::Success { .. } => {
                    state.push_toast(ToastLevel::Success, format!("{label} OK ({secs:.2}s)"));
                    if is_mutation {
                        trigger_hot_reload(state, loader, true);
                    }
                }
                JobStatus::Killed { .. } => {
                    state.push_toast(ToastLevel::Warning, format!("{label} killed ({secs:.2}s)"));
                }
                JobStatus::Failed { code, .. } => {
                    state.push_toast(
                        ToastLevel::Error,
                        format!("{label} failed exit {code} ({secs:.2}s)"),
                    );
                }
                JobStatus::Running => {}
            }
        }
    }
    state.dirty = true;
}

/// Jump directly to a named task from the Command Palette: switches to Tasks panel,
/// ensures hidden/monorepo visibility and expands its group, clears conflicting filter,
/// selects the task row, and preloads its script cache.
pub fn jump_to_task(state: &mut AppState, task_name: &str, loader: &MiseLoader) {
    state.active_panel = Panel::Tasks;
    state.focus = FocusArea::Sidebar;

    if let Some(task) = state.tasks.iter().find(|t| t.name == task_name).cloned() {
        if task.hide {
            state.show_hidden_tasks = true;
        }
        if task.name.starts_with("//") {
            state.monorepo_all_tasks = true;
        }
        let group = task_group_key(&task);
        state.collapsed_groups.remove(&group);
    }

    state.rebuild_task_rows();
    let found = state
        .task_rows
        .iter()
        .position(|r| r.as_task().is_some_and(|t| t.name == task_name));

    let pos = if let Some(idx) = found {
        Some(idx)
    } else if !state.filters.tasks.is_empty() {
        state.filters.tasks.clear();
        state.rebuild_task_rows();
        state
            .task_rows
            .iter()
            .position(|r| r.as_task().is_some_and(|t| t.name == task_name))
    } else {
        None
    };

    if let Some(idx) = pos {
        state.list_states.tasks.select(Some(idx));
        state.detail_scroll.scroll_top();
        state.preload_selected_content();
        ensure_active_data_loaded(state, loader);
        state.push_toast(ToastLevel::Info, format!("Jumped to task `{task_name}`"));
    } else {
        state.push_toast(ToastLevel::Warning, format!("Task `{task_name}` not found"));
    }
    state.dirty = true;
}

// -----------------------------------------------------------------------------
// Internal helpers
// -----------------------------------------------------------------------------

fn is_daemon_running(status: &str) -> bool {
    status.eq_ignore_ascii_case("running")
        || status.eq_ignore_ascii_case("active")
        || status.eq_ignore_ascii_case("started")
}

fn move_cursor_or_scroll(state: &mut AppState, delta: isize, loader: &MiseLoader) {
    match state.focus {
        FocusArea::Main => {
            if state.active_panel == Panel::Config
                && state.config_subview == ConfigSubView::Services
                && !state.config_vm.daemons.is_empty()
            {
                let len = state.config_vm.daemons.len();
                let cur = state.selected_daemon_idx as isize;
                state.selected_daemon_idx = (cur + delta).clamp(0, (len - 1) as isize) as usize;
                return;
            }
            state.detail_scroll.scroll_by(delta);
            if delta < 0 {
                state.auto_follow = false;
            }
        }
        FocusArea::Log => {
            move_panel_selection(state, Panel::Jobs, delta);
            state.selected_job_id = state.selected_command_log_entry().and_then(|e| e.job_id);
        }
        FocusArea::Sidebar => {
            let panel = state.active_panel;
            move_panel_selection(state, panel, delta);
            state.detail_scroll.scroll_top();
            state.preload_selected_content();
            ensure_active_data_loaded(state, loader);
        }
    }
}

fn move_top(state: &mut AppState, loader: &MiseLoader) {
    match state.focus {
        FocusArea::Main => {
            if state.active_panel == Panel::Config
                && state.config_subview == ConfigSubView::Services
                && !state.config_vm.daemons.is_empty()
            {
                state.selected_daemon_idx = 0;
                return;
            }
            state.detail_scroll.scroll_top();
            state.auto_follow = false;
        }
        FocusArea::Log => {
            if state.current_list_len(Panel::Jobs) > 0 {
                state.list_states.jobs.select(Some(0));
                state.selected_job_id = state.selected_command_log_entry().and_then(|e| e.job_id);
            }
        }
        FocusArea::Sidebar => {
            let panel = state.active_panel;
            if state.current_list_len(panel) > 0 {
                state.list_states.get_mut(panel).select(Some(0));
                state.detail_scroll.scroll_top();
                state.preload_selected_content();
                ensure_active_data_loaded(state, loader);
            }
        }
    }
}

fn move_bottom(state: &mut AppState, loader: &MiseLoader) {
    match state.focus {
        FocusArea::Main => {
            if state.active_panel == Panel::Config
                && state.config_subview == ConfigSubView::Services
                && !state.config_vm.daemons.is_empty()
            {
                state.selected_daemon_idx = state.config_vm.daemons.len() - 1;
                return;
            }
            state.detail_scroll.scroll_bottom();
        }
        FocusArea::Log => {
            let len = state.current_list_len(Panel::Jobs);
            if len > 0 {
                state.list_states.jobs.select(Some(len - 1));
                state.selected_job_id = state.selected_command_log_entry().and_then(|e| e.job_id);
            }
        }
        FocusArea::Sidebar => {
            let panel = state.active_panel;
            let len = state.current_list_len(panel);
            if len > 0 {
                state.list_states.get_mut(panel).select(Some(len - 1));
                state.detail_scroll.scroll_top();
                state.preload_selected_content();
                ensure_active_data_loaded(state, loader);
            }
        }
    }
}

fn move_panel_selection(state: &mut AppState, panel: Panel, delta: isize) {
    let len = state.current_list_len(panel);
    if len == 0 {
        state.list_states.get_mut(panel).select(None);
        return;
    }
    let cur = state.list_states.get(panel).selected().unwrap_or(0) as isize;
    let next = (cur + delta).clamp(0, (len - 1) as isize) as usize;
    state.list_states.get_mut(panel).select(Some(next));
}

fn toggle_current_task_group(state: &mut AppState) {
    let Some(idx) = state.list_states.tasks.selected() else {
        return;
    };
    let key = match state.task_rows.get(idx) {
        Some(TaskRow::Group { key, .. }) => Some(key.clone()),
        Some(TaskRow::Task { task, .. }) => Some(task_group_key(task)),
        None => None,
    };
    if let Some(k) = key {
        if state.collapsed_groups.contains(&k) {
            state.collapsed_groups.remove(&k);
        } else {
            state.collapsed_groups.insert(k.clone());
        }
        state.rebuild_task_rows();
        if let Some(pos) = state
            .task_rows
            .iter()
            .position(|r| r.group_key() == Some(k.as_str()))
        {
            state.list_states.tasks.select(Some(pos));
        }
    }
}

fn open_task_args_form(
    state: &mut AppState,
    task: &MiseTask,
    background: bool,
    force: bool,
    loader: &MiseLoader,
) {
    let fields = task
        .usage_spec
        .as_ref()
        .map(build_arg_fields)
        .unwrap_or_default();

    if fields.is_empty() && !state.task_info_cache.contains_key(&task.name) {
        state.loading_domains.insert(Domain::TaskInfo);
        loader.request_domain(
            state.generation,
            state.profile.clone(),
            Domain::TaskInfo,
            Some(task.name.clone()),
        );
    }

    state.overlay = Overlay::TaskArgsForm {
        task_name: task.name.clone(),
        background,
        force,
        fields,
        extra_args: TextInput::default(),
        focused_field: 0,
    };
}

fn build_arg_fields(spec: &crate::mise::schema::TaskUsageSpec) -> Vec<ArgField> {
    let mut fields = Vec::new();
    for arg in &spec.cmd.args {
        fields.push(ArgField {
            name: arg.name.clone(),
            flag: None,
            help: arg.help.clone().unwrap_or_else(|| arg.usage.clone()),
            required: arg.required,
            input: TextInput::new(arg.default.clone().unwrap_or_default()),
        });
    }
    for flag in &spec.cmd.flags {
        let flag_tok = flag
            .long
            .first()
            .map(|l| format!("--{l}"))
            .or_else(|| flag.short.first().map(|s| format!("-{s}")))
            .unwrap_or_else(|| format!("--{}", flag.name));
        fields.push(ArgField {
            name: flag.name.clone(),
            flag: Some(flag_tok),
            help: flag.help.clone().unwrap_or_else(|| flag.usage.clone()),
            required: flag.required,
            input: TextInput::new(flag.default.clone().unwrap_or_default()),
        });
    }
    fields
}

fn build_env_target_files(state: &AppState) -> Vec<String> {
    let mut targets = vec!["mise.toml".to_string(), "mise.local.toml".to_string()];
    if let Some(env_name) = state.profile.as_deref().filter(|s| !s.contains(',')) {
        targets.push(format!("mise.{env_name}.toml"));
    }
    targets
}

fn spawn_user_mise_job(
    state: &mut AppState,
    runner: &BackgroundRunner,
    args: Vec<String>,
    label: String,
    task_name: Option<String>,
) -> JobId {
    let cmd_display = state.format_mise_command_safe(&args);
    let job_id = runner.spawn_mise_command(state.workdir.clone(), state.profile.clone(), args);
    let record = JobRecord {
        id: job_id,
        label,
        task_name,
        command_display: cmd_display.clone(),
        pid: None,
        started_at: Instant::now(),
        status: JobStatus::Running,
        lines: vec![JobOutputLine {
            text: format!("$ {cmd_display}"),
            is_stderr: false,
        }],
    };
    state.jobs.insert(job_id, record);
    state.selected_job_id = Some(job_id);
    state.command_log.record(
        CommandSource::User,
        cmd_display,
        Some(job_id),
        JobStatus::Running,
        Some(format!("job #{}", job_id.0)),
    );
    state.clamp_panel_selection(Panel::Jobs);
    job_id
}

fn copy_and_toast(state: &mut AppState, text: &str, label: &str, is_secret: bool) {
    match copy_to_clipboard_detailed(text) {
        Ok(method) => {
            if is_secret {
                state.push_toast(
                    ToastLevel::Warning,
                    format!("{label} via {} (SENSITIVE VALUE COPIED!)", method.label()),
                );
            } else {
                state.push_toast(
                    ToastLevel::Success,
                    format!("{label} via {}", method.label()),
                );
            }
        }
        Err(err) => {
            state.push_toast(ToastLevel::Error, format!("Clipboard error: {err}"));
        }
    }
}

fn open_url(url: &str) -> anyhow::Result<()> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(program)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

fn strip_leading_dir_and_env(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if (args[i] == "-C" || args[i] == "-E") && i + 1 < args.len() {
            i += 2;
            continue;
        }
        out.push(args[i].clone());
        i += 1;
    }
    out
}
