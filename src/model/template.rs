use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

const TEMPLATE_PREVIEW_MAX_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceTemplate {
    pub name: String,
    pub path: PathBuf,
    pub files: Vec<String>,
    pub mise_toml_preview: String,
}

/// Official `mise generate <target>` catalog entry for the `g` Generate & Scaffold modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MiseGenerateTarget {
    pub subcommand: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub default_args: &'static [&'static str],
    pub command_preview: &'static str,
}

/// All supported official `mise generate` subcommands.
pub const OFFICIAL_GENERATE_TARGETS: &[MiseGenerateTarget] = &[
    MiseGenerateTarget {
        subcommand: "config",
        label: "mise.toml config",
        description: "Generate a starter mise.toml configuration file",
        default_args: &["generate", "config"],
        command_preview: "mise generate config",
    },
    MiseGenerateTarget {
        subcommand: "devcontainer",
        label: "Dev Container",
        description: "Generate .devcontainer/devcontainer.json with mise",
        default_args: &["generate", "devcontainer"],
        command_preview: "mise generate devcontainer",
    },
    MiseGenerateTarget {
        subcommand: "github-action",
        label: "GitHub Actions workflow",
        description: "Generate a CI workflow using jdx/mise-action",
        default_args: &["generate", "github-action"],
        command_preview: "mise generate github-action",
    },
    MiseGenerateTarget {
        subcommand: "git-pre-commit",
        label: "Git pre-commit hook",
        description: "Generate a git pre-commit hook running mise tasks",
        default_args: &["generate", "git-pre-commit"],
        command_preview: "mise generate git-pre-commit",
    },
    MiseGenerateTarget {
        subcommand: "task-docs",
        label: "Task documentation",
        description: "Generate Markdown documentation for project tasks",
        default_args: &["generate", "task-docs"],
        command_preview: "mise generate task-docs",
    },
    MiseGenerateTarget {
        subcommand: "task-stubs",
        label: "Task shell stubs",
        description: "Generate standalone executable stubs for mise tasks",
        default_args: &["generate", "task-stubs"],
        command_preview: "mise generate task-stubs",
    },
    MiseGenerateTarget {
        subcommand: "tool-stub",
        label: "Tool stub",
        description: "Generate a lazy tool stub file",
        default_args: &["generate", "tool-stub"],
        command_preview: "mise generate tool-stub",
    },
];

pub fn official_generate_targets() -> &'static [MiseGenerateTarget] {
    OFFICIAL_GENERATE_TARGETS
}

/// Discriminator for entries in the unified `g` Generate & Scaffold menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScaffoldKind {
    MiseGenerate {
        subcommand: String,
        args: Vec<String>,
    },
    WorkspaceTemplate {
        template_name: String,
        path: PathBuf,
    },
}

/// Unified item consumed by the `g` Generate & Scaffold modal (official generators + custom templates).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScaffoldEntry {
    pub label: String,
    pub description: String,
    pub command_preview: String,
    pub kind: ScaffoldKind,
}

/// Combine official `mise generate` targets and discovered custom workspace templates.
pub fn build_scaffold_catalog(custom_templates: &[WorkspaceTemplate]) -> Vec<ScaffoldEntry> {
    let mut items = Vec::with_capacity(OFFICIAL_GENERATE_TARGETS.len() + custom_templates.len());

    for target in OFFICIAL_GENERATE_TARGETS {
        items.push(ScaffoldEntry {
            label: format!("mise generate {}", target.subcommand),
            description: target.description.to_string(),
            command_preview: target.command_preview.to_string(),
            kind: ScaffoldKind::MiseGenerate {
                subcommand: target.subcommand.to_string(),
                args: target.default_args.iter().map(|s| (*s).to_string()).collect(),
            },
        });
    }

    for tpl in custom_templates {
        items.push(ScaffoldEntry {
            label: format!("template: {}", tpl.name),
            description: format!("Custom workspace template ({} files)", tpl.files.len()),
            command_preview: format!("mise run new {} <name>", tpl.name),
            kind: ScaffoldKind::WorkspaceTemplate {
                template_name: tpl.name.clone(),
                path: tpl.path.clone(),
            },
        });
    }

    items
}

/// Discover custom templates under `$MISE_TEMPLATES_DIR` or `~/.config/mise/templates`.
pub fn discover_templates() -> Vec<WorkspaceTemplate> {
    let base = std::env::var("MISE_TEMPLATES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config/mise/templates")
        });

    discover_templates_in(&base)
}

/// Discover custom workspace templates inside an explicit directory.
pub fn discover_templates_in(base: &Path) -> Vec<WorkspaceTemplate> {
    let Ok(entries) = fs::read_dir(base) else {
        return Vec::new();
    };

    let mut templates = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('_') || name.starts_with('.') {
            continue;
        }

        let mut files = Vec::new();
        collect_rel_files(&path, &path, &mut files);
        files.sort();

        let mise_toml_preview = read_preview_bounded(&path.join("mise.toml"))
            .unwrap_or_else(|| "(no mise.toml in template)".to_string());

        templates.push(WorkspaceTemplate {
            name,
            path,
            files,
            mise_toml_preview,
        });
    }

    templates.sort_by(|a, b| a.name.cmp(&b.name));
    templates
}

fn read_preview_bounded(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut buf = Vec::new();
    file.take(TEMPLATE_PREVIEW_MAX_BYTES as u64)
        .read_to_end(&mut buf)
        .ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn collect_rel_files(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        let fname = entry.file_name().to_string_lossy().to_string();
        if fname == ".DS_Store" {
            continue;
        }
        if p.is_dir() {
            collect_rel_files(root, &p, out);
        } else if let Ok(rel) = p.strip_prefix(root) {
            out.push(rel.to_string_lossy().to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn official_generate_catalog_covers_all_seven_targets() {
        let subcommands: Vec<&str> = official_generate_targets()
            .iter()
            .map(|t| t.subcommand)
            .collect();
        assert_eq!(
            subcommands,
            vec![
                "config",
                "devcontainer",
                "github-action",
                "git-pre-commit",
                "task-docs",
                "task-stubs",
                "tool-stub",
            ]
        );
    }

    #[test]
    fn discovers_custom_templates_and_builds_unified_scaffold_catalog() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "lazymise-templates-{}-{nanos}",
            std::process::id()
        ));
        let rust_tpl = base.join("rust-cli");
        let ignored_hidden = base.join(".git");
        let ignored_underscore = base.join("_shared");

        fs::create_dir_all(rust_tpl.join("src")).unwrap();
        fs::create_dir_all(&ignored_hidden).unwrap();
        fs::create_dir_all(&ignored_underscore).unwrap();
        fs::write(rust_tpl.join("mise.toml"), "[tools]\nrust = \"stable\"\n").unwrap();
        fs::write(rust_tpl.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(rust_tpl.join(".DS_Store"), "junk").unwrap();

        let discovered = discover_templates_in(&base);
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].name, "rust-cli");
        assert_eq!(discovered[0].files, vec!["mise.toml", "src/main.rs"]);
        assert!(discovered[0].mise_toml_preview.contains("rust = \"stable\""));

        let catalog = build_scaffold_catalog(&discovered);
        assert_eq!(catalog.len(), OFFICIAL_GENERATE_TARGETS.len() + 1);
        assert_eq!(catalog.last().unwrap().label, "template: rust-cli");

        let _ = fs::remove_dir_all(&base);
    }
}
