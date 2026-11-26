use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::{Action, ConfigSubView, FocusArea, Panel};
use crate::app::state::{AppState, MenuItem};
use crate::model::fuzzy::fuzzy_match;

/// Single-source-of-truth keybinding specification.
/// Footer hints, searchable `?` Help modal, Context Menu, Command Palette, and key dispatch
/// all derive from this registry.
#[derive(Debug, Clone)]
pub struct KeyBinding {
    pub key_display: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub scope: KeyScope,
    pub action: Action,
    pub show_in_footer: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyScope {
    Global,
    Panel(Panel),
}

impl KeyScope {
    pub fn section_title(self) -> &'static str {
        match self {
            KeyScope::Global => "GLOBAL",
            KeyScope::Panel(Panel::Tasks) => "TASKS [1]",
            KeyScope::Panel(Panel::Tools) => "TOOLS [2]",
            KeyScope::Panel(Panel::Env) => "ENV [3]",
            KeyScope::Panel(Panel::Config) => "CONFIG & HEALTH [4]",
            KeyScope::Panel(Panel::Jobs) => "JOBS & LOG [5]",
        }
    }
}

/// Context used when resolving a key event or querying visible footer hints.
#[derive(Debug, Clone, Copy)]
pub struct KeyContext {
    pub panel: Panel,
    pub focus: FocusArea,
}

impl KeyContext {
    pub fn from_state(state: &AppState) -> Self {
        Self {
            panel: state.active_panel,
            focus: state.focus,
        }
    }

    pub fn effective_panel(self) -> Panel {
        if self.focus == FocusArea::Log {
            Panel::Jobs
        } else {
            self.panel
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PendingChord {
    #[default]
    None,
    G,
    Z,
}

/// Return all registered keybindings (single source of truth).
pub fn all_bindings() -> Vec<KeyBinding> {
    vec![
        // Global navigation & layout
        KeyBinding {
            key_display: "1-5",
            label: "panel",
            description: "Switch active panel (Tasks / Tools / Env / Config / Jobs)",
            scope: KeyScope::Global,
            action: Action::FocusPanel(Panel::Tasks),
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "j/k",
            label: "move",
            description: "Move cursor or scroll detail down / up",
            scope: KeyScope::Global,
            action: Action::MoveDown,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "gg/G",
            label: "top/end",
            description: "Jump to top / bottom (Home / End)",
            scope: KeyScope::Global,
            action: Action::MoveTop,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "Ctrl-d/u",
            label: "page",
            description: "Scroll half-page down / up (PgDn / PgUp)",
            scope: KeyScope::Global,
            action: Action::HalfPageDown,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "h/l",
            label: "pane",
            description: "Focus left sidebar / right detail pane (Tab cycles)",
            scope: KeyScope::Global,
            action: Action::CycleFocusNext,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "[/]",
            label: "subview",
            description: "Switch right pane sub-view (Spec/Output/Deps/History...)",
            scope: KeyScope::Global,
            action: Action::NextSubView,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "+/_",
            label: "zoom",
            description: "Cycle screen zoom mode (Normal / Half / Full)",
            scope: KeyScope::Global,
            action: Action::ZoomIn,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "@",
            label: "log",
            description: "Toggle Command Log & Jobs drawer (Hidden / Compact / Expanded)",
            scope: KeyScope::Global,
            action: Action::ToggleLogPanel,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "R",
            label: "reload",
            description: "Reload mise state asynchronously",
            scope: KeyScope::Global,
            action: Action::ReloadAll,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "r",
            label: "rerun",
            description: "Re-run last executed task (or selected job in Jobs panel)",
            scope: KeyScope::Global,
            action: Action::RerunLastTask,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "x",
            label: "menu",
            description: "Open context action menu for current selection",
            scope: KeyScope::Global,
            action: Action::OpenContextMenu,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "/",
            label: "filter",
            description: "Fuzzy filter current panel (Esc clears)",
            scope: KeyScope::Global,
            action: Action::OpenFilter,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "E",
            label: "env",
            description: "Switch active MISE_ENV (-E) profile",
            scope: KeyScope::Global,
            action: Action::OpenProfileSwitcher,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: ":",
            label: "palette",
            description: "Open global Command Palette (also Ctrl-p)",
            scope: KeyScope::Global,
            action: Action::OpenCommandPalette,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "?",
            label: "help",
            description: "Open searchable keybindings help",
            scope: KeyScope::Global,
            action: Action::OpenHelp,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "q",
            label: "quit",
            description: "Quit lazymise (guards running background jobs)",
            scope: KeyScope::Global,
            action: Action::Quit,
            show_in_footer: true,
        },
        // Panel 1: Tasks
        KeyBinding {
            key_display: "Enter",
            label: "run",
            description: "Run task in foreground TTY (or fold group)",
            scope: KeyScope::Panel(Panel::Tasks),
            action: Action::TaskRunForeground,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "b",
            label: "bg",
            description: "Run task in background with live ANSI output",
            scope: KeyScope::Panel(Panel::Tasks),
            action: Action::TaskRunBackground,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "A",
            label: "args",
            description: "Run task with arguments / #USAGE flags form",
            scope: KeyScope::Panel(Panel::Tasks),
            action: Action::TaskRunWithArgs,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "f",
            label: "force",
            description: "Run task with --force (bypass sources/outputs cache)",
            scope: KeyScope::Panel(Panel::Tasks),
            action: Action::TaskRunForce,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "w",
            label: "watch",
            description: "Watch task via `mise watch -t <task>`",
            scope: KeyScope::Panel(Panel::Tasks),
            action: Action::TaskWatch,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "Space",
            label: "fold",
            description: "Fold / unfold task namespace group (also za)",
            scope: KeyScope::Panel(Panel::Tasks),
            action: Action::TaskToggleFold,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "H",
            label: "hidden",
            description: "Toggle hidden tasks (--hidden)",
            scope: KeyScope::Panel(Panel::Tasks),
            action: Action::TaskToggleHidden,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "M",
            label: "monorepo",
            description: "Toggle monorepo all subpackage tasks (--all)",
            scope: KeyScope::Panel(Panel::Tasks),
            action: Action::TaskToggleMonorepoAll,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "v",
            label: "validate",
            description: "Validate tasks (`mise tasks validate`)",
            scope: KeyScope::Panel(Panel::Tasks),
            action: Action::TaskValidate,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "e",
            label: "edit",
            description: "Edit task source file in $EDITOR",
            scope: KeyScope::Panel(Panel::Tasks),
            action: Action::TaskEditSource,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "y",
            label: "copy",
            description: "Copy `mise run <task>` command to clipboard",
            scope: KeyScope::Panel(Panel::Tasks),
            action: Action::TaskCopyCommand,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "F",
            label: "follow",
            description: "Toggle auto-follow tailing in Output view",
            scope: KeyScope::Panel(Panel::Tasks),
            action: Action::TaskToggleAutoFollow,
            show_in_footer: false,
        },
        // Panel 2: Tools
        KeyBinding {
            key_display: "a",
            label: "add",
            description: "Search mise registry and add tool (`mise use`)",
            scope: KeyScope::Panel(Panel::Tools),
            action: Action::ToolOpenRegistry,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "i",
            label: "install",
            description: "Install configured tools (`mise install`)",
            scope: KeyScope::Panel(Panel::Tools),
            action: Action::ToolInstallAll,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "u",
            label: "upgrade",
            description: "Upgrade selected tool (`mise upgrade <tool>`)",
            scope: KeyScope::Panel(Panel::Tools),
            action: Action::ToolUpgradeSelected,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "U",
            label: "upg-all",
            description: "Upgrade all outdated tools (`mise upgrade`)",
            scope: KeyScope::Panel(Panel::Tools),
            action: Action::ToolUpgradeAll,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "V",
            label: "fix-cve",
            description: "Upgrade vulnerable tools (`mise upgrade --vulnerable`)",
            scope: KeyScope::Panel(Panel::Tools),
            action: Action::ToolUpgradeVulnerable,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "d",
            label: "remove",
            description: "Remove tool from config (`mise unuse <tool>`)",
            scope: KeyScope::Panel(Panel::Tools),
            action: Action::ToolRemoveSelected,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "p",
            label: "prune",
            description: "Prune unused tool versions (`mise prune`)",
            scope: KeyScope::Panel(Panel::Tools),
            action: Action::ToolPrune,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "L",
            label: "lock",
            description: "Preview `mise lock --dry-run` / update mise.lock",
            scope: KeyScope::Panel(Panel::Tools),
            action: Action::ToolLockPreview,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "A",
            label: "audit",
            description: "Run OSV.dev security audit (`mise audit --json`)",
            scope: KeyScope::Panel(Panel::Tools),
            action: Action::ToolRunAudit,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "O",
            label: "outdated",
            description: "Refresh outdated tools check (`mise outdated --json`)",
            scope: KeyScope::Panel(Panel::Tools),
            action: Action::ToolRefreshOutdated,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "f",
            label: "scope",
            description: "Cycle tool scope (All / Active / Project / Missing / Prunable)",
            scope: KeyScope::Panel(Panel::Tools),
            action: Action::ToolCycleScope,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "o",
            label: "open",
            description: "Open tool homepage or release URL in browser",
            scope: KeyScope::Panel(Panel::Tools),
            action: Action::ToolOpenUrl,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "e",
            label: "edit",
            description: "Edit source mise.toml for selected tool",
            scope: KeyScope::Panel(Panel::Tools),
            action: Action::ToolEditSource,
            show_in_footer: false,
        },
        // Panel 3: Env
        KeyBinding {
            key_display: "a",
            label: "add",
            description: "Add new environment variable (`mise set`)",
            scope: KeyScope::Panel(Panel::Env),
            action: Action::EnvAddNew,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "e",
            label: "edit",
            description: "Edit selected environment variable (`mise set`)",
            scope: KeyScope::Panel(Panel::Env),
            action: Action::EnvAddOrEdit,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "d",
            label: "unset",
            description: "Unset selected environment variable (`mise unset`)",
            scope: KeyScope::Panel(Panel::Env),
            action: Action::EnvUnset,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "S",
            label: "reveal",
            description: "Toggle reveal secret value (auto-remasks in 30s)",
            scope: KeyScope::Panel(Panel::Env),
            action: Action::EnvToggleReveal,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "y",
            label: "copy",
            description: "Copy env variable value to clipboard",
            scope: KeyScope::Panel(Panel::Env),
            action: Action::EnvCopyValue,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "Y",
            label: "copy-kv",
            description: "Copy `KEY=VALUE` pair to clipboard",
            scope: KeyScope::Panel(Panel::Env),
            action: Action::EnvCopyPair,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "f",
            label: "filter",
            description: "Cycle env source filter (All / Project / Local / Global / Tool / Redacted)",
            scope: KeyScope::Panel(Panel::Env),
            action: Action::EnvCycleFilter,
            show_in_footer: true,
        },
        // Panel 4: Config & Health
        KeyBinding {
            key_display: "Enter/e",
            label: "edit",
            description: "Edit selected config file in $EDITOR",
            scope: KeyScope::Panel(Panel::Config),
            action: Action::ConfigEditSelected,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "t",
            label: "trust",
            description: "Trust / untrust selected config (`mise trust`)",
            scope: KeyScope::Panel(Panel::Config),
            action: Action::ConfigToggleTrust,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "F",
            label: "fmt",
            description: "Format mise.toml (`mise fmt`)",
            scope: KeyScope::Panel(Panel::Config),
            action: Action::ConfigFormat,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "D",
            label: "doctor",
            description: "Refresh `mise doctor --json` diagnostics",
            scope: KeyScope::Panel(Panel::Config),
            action: Action::ConfigRefreshDoctor,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "g",
            label: "gen",
            description: "Open Generate & Scaffold menu (`mise generate` + templates)",
            scope: KeyScope::Panel(Panel::Config),
            action: Action::ConfigOpenGenerate,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "s",
            label: "daemon",
            description: "Start / stop selected daemon (in Services sub-view)",
            scope: KeyScope::Panel(Panel::Config),
            action: Action::DaemonToggleSelected,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "S",
            label: "deps",
            description: "Sync project dependencies (`mise deps`)",
            scope: KeyScope::Panel(Panel::Config),
            action: Action::DepsSync,
            show_in_footer: false,
        },
        // Panel 5: Jobs & Command Log
        KeyBinding {
            key_display: "Enter",
            label: "inspect",
            description: "Inspect selected job output in main pane",
            scope: KeyScope::Panel(Panel::Jobs),
            action: Action::JobInspect,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "K",
            label: "kill",
            description: "Kill running background job process group",
            scope: KeyScope::Panel(Panel::Jobs),
            action: Action::JobKillSelected,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "y",
            label: "copy",
            description: "Copy exact CLI command to clipboard",
            scope: KeyScope::Panel(Panel::Jobs),
            action: Action::JobCopyCommand,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "c",
            label: "clear",
            description: "Clear completed entries from Command Log",
            scope: KeyScope::Panel(Panel::Jobs),
            action: Action::JobClearFinished,
            show_in_footer: false,
        },
        KeyBinding {
            key_display: "i",
            label: "internal",
            description: "Toggle showing internal read/refresh mise commands",
            scope: KeyScope::Panel(Panel::Jobs),
            action: Action::ToggleInternalCommands,
            show_in_footer: true,
        },
        KeyBinding {
            key_display: "F",
            label: "follow",
            description: "Toggle auto-follow tailing in job output",
            scope: KeyScope::Panel(Panel::Jobs),
            action: Action::JobToggleAutoFollow,
            show_in_footer: false,
        },
    ]
}

/// Footer key hints dynamically derived from `all_bindings()` for the active panel/focus.
pub fn footer_hints(ctx: KeyContext) -> Vec<(&'static str, &'static str)> {
    let eff = ctx.effective_panel();
    let mut panel_hints = Vec::new();
    let mut global_hints = Vec::new();
    for b in all_bindings() {
        if !b.show_in_footer {
            continue;
        }
        if b.scope == KeyScope::Panel(eff) {
            panel_hints.push((b.key_display, b.label));
        } else if b.scope == KeyScope::Global {
            global_hints.push((b.key_display, b.label));
        }
    }
    panel_hints.extend(global_hints);
    panel_hints
}

pub fn footer_hints_for_state(state: &AppState) -> Vec<(&'static str, &'static str)> {
    footer_hints(KeyContext::from_state(state))
}

/// Return keybindings ordered with the current panel first, then global, then other panels,
/// optionally filtered by fuzzy query (used by searchable `?` Help modal).
pub fn help_entries(state: &AppState, query: &str) -> Vec<KeyBinding> {
    let eff = KeyContext::from_state(state).effective_panel();
    let mut all = all_bindings();
    all.sort_by_key(|b| match b.scope {
        KeyScope::Panel(p) if p == eff => 0u8,
        KeyScope::Global => 1u8,
        KeyScope::Panel(_) => 2u8,
    });
    let q = query.trim();
    if q.is_empty() {
        return all;
    }
    let mut scored: Vec<(KeyBinding, i32)> = all
        .into_iter()
        .filter_map(|b| {
            let hay = format!(
                "{} {} {} {}",
                b.key_display,
                b.label,
                b.description,
                b.scope.section_title()
            );
            fuzzy_match(q, &hay).map(|m| (b, m.score))
        })
        .collect();
    scored.sort_by_key(|a| std::cmp::Reverse(a.1));
    scored.into_iter().map(|(b, _)| b).collect()
}

/// Contextual action menu items for the `x` modal based on the currently focused panel & selection.
pub fn context_menu_items(state: &AppState) -> Vec<MenuItem> {
    let eff = KeyContext::from_state(state).effective_panel();
    match eff {
        Panel::Tasks => {
            let task_name = state
                .selected_task()
                .map(|t| t.name.clone())
                .unwrap_or_else(|| "<task>".to_string());
            vec![
                MenuItem {
                    key_hint: "Enter".into(),
                    label: "Run in foreground TTY".into(),
                    description: "Suspend TUI and run interactively".into(),
                    action: Action::TaskRunForeground,
                    command_preview: Some(
                        state.format_mise_command(&["run".into(), task_name.clone()]),
                    ),
                },
                MenuItem {
                    key_hint: "b".into(),
                    label: "Run in background".into(),
                    description: "Stream live ANSI output in Output sub-view".into(),
                    action: Action::TaskRunBackground,
                    command_preview: Some(
                        state.format_mise_command(&["run".into(), task_name.clone()]),
                    ),
                },
                MenuItem {
                    key_hint: "A".into(),
                    label: "Run with arguments / flags".into(),
                    description: "Open interactive usage_spec / args form".into(),
                    action: Action::TaskRunWithArgs,
                    command_preview: Some(state.format_mise_command(&[
                        "run".into(),
                        task_name.clone(),
                        "--".into(),
                        "<args>".into(),
                    ])),
                },
                MenuItem {
                    key_hint: "f".into(),
                    label: "Run with --force".into(),
                    description: "Ignore sources/outputs freshness cache".into(),
                    action: Action::TaskRunForce,
                    command_preview: Some(state.format_mise_command(&[
                        "run".into(),
                        "--force".into(),
                        task_name.clone(),
                    ])),
                },
                MenuItem {
                    key_hint: "w".into(),
                    label: "Watch task".into(),
                    description: "Re-run on file changes (requires watchexec)".into(),
                    action: Action::TaskWatch,
                    command_preview: Some(state.format_mise_command(&[
                        "watch".into(),
                        "-t".into(),
                        task_name.clone(),
                    ])),
                },
                MenuItem {
                    key_hint: "v".into(),
                    label: "Validate tasks".into(),
                    description: "Check dependency cycles & task definitions".into(),
                    action: Action::TaskValidate,
                    command_preview: Some(
                        state.format_mise_command(&["tasks".into(), "validate".into()]),
                    ),
                },
                MenuItem {
                    key_hint: "e".into(),
                    label: "Edit task source".into(),
                    description: "Open task file or mise.toml in $EDITOR".into(),
                    action: Action::TaskEditSource,
                    command_preview: state
                        .selected_task()
                        .and_then(|t| t.editable_path().map(|p| format!("$EDITOR {p}"))),
                },
                MenuItem {
                    key_hint: "y".into(),
                    label: "Copy run command".into(),
                    description: "Copy exact `mise run` CLI command".into(),
                    action: Action::TaskCopyCommand,
                    command_preview: Some(
                        state.format_mise_command(&["run".into(), task_name]),
                    ),
                },
                MenuItem {
                    key_hint: "H".into(),
                    label: if state.show_hidden_tasks {
                        "Hide hidden tasks".into()
                    } else {
                        "Show hidden tasks".into()
                    },
                    description: "Toggle `hide = true` tasks visibility".into(),
                    action: Action::TaskToggleHidden,
                    command_preview: None,
                },
                MenuItem {
                    key_hint: "M".into(),
                    label: if state.monorepo_all_tasks {
                        "Show local tasks only".into()
                    } else {
                        "Show all monorepo tasks (--all)".into()
                    },
                    description: "Toggle cross-package //pkg:task discovery".into(),
                    action: Action::TaskToggleMonorepoAll,
                    command_preview: None,
                },
            ]
        }
        Panel::Tools => {
            let tool_name = state
                .selected_tool()
                .map(|t| t.name.clone())
                .unwrap_or_else(|| "<tool>".to_string());
            vec![
                MenuItem {
                    key_hint: "a".into(),
                    label: "Add tool from Registry".into(),
                    description: "Fuzzy search 1,079+ mise registry tools".into(),
                    action: Action::ToolOpenRegistry,
                    command_preview: Some("mise use <tool>@<version>".into()),
                },
                MenuItem {
                    key_hint: "i".into(),
                    label: "Install all configured tools".into(),
                    description: "Install missing versions from mise.toml".into(),
                    action: Action::ToolInstallAll,
                    command_preview: Some(state.format_mise_command(&["install".into()])),
                },
                MenuItem {
                    key_hint: "u".into(),
                    label: format!("Upgrade `{tool_name}`"),
                    description: "Upgrade selected tool to latest matching version".into(),
                    action: Action::ToolUpgradeSelected,
                    command_preview: Some(
                        state.format_mise_command(&["upgrade".into(), tool_name.clone()]),
                    ),
                },
                MenuItem {
                    key_hint: "U".into(),
                    label: "Upgrade all outdated tools".into(),
                    description: "Upgrade all tools in current config".into(),
                    action: Action::ToolUpgradeAll,
                    command_preview: Some(state.format_mise_command(&["upgrade".into()])),
                },
                MenuItem {
                    key_hint: "V".into(),
                    label: "Upgrade vulnerable tools (CVE)".into(),
                    description: "Fix reported OSV.dev vulnerabilities".into(),
                    action: Action::ToolUpgradeVulnerable,
                    command_preview: Some(
                        state.format_mise_command(&["upgrade".into(), "--vulnerable".into()]),
                    ),
                },
                MenuItem {
                    key_hint: "d".into(),
                    label: format!("Remove `{tool_name}` (`unuse`)"),
                    description: "Remove tool from project mise.toml".into(),
                    action: Action::ToolRemoveSelected,
                    command_preview: Some(
                        state.format_mise_command(&["unuse".into(), tool_name]),
                    ),
                },
                MenuItem {
                    key_hint: "p".into(),
                    label: "Prune unused versions".into(),
                    description: "Remove unreferenced tool installations".into(),
                    action: Action::ToolPrune,
                    command_preview: Some(state.format_mise_command(&["prune".into()])),
                },
                MenuItem {
                    key_hint: "L".into(),
                    label: "Lockfile preview (`mise lock --dry-run`)".into(),
                    description: "Inspect lockfile updates before writing".into(),
                    action: Action::ToolLockPreview,
                    command_preview: Some(
                        state.format_mise_command(&["lock".into(), "--dry-run".into()]),
                    ),
                },
                MenuItem {
                    key_hint: " ".into(),
                    label: "Apply lockfile (`mise lock`)".into(),
                    description: "Update mise.lock checksums & URLs".into(),
                    action: Action::ToolLockApply,
                    command_preview: Some(state.format_mise_command(&["lock".into()])),
                },
                MenuItem {
                    key_hint: "A".into(),
                    label: "Run security audit".into(),
                    description: "Query OSV.dev for known CVEs".into(),
                    action: Action::ToolRunAudit,
                    command_preview: Some(
                        state.format_mise_command(&["audit".into(), "--json".into()]),
                    ),
                },
                MenuItem {
                    key_hint: "O".into(),
                    label: "Check outdated tools".into(),
                    description: "Refresh available upstream versions".into(),
                    action: Action::ToolRefreshOutdated,
                    command_preview: Some(
                        state.format_mise_command(&["outdated".into(), "--json".into()]),
                    ),
                },
                MenuItem {
                    key_hint: "o".into(),
                    label: "Open homepage / release URL".into(),
                    description: "Launch URL in default browser".into(),
                    action: Action::ToolOpenUrl,
                    command_preview: None,
                },
                MenuItem {
                    key_hint: "e".into(),
                    label: "Edit source config".into(),
                    description: "Open config defining this tool in $EDITOR".into(),
                    action: Action::ToolEditSource,
                    command_preview: None,
                },
            ]
        }
        Panel::Env => {
            let key = state
                .selected_env()
                .map(|r| r.key.clone())
                .unwrap_or_else(|| "<KEY>".to_string());
            vec![
                MenuItem {
                    key_hint: "a".into(),
                    label: "Add env variable".into(),
                    description: "Set a new key=value in mise config".into(),
                    action: Action::EnvAddNew,
                    command_preview: Some(state.format_mise_command(&["set".into(), "KEY=VAL".into()])),
                },
                MenuItem {
                    key_hint: "e".into(),
                    label: format!("Edit `{key}`"),
                    description: "Update selected variable via `mise set`".into(),
                    action: Action::EnvAddOrEdit,
                    command_preview: Some(
                        state.format_mise_command(&["set".into(), format!("{key}=<value>")]),
                    ),
                },
                MenuItem {
                    key_hint: "d".into(),
                    label: format!("Unset `{key}`"),
                    description: "Remove variable from mise config".into(),
                    action: Action::EnvUnset,
                    command_preview: Some(state.format_mise_command(&["unset".into(), key])),
                },
                MenuItem {
                    key_hint: "S".into(),
                    label: "Toggle reveal secret".into(),
                    description: "Show masked value (auto-hides after 30s)".into(),
                    action: Action::EnvToggleReveal,
                    command_preview: None,
                },
                MenuItem {
                    key_hint: "y".into(),
                    label: "Copy value".into(),
                    description: "Copy value to clipboard (warns on secrets)".into(),
                    action: Action::EnvCopyValue,
                    command_preview: None,
                },
                MenuItem {
                    key_hint: "Y".into(),
                    label: "Copy KEY=VALUE".into(),
                    description: "Copy assignment pair to clipboard".into(),
                    action: Action::EnvCopyPair,
                    command_preview: None,
                },
                MenuItem {
                    key_hint: "E".into(),
                    label: "Switch MISE_ENV profile".into(),
                    description: "Switch active environment profile".into(),
                    action: Action::OpenProfileSwitcher,
                    command_preview: None,
                },
            ]
        }
        Panel::Config => {
            let cfg_path = state
                .selected_config()
                .map(|c| c.entry.path.clone())
                .unwrap_or_else(|| "mise.toml".to_string());
            let mut items = vec![
                MenuItem {
                    key_hint: "e".into(),
                    label: "Edit config file".into(),
                    description: "Open selected config in $EDITOR".into(),
                    action: Action::ConfigEditSelected,
                    command_preview: Some(format!("$EDITOR {cfg_path}")),
                },
                MenuItem {
                    key_hint: "t".into(),
                    label: "Toggle trust".into(),
                    description: "Trust or untrust selected config".into(),
                    action: Action::ConfigToggleTrust,
                    command_preview: Some(
                        state.format_mise_command(&["trust".into(), cfg_path]),
                    ),
                },
                MenuItem {
                    key_hint: "F".into(),
                    label: "Format config (`mise fmt`)".into(),
                    description: "Format mise.toml according to mise rules".into(),
                    action: Action::ConfigFormat,
                    command_preview: Some(state.format_mise_command(&["fmt".into()])),
                },
                MenuItem {
                    key_hint: "D".into(),
                    label: "Refresh Doctor diagnostics".into(),
                    description: "Re-run `mise doctor --json`".into(),
                    action: Action::ConfigRefreshDoctor,
                    command_preview: Some(
                        state.format_mise_command(&["doctor".into(), "--json".into()]),
                    ),
                },
                MenuItem {
                    key_hint: " ".into(),
                    label: "Fix shims (`mise reshim`)".into(),
                    description: "Rebuild mise shims directory".into(),
                    action: Action::ConfigDoctorReshim,
                    command_preview: Some(state.format_mise_command(&["reshim".into()])),
                },
                MenuItem {
                    key_hint: " ".into(),
                    label: "Run project checks (`mise doctor project`)".into(),
                    description: "Execute custom [doctor.checks]".into(),
                    action: Action::ConfigDoctorProject,
                    command_preview: Some(
                        state.format_mise_command(&["doctor".into(), "project".into()]),
                    ),
                },
                MenuItem {
                    key_hint: "g".into(),
                    label: "Generate & Scaffold".into(),
                    description: "Official `mise generate` + workspace templates".into(),
                    action: Action::ConfigOpenGenerate,
                    command_preview: Some("mise generate ...".into()),
                },
            ];
            if state.config_subview == ConfigSubView::Services || state.config_vm.has_services() {
                items.push(MenuItem {
                    key_hint: "s".into(),
                    label: "Start / stop daemon".into(),
                    description: "Toggle selected daemon in [daemons]".into(),
                    action: Action::DaemonToggleSelected,
                    command_preview: Some("mise daemons start|stop <name>".into()),
                });
                items.push(MenuItem {
                    key_hint: " ".into(),
                    label: "Restart daemon".into(),
                    description: "Restart selected daemon".into(),
                    action: Action::DaemonRestartSelected,
                    command_preview: Some("mise daemons restart <name>".into()),
                });
                items.push(MenuItem {
                    key_hint: "S".into(),
                    label: "Sync project deps (`mise deps`)".into(),
                    description: "Synchronize configured project dependencies".into(),
                    action: Action::DepsSync,
                    command_preview: Some(state.format_mise_command(&["deps".into()])),
                });
            }
            items
        }
        Panel::Jobs => vec![
            MenuItem {
                key_hint: "Enter".into(),
                label: "Inspect job output".into(),
                description: "View full ANSI output in main pane".into(),
                action: Action::JobInspect,
                command_preview: None,
            },
            MenuItem {
                key_hint: "K".into(),
                label: "Kill running job".into(),
                description: "Send SIGTERM (then SIGKILL) to job process group".into(),
                action: Action::JobKillSelected,
                command_preview: None,
            },
            MenuItem {
                key_hint: "r".into(),
                label: "Re-run selected command".into(),
                description: "Spawn selected command again in background".into(),
                action: Action::JobRerunSelected,
                command_preview: state
                    .selected_command_log_entry()
                    .map(|e| e.command.clone()),
            },
            MenuItem {
                key_hint: "y".into(),
                label: "Copy CLI command".into(),
                description: "Copy exact command string to clipboard".into(),
                action: Action::JobCopyCommand,
                command_preview: None,
            },
            MenuItem {
                key_hint: "i".into(),
                label: if state.show_internal_commands {
                    "Hide internal refresh queries".into()
                } else {
                    "Show internal refresh queries".into()
                },
                description: "Toggle visibility of read-only mise --json calls".into(),
                action: Action::ToggleInternalCommands,
                command_preview: None,
            },
            MenuItem {
                key_hint: "c".into(),
                label: "Clear finished entries".into(),
                description: "Keep only currently running jobs".into(),
                action: Action::JobClearFinished,
                command_preview: None,
            },
        ],
    }
}

/// Global Command Palette items (all actions across panels + quick task jumps), fuzzy-filtered.
pub fn palette_items(state: &AppState, query: &str) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = vec![
        MenuItem {
            key_hint: "1".into(),
            label: "Panel: Tasks".into(),
            description: "Focus Tasks tree & DAG inspector".into(),
            action: Action::FocusPanel(Panel::Tasks),
            command_preview: None,
        },
        MenuItem {
            key_hint: "2".into(),
            label: "Panel: Tools".into(),
            description: "Focus Tools manager, Outdated, Audit & Lock".into(),
            action: Action::FocusPanel(Panel::Tools),
            command_preview: None,
        },
        MenuItem {
            key_hint: "3".into(),
            label: "Panel: Env".into(),
            description: "Focus Environment variables, PATH & Vars".into(),
            action: Action::FocusPanel(Panel::Env),
            command_preview: None,
        },
        MenuItem {
            key_hint: "4".into(),
            label: "Panel: Config & Health".into(),
            description: "Focus Config hierarchy, Doctor, Settings & Services".into(),
            action: Action::FocusPanel(Panel::Config),
            command_preview: None,
        },
        MenuItem {
            key_hint: "5".into(),
            label: "Panel: Jobs & Command Log".into(),
            description: "Focus live background jobs & command audit log".into(),
            action: Action::FocusPanel(Panel::Jobs),
            command_preview: None,
        },
        MenuItem {
            key_hint: "E".into(),
            label: "Switch MISE_ENV profile".into(),
            description: "Select or enter active -E environment profile".into(),
            action: Action::OpenProfileSwitcher,
            command_preview: None,
        },
        MenuItem {
            key_hint: "a".into(),
            label: "Tools: Search Registry & Install".into(),
            description: "Fuzzy search official mise registry and run `mise use`".into(),
            action: Action::ToolOpenRegistry,
            command_preview: Some("mise use <tool>@<version>".into()),
        },
        MenuItem {
            key_hint: "A".into(),
            label: "Tools: Security Audit (OSV.dev)".into(),
            description: "Scan installed tools for known CVEs".into(),
            action: Action::ToolRunAudit,
            command_preview: Some("mise audit --json".into()),
        },
        MenuItem {
            key_hint: "O".into(),
            label: "Tools: Refresh Outdated".into(),
            description: "Check upstream versions for installed tools".into(),
            action: Action::ToolRefreshOutdated,
            command_preview: Some("mise outdated --json".into()),
        },
        MenuItem {
            key_hint: "L".into(),
            label: "Tools: Lockfile Dry-Run Preview".into(),
            description: "Preview `mise lock --dry-run` changes".into(),
            action: Action::ToolLockPreview,
            command_preview: Some("mise lock --dry-run".into()),
        },
        MenuItem {
            key_hint: "D".into(),
            label: "Config: Run Mise Doctor".into(),
            description: "Refresh `mise doctor --json` diagnostics".into(),
            action: Action::ConfigRefreshDoctor,
            command_preview: Some("mise doctor --json".into()),
        },
        MenuItem {
            key_hint: "F".into(),
            label: "Config: Format mise.toml".into(),
            description: "Run `mise fmt` on project configuration".into(),
            action: Action::ConfigFormat,
            command_preview: Some("mise fmt".into()),
        },
        MenuItem {
            key_hint: "g".into(),
            label: "Generate & Scaffold".into(),
            description: "Run `mise generate` or scaffold from template".into(),
            action: Action::ConfigOpenGenerate,
            command_preview: Some("mise generate ...".into()),
        },
        MenuItem {
            key_hint: "v".into(),
            label: "Tasks: Validate".into(),
            description: "Run `mise tasks validate`".into(),
            action: Action::TaskValidate,
            command_preview: Some("mise tasks validate".into()),
        },
        MenuItem {
            key_hint: "R".into(),
            label: "Reload all mise state".into(),
            description: "Async refresh Tasks, Tools, Env, Config".into(),
            action: Action::ReloadAll,
            command_preview: None,
        },
        MenuItem {
            key_hint: "@".into(),
            label: "Toggle Command Log drawer".into(),
            description: "Cycle Hidden / Compact / Expanded".into(),
            action: Action::ToggleLogPanel,
            command_preview: None,
        },
        MenuItem {
            key_hint: "+".into(),
            label: "Zoom screen in".into(),
            description: "Normal -> Half -> Full screen".into(),
            action: Action::ZoomIn,
            command_preview: None,
        },
        MenuItem {
            key_hint: "_".into(),
            label: "Zoom screen out".into(),
            description: "Full -> Half -> Normal screen".into(),
            action: Action::ZoomOut,
            command_preview: None,
        },
        MenuItem {
            key_hint: "i".into(),
            label: "Jobs: Toggle internal commands".into(),
            description: "Show/hide read-only mise queries in Command Log".into(),
            action: Action::ToggleInternalCommands,
            command_preview: None,
        },
    ];

    // Append current panel's contextual items that aren't duplicated by action
    for item in context_menu_items(state) {
        if !items.iter().any(|existing| existing.action == item.action) {
            items.push(item);
        }
    }

    // Append quick-jump entries for all loaded mise tasks
    for task in &state.tasks {
        items.push(MenuItem {
            key_hint: "Task".into(),
            label: format!("Task: {}", task.name),
            description: if task.description.is_empty() {
                format!("Jump to task `{}`", task.name)
            } else {
                task.description.clone()
            },
            action: Action::FocusPanel(Panel::Tasks),
            command_preview: Some(state.format_mise_command(&["run".into(), task.name.clone()])),
        });
    }

    let q = query.trim();
    if q.is_empty() {
        return items;
    }
    let mut scored: Vec<(MenuItem, i32)> = items
        .into_iter()
        .filter_map(|item| {
            let mut best: Option<i32> = None;
            if let Some(m) = fuzzy_match(q, &item.label) {
                best = Some(m.score + 120);
            }
            if let Some(cmd) = item.command_preview.as_deref()
                && let Some(m) = fuzzy_match(q, cmd)
            {
                let s = m.score + 60;
                if best.is_none_or(|cur| s > cur) {
                    best = Some(s);
                }
            }
            if let Some(m) = fuzzy_match(q, &item.description)
                && best.is_none_or(|cur| m.score > cur)
            {
                best = Some(m.score);
            }
            best.map(|s| (item, s))
        })
        .collect();
    scored.sort_by_key(|a| std::cmp::Reverse(a.1));
    scored.into_iter().map(|(item, _)| item).collect()
}

/// Resolve a pressed key in the current `AppState` into a semantic `Action`.
pub fn resolve(key: KeyEvent, state: &AppState, chord: &mut PendingChord) -> Option<Action> {
    resolve_key(key, KeyContext::from_state(state), chord)
}

/// Resolve a pressed key into a semantic `Action` for a given `KeyContext`.
pub fn resolve_key(key: KeyEvent, ctx: KeyContext, chord: &mut PendingChord) -> Option<Action> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        *chord = PendingChord::None;
        return match key.code {
            KeyCode::Char('c') => Some(Action::ForceQuit),
            KeyCode::Char('p') => Some(Action::OpenCommandPalette),
            KeyCode::Char('d') => Some(Action::HalfPageDown),
            KeyCode::Char('u') => Some(Action::HalfPageUp),
            _ => None,
        };
    }

    if *chord != PendingChord::None {
        let prev = *chord;
        *chord = PendingChord::None;
        return match (prev, key.code) {
            (PendingChord::G, KeyCode::Char('g')) => Some(Action::MoveTop),
            (PendingChord::Z, KeyCode::Char('a')) => Some(Action::TaskToggleFold),
            _ => None,
        };
    }

    let eff = ctx.effective_panel();

    // Global navigation & discovery bindings
    match key.code {
        KeyCode::Char('q') => return Some(Action::Quit),
        KeyCode::Char('1') => return Some(Action::FocusPanel(Panel::Tasks)),
        KeyCode::Char('2') => return Some(Action::FocusPanel(Panel::Tools)),
        KeyCode::Char('3') => return Some(Action::FocusPanel(Panel::Env)),
        KeyCode::Char('4') => return Some(Action::FocusPanel(Panel::Config)),
        KeyCode::Char('5') => return Some(Action::FocusPanel(Panel::Jobs)),
        KeyCode::Tab => return Some(Action::CycleFocusNext),
        KeyCode::BackTab => return Some(Action::CycleFocusPrev),
        KeyCode::Char('h') | KeyCode::Left => return Some(Action::FocusLeft),
        KeyCode::Char('l') | KeyCode::Right => return Some(Action::FocusRight),
        KeyCode::Char('j') | KeyCode::Down => return Some(Action::MoveDown),
        KeyCode::Char('k') | KeyCode::Up => return Some(Action::MoveUp),
        KeyCode::Home => return Some(Action::MoveTop),
        KeyCode::End | KeyCode::Char('G') => return Some(Action::MoveBottom),
        KeyCode::PageDown => return Some(Action::HalfPageDown),
        KeyCode::PageUp => return Some(Action::HalfPageUp),
        KeyCode::Char('g') if eff != Panel::Config => {
            *chord = PendingChord::G;
            return None;
        }
        KeyCode::Char('z') if eff == Panel::Tasks => {
            *chord = PendingChord::Z;
            return None;
        }
        KeyCode::Char('[') => return Some(Action::PrevSubView),
        KeyCode::Char(']') => return Some(Action::NextSubView),
        KeyCode::Char('+') | KeyCode::Char('=') => return Some(Action::ZoomIn),
        KeyCode::Char('_') | KeyCode::Char('-') => return Some(Action::ZoomOut),
        KeyCode::Char('@') => return Some(Action::ToggleLogPanel),
        KeyCode::Char('/') => return Some(Action::OpenFilter),
        KeyCode::Esc => return Some(Action::ClearFilter),
        KeyCode::Char('?') => return Some(Action::OpenHelp),
        KeyCode::Char('x') => return Some(Action::OpenContextMenu),
        KeyCode::Char(':') => return Some(Action::OpenCommandPalette),
        KeyCode::Char('E') => return Some(Action::OpenProfileSwitcher),
        KeyCode::Char('R') => return Some(Action::ReloadAll),
        KeyCode::Char('r') => {
            return if eff == Panel::Jobs {
                Some(Action::JobRerunSelected)
            } else {
                Some(Action::RerunLastTask)
            };
        }
        _ => {}
    }

    // Panel-specific bindings
    match eff {
        Panel::Tasks => match key.code {
            KeyCode::Enter => Some(Action::TaskRunForeground),
            KeyCode::Char('b') => Some(Action::TaskRunBackground),
            KeyCode::Char('A') => Some(Action::TaskRunWithArgs),
            KeyCode::Char('f') => Some(Action::TaskRunForce),
            KeyCode::Char('w') => Some(Action::TaskWatch),
            KeyCode::Char(' ') => Some(Action::TaskToggleFold),
            KeyCode::Char('H') => Some(Action::TaskToggleHidden),
            KeyCode::Char('M') => Some(Action::TaskToggleMonorepoAll),
            KeyCode::Char('v') => Some(Action::TaskValidate),
            KeyCode::Char('e') => Some(Action::TaskEditSource),
            KeyCode::Char('y') => Some(Action::TaskCopyCommand),
            KeyCode::Char('F') => Some(Action::TaskToggleAutoFollow),
            _ => None,
        },
        Panel::Tools => match key.code {
            KeyCode::Char('a') => Some(Action::ToolOpenRegistry),
            KeyCode::Char('i') => Some(Action::ToolInstallAll),
            KeyCode::Char('u') => Some(Action::ToolUpgradeSelected),
            KeyCode::Char('U') => Some(Action::ToolUpgradeAll),
            KeyCode::Char('V') => Some(Action::ToolUpgradeVulnerable),
            KeyCode::Char('d') => Some(Action::ToolRemoveSelected),
            KeyCode::Char('p') => Some(Action::ToolPrune),
            KeyCode::Char('L') => Some(Action::ToolLockPreview),
            KeyCode::Char('A') => Some(Action::ToolRunAudit),
            KeyCode::Char('O') => Some(Action::ToolRefreshOutdated),
            KeyCode::Char('f') => Some(Action::ToolCycleScope),
            KeyCode::Char('o') => Some(Action::ToolOpenUrl),
            KeyCode::Char('e') => Some(Action::ToolEditSource),
            _ => None,
        },
        Panel::Env => match key.code {
            KeyCode::Char('a') => Some(Action::EnvAddNew),
            KeyCode::Char('e') => Some(Action::EnvAddOrEdit),
            KeyCode::Char('d') => Some(Action::EnvUnset),
            KeyCode::Char('S') => Some(Action::EnvToggleReveal),
            KeyCode::Char('y') => Some(Action::EnvCopyValue),
            KeyCode::Char('Y') => Some(Action::EnvCopyPair),
            KeyCode::Char('f') => Some(Action::EnvCycleFilter),
            _ => None,
        },
        Panel::Config => match key.code {
            KeyCode::Enter | KeyCode::Char('e') => Some(Action::ConfigEditSelected),
            KeyCode::Char('t') => Some(Action::ConfigToggleTrust),
            KeyCode::Char('F') => Some(Action::ConfigFormat),
            KeyCode::Char('D') => Some(Action::ConfigRefreshDoctor),
            KeyCode::Char('g') => Some(Action::ConfigOpenGenerate),
            KeyCode::Char('s') => Some(Action::DaemonToggleSelected),
            KeyCode::Char('S') => Some(Action::DepsSync),
            _ => None,
        },
        Panel::Jobs => match key.code {
            KeyCode::Enter => Some(Action::JobInspect),
            KeyCode::Char('K') => Some(Action::JobKillSelected),
            KeyCode::Char('y') => Some(Action::JobCopyCommand),
            KeyCode::Char('c') => Some(Action::JobClearFinished),
            KeyCode::Char('i') => Some(Action::ToggleInternalCommands),
            KeyCode::Char('F') => Some(Action::JobToggleAutoFollow),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::PathBuf;

    use pretty_assertions::assert_eq;

    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn no_duplicate_key_displays_within_any_panel_scope() {
        let bindings = all_bindings();
        for panel in Panel::ALL {
            let mut seen = HashSet::new();
            for b in &bindings {
                if b.scope == KeyScope::Global || b.scope == KeyScope::Panel(panel) {
                    assert!(
                        seen.insert(b.key_display),
                        "duplicate key `{}` in panel {:?}",
                        b.key_display,
                        panel
                    );
                }
            }
        }
    }

    #[test]
    fn resolves_chords_and_panel_specific_keys_without_conflict() {
        let mut chord = PendingChord::None;
        let ctx_tasks = KeyContext {
            panel: Panel::Tasks,
            focus: FocusArea::Sidebar,
        };
        // `gg` chord -> MoveTop
        assert_eq!(resolve_key(key(KeyCode::Char('g')), ctx_tasks, &mut chord), None);
        assert_eq!(chord, PendingChord::G);
        assert_eq!(
            resolve_key(key(KeyCode::Char('g')), ctx_tasks, &mut chord),
            Some(Action::MoveTop)
        );
        assert_eq!(chord, PendingChord::None);

        // `za` chord -> TaskToggleFold
        assert_eq!(resolve_key(key(KeyCode::Char('z')), ctx_tasks, &mut chord), None);
        assert_eq!(
            resolve_key(key(KeyCode::Char('a')), ctx_tasks, &mut chord),
            Some(Action::TaskToggleFold)
        );

        // `g` on Config immediately opens Generate menu
        let ctx_config = KeyContext {
            panel: Panel::Config,
            focus: FocusArea::Sidebar,
        };
        assert_eq!(
            resolve_key(key(KeyCode::Char('g')), ctx_config, &mut chord),
            Some(Action::ConfigOpenGenerate)
        );

        // `FocusArea::Log` routes to Jobs bindings (`K` -> JobKillSelected, `r` -> JobRerunSelected)
        let ctx_log = KeyContext {
            panel: Panel::Tasks,
            focus: FocusArea::Log,
        };
        assert_eq!(
            resolve_key(key(KeyCode::Char('K')), ctx_log, &mut chord),
            Some(Action::JobKillSelected)
        );
        assert_eq!(
            resolve_key(key(KeyCode::Char('r')), ctx_log, &mut chord),
            Some(Action::JobRerunSelected)
        );
    }

    #[test]
    fn footer_hints_and_help_search_derive_from_registry() {
        let state = AppState::new(PathBuf::from("/tmp/repo"), None, Panel::Tools);
        let hints = footer_hints_for_state(&state);
        assert!(hints.iter().any(|(k, _)| *k == "a"));
        assert!(hints.iter().any(|(k, _)| *k == "x"));
        assert!(hints.iter().any(|(k, _)| *k == "?"));

        let filtered = help_entries(&state, "audit");
        assert!(filtered.iter().any(|b| b.action == Action::ToolRunAudit));

        let palette = palette_items(&state, "doctor");
        assert!(palette.iter().any(|m| m.action == Action::ConfigRefreshDoctor));
    }
}
