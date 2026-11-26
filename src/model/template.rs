use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct WorkspaceTemplate {
    pub name: String,
    pub path: PathBuf,
    pub files: Vec<String>,
    pub mise_toml_preview: String,
}

pub fn discover_templates() -> Vec<WorkspaceTemplate> {
    let base = std::env::var("MISE_TEMPLATES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config/mise/templates")
        });

    let Ok(entries) = fs::read_dir(&base) else {
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

        let mise_toml_preview = fs::read_to_string(path.join("mise.toml"))
            .unwrap_or_else(|_| "(no mise.toml in template)".to_string());

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
