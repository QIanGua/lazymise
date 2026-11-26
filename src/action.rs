use std::fmt;

/// Primary workspace panels in the lazygit-style accordion sidebar + bottom log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Panel {
    Tasks,
    Tools,
    Env,
    Config,
    Jobs,
}

impl Panel {
    pub const SIDEBAR_PANELS: [Panel; 4] = [Panel::Tasks, Panel::Tools, Panel::Env, Panel::Config];
    pub const ALL: [Panel; 5] = [
        Panel::Tasks,
        Panel::Tools,
        Panel::Env,
        Panel::Config,
        Panel::Jobs,
    ];

    pub fn number(self) -> u8 {
        match self {
            Panel::Tasks => 1,
            Panel::Tools => 2,
            Panel::Env => 3,
            Panel::Config => 4,
            Panel::Jobs => 5,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Panel::Tasks => "TASKS",
            Panel::Tools => "TOOLS",
            Panel::Env => "ENV",
            Panel::Config => "CONFIG & HEALTH",
            Panel::Jobs => "JOBS & LOG",
        }
    }

    pub fn short_title(self) -> &'static str {
        match self {
            Panel::Tasks => "TASKS",
            Panel::Tools => "TOOLS",
            Panel::Env => "ENV",
            Panel::Config => "CONFIG",
            Panel::Jobs => "JOBS",
        }
    }
}

/// Sub-views inside the right main pane for Panel::Tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TaskSubView {
    #[default]
    Spec,
    Output,
    Deps,
    History,
}

impl TaskSubView {
    pub const ALL: [TaskSubView; 4] = [
        TaskSubView::Spec,
        TaskSubView::Output,
        TaskSubView::Deps,
        TaskSubView::History,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TaskSubView::Spec => "Spec",
            TaskSubView::Output => "Output",
            TaskSubView::Deps => "Deps",
            TaskSubView::History => "History",
        }
    }

    pub fn next(self) -> Self {
        match self {
            TaskSubView::Spec => TaskSubView::Output,
            TaskSubView::Output => TaskSubView::Deps,
            TaskSubView::Deps => TaskSubView::History,
            TaskSubView::History => TaskSubView::Spec,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            TaskSubView::Spec => TaskSubView::History,
            TaskSubView::Output => TaskSubView::Spec,
            TaskSubView::Deps => TaskSubView::Output,
            TaskSubView::History => TaskSubView::Deps,
        }
    }
}

/// Sub-views inside the right main pane for Panel::Tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ToolSubView {
    #[default]
    Details,
    Outdated,
    Audit,
    Lock,
}

impl ToolSubView {
    pub const ALL: [ToolSubView; 4] = [
        ToolSubView::Details,
        ToolSubView::Outdated,
        ToolSubView::Audit,
        ToolSubView::Lock,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ToolSubView::Details => "Details",
            ToolSubView::Outdated => "Outdated",
            ToolSubView::Audit => "Audit",
            ToolSubView::Lock => "Lock",
        }
    }

    pub fn next(self) -> Self {
        match self {
            ToolSubView::Details => ToolSubView::Outdated,
            ToolSubView::Outdated => ToolSubView::Audit,
            ToolSubView::Audit => ToolSubView::Lock,
            ToolSubView::Lock => ToolSubView::Details,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            ToolSubView::Details => ToolSubView::Lock,
            ToolSubView::Outdated => ToolSubView::Details,
            ToolSubView::Audit => ToolSubView::Outdated,
            ToolSubView::Lock => ToolSubView::Audit,
        }
    }
}

/// Sub-views inside the right main pane for Panel::Env.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EnvSubView {
    #[default]
    Detail,
    Paths,
    Vars,
}

impl EnvSubView {
    pub const ALL: [EnvSubView; 3] = [EnvSubView::Detail, EnvSubView::Paths, EnvSubView::Vars];

    pub fn label(self) -> &'static str {
        match self {
            EnvSubView::Detail => "Value & Source",
            EnvSubView::Paths => "PATH",
            EnvSubView::Vars => "Vars",
        }
    }

    pub fn next(self) -> Self {
        match self {
            EnvSubView::Detail => EnvSubView::Paths,
            EnvSubView::Paths => EnvSubView::Vars,
            EnvSubView::Vars => EnvSubView::Detail,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            EnvSubView::Detail => EnvSubView::Vars,
            EnvSubView::Paths => EnvSubView::Detail,
            EnvSubView::Vars => EnvSubView::Paths,
        }
    }
}

/// Sub-views inside the right main pane for Panel::Config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ConfigSubView {
    #[default]
    File,
    Doctor,
    Settings,
    Services,
}

impl ConfigSubView {
    pub const ALL: [ConfigSubView; 4] = [
        ConfigSubView::File,
        ConfigSubView::Doctor,
        ConfigSubView::Settings,
        ConfigSubView::Services,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ConfigSubView::File => "File & Explain",
            ConfigSubView::Doctor => "Doctor",
            ConfigSubView::Settings => "Settings",
            ConfigSubView::Services => "Daemons & Deps",
        }
    }

    pub fn next(self) -> Self {
        match self {
            ConfigSubView::File => ConfigSubView::Doctor,
            ConfigSubView::Doctor => ConfigSubView::Settings,
            ConfigSubView::Settings => ConfigSubView::Services,
            ConfigSubView::Services => ConfigSubView::File,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            ConfigSubView::File => ConfigSubView::Services,
            ConfigSubView::Doctor => ConfigSubView::File,
            ConfigSubView::Settings => ConfigSubView::Doctor,
            ConfigSubView::Services => ConfigSubView::Settings,
        }
    }
}

/// Lazygit-style screen zoom mode (`+` / `_`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScreenMode {
    #[default]
    Normal,
    Half,
    Full,
}

impl ScreenMode {
    pub fn zoom_in(self) -> Self {
        match self {
            ScreenMode::Normal => ScreenMode::Half,
            ScreenMode::Half => ScreenMode::Full,
            ScreenMode::Full => ScreenMode::Full,
        }
    }

    pub fn zoom_out(self) -> Self {
        match self {
            ScreenMode::Normal => ScreenMode::Normal,
            ScreenMode::Half => ScreenMode::Normal,
            ScreenMode::Full => ScreenMode::Half,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ScreenMode::Normal => "normal",
            ScreenMode::Half => "half",
            ScreenMode::Full => "full",
        }
    }
}

/// Which major region currently holds keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FocusArea {
    #[default]
    Sidebar,
    Main,
    Log,
}

/// Visibility state of the bottom Command Log & Jobs drawer (`@`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LogPanelMode {
    Hidden,
    #[default]
    Compact,
    Expanded,
}

impl LogPanelMode {
    pub fn cycle(self) -> Self {
        match self {
            LogPanelMode::Hidden => LogPanelMode::Compact,
            LogPanelMode::Compact => LogPanelMode::Expanded,
            LogPanelMode::Expanded => LogPanelMode::Hidden,
        }
    }
}

/// Async data domains fetched by `MiseLoader`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Domain {
    Tasks,
    TaskInfo,
    TaskDeps,
    TaskHistory,
    Tools,
    ToolDetail,
    ToolVersions,
    Registry,
    Outdated,
    Audit,
    LockDryRun,
    Env,
    Config,
    ConfigExplain,
    Doctor,
    Settings,
    Daemons,
    Deps,
    Templates,
}

impl fmt::Display for Domain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Domain::Tasks => "tasks",
            Domain::TaskInfo => "task info",
            Domain::TaskDeps => "task deps",
            Domain::TaskHistory => "task history",
            Domain::Tools => "tools",
            Domain::ToolDetail => "tool detail",
            Domain::ToolVersions => "tool versions",
            Domain::Registry => "registry",
            Domain::Outdated => "outdated",
            Domain::Audit => "audit",
            Domain::LockDryRun => "lock dry-run",
            Domain::Env => "env",
            Domain::Config => "config",
            Domain::ConfigExplain => "config explain",
            Domain::Doctor => "doctor",
            Domain::Settings => "settings",
            Domain::Daemons => "daemons",
            Domain::Deps => "deps",
            Domain::Templates => "templates",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ToolScope {
    #[default]
    All,
    Active,
    Project,
    Missing,
    Prunable,
}

impl ToolScope {
    pub fn next(self) -> Self {
        match self {
            ToolScope::All => ToolScope::Active,
            ToolScope::Active => ToolScope::Project,
            ToolScope::Project => ToolScope::Missing,
            ToolScope::Missing => ToolScope::Prunable,
            ToolScope::Prunable => ToolScope::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ToolScope::All => "all",
            ToolScope::Active => "active",
            ToolScope::Project => "project",
            ToolScope::Missing => "missing",
            ToolScope::Prunable => "prunable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EnvFilter {
    #[default]
    All,
    Project,
    Local,
    Global,
    ToolInjected,
    Redacted,
}

impl EnvFilter {
    pub fn next(self) -> Self {
        match self {
            EnvFilter::All => EnvFilter::Project,
            EnvFilter::Project => EnvFilter::Local,
            EnvFilter::Local => EnvFilter::Global,
            EnvFilter::Global => EnvFilter::ToolInjected,
            EnvFilter::ToolInjected => EnvFilter::Redacted,
            EnvFilter::Redacted => EnvFilter::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EnvFilter::All => "all",
            EnvFilter::Project => "project",
            EnvFilter::Local => "local",
            EnvFilter::Global => "global",
            EnvFilter::ToolInjected => "tool",
            EnvFilter::Redacted => "redacted",
        }
    }
}

/// Active `MISE_ENV` profile specification (None = default environment).
pub type ProfileSpec = Option<String>;

/// Unified semantic actions dispatched by Keymap, Command Palette, Context Menu, and Mouse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    // Global navigation & layout
    Quit,
    ForceQuit,
    FocusPanel(Panel),
    CycleFocusNext,
    CycleFocusPrev,
    FocusLeft,
    FocusRight,
    NextSubView,
    PrevSubView,
    ZoomIn,
    ZoomOut,
    ToggleLogPanel,
    MoveUp,
    MoveDown,
    MoveTop,
    MoveBottom,
    HalfPageDown,
    HalfPageUp,

    // Global discovery & controls
    OpenFilter,
    ClearFilter,
    OpenHelp,
    OpenContextMenu,
    OpenCommandPalette,
    OpenProfileSwitcher,
    ReloadAll,
    RerunLastTask,

    // Tasks panel
    TaskRunForeground,
    TaskRunBackground,
    TaskRunWithArgs,
    TaskRunForce,
    TaskWatch,
    TaskToggleFold,
    TaskToggleHidden,
    TaskToggleMonorepoAll,
    TaskValidate,
    TaskEditSource,
    TaskCopyCommand,
    TaskToggleAutoFollow,

    // Tools panel
    ToolOpenRegistry,
    ToolInstallAll,
    ToolUpgradeSelected,
    ToolUpgradeAll,
    ToolUpgradeVulnerable,
    ToolRemoveSelected,
    ToolPrune,
    ToolLockPreview,
    ToolLockApply,
    ToolRunAudit,
    ToolRefreshOutdated,
    ToolCycleScope,
    ToolOpenUrl,
    ToolEditSource,

    // Env panel
    EnvAddNew,
    EnvAddOrEdit,
    EnvUnset,
    EnvToggleReveal,
    EnvCopyValue,
    EnvCopyPair,
    EnvCycleFilter,

    // Config & Health panel
    ConfigEditSelected,
    ConfigToggleTrust,
    ConfigFormat,
    ConfigRefreshDoctor,
    ConfigDoctorReshim,
    ConfigDoctorProject,
    ConfigOpenGenerate,
    DaemonToggleSelected,
    DaemonRestartSelected,
    DepsSync,

    // Jobs & Command Log panel
    JobInspect,
    JobKillSelected,
    JobRerunSelected,
    JobCopyCommand,
    JobClearFinished,
    JobToggleAutoFollow,
    ToggleInternalCommands,
}
