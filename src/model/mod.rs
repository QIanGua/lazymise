pub mod config;
pub mod env;
pub mod fuzzy;
pub mod task_tree;
pub mod template;
pub mod tools;

pub use config::{ConfigFileRow, ConfigStats, ConfigViewModel, DoctorSummary, SettingMatch};
pub use env::{EnvRow, EnvSourceKind, EnvStats, EnvViewModel, PathEntry};
pub use fuzzy::{FuzzyMatch, fuzzy_match};
pub use task_tree::{ScriptCache, TaskRow, TaskTree, build_task_rows};
pub use template::{
    MiseGenerateTarget, ScaffoldEntry, ScaffoldKind, WorkspaceTemplate, build_scaffold_catalog,
    discover_templates, official_generate_targets,
};
pub use tools::{RegistryMatch, ToolRow, ToolStats, ToolViewModel, filter_registry_entries};
