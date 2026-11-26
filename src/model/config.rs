use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use crate::mise::schema::{
    ConfigExplainItem, ConfigFileEntry, DaemonEntry, DepEntry, DoctorReport, SettingEntry,
    TrustEntry,
};
use crate::model::fuzzy::fuzzy_match;
use crate::model::task_tree::ScriptCache;

#[derive(Debug, Clone)]
pub struct ConfigFileRow {
    pub entry: ConfigFileEntry,
    pub trusted: Option<bool>,
    pub trust_status: String,
    pub explain_warnings: usize,
    pub match_indices: Vec<usize>,
}

/// Fuzzy-matched setting row with key highlight indices.
#[derive(Debug, Clone)]
pub struct SettingMatch {
    pub entry: SettingEntry,
    pub score: i32,
    pub match_indices: Vec<usize>,
}

/// Structured presentation summary extracted from `DoctorReport`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DoctorSummary {
    pub version: String,
    pub activated: bool,
    pub shims_on_path: bool,
    pub self_update_available: bool,
    pub shell_display: Option<String>,
    pub error_count: usize,
    pub warning_count: usize,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub config_files: Vec<String>,
    pub ignored_config_files: Vec<String>,
    pub dirs: Vec<(String, String)>,
}

/// Summary counters for the Config & Health sidebar card.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConfigStats {
    pub total_files: usize,
    pub trusted_files: usize,
    pub untrusted_files: usize,
    pub explain_warnings: usize,
    pub doctor_errors: usize,
    pub doctor_warnings: usize,
    pub settings_count: usize,
    pub daemons_count: usize,
    pub deps_count: usize,
}

/// Aggregated view model for Panel::Config (Config hierarchy, Trust, Explain, Doctor, Settings, Services).
#[derive(Debug, Default, Clone)]
pub struct ConfigViewModel {
    pub raw_files: Vec<ConfigFileEntry>,
    pub trust_entries: Vec<TrustEntry>,
    pub explain_items: Vec<ConfigExplainItem>,
    pub explain_by_file: HashMap<PathBuf, Vec<ConfigExplainItem>>,
    pub doctor: Option<DoctorReport>,
    pub settings: Vec<SettingEntry>,
    pub daemons: Vec<DaemonEntry>,
    pub deps: Vec<DepEntry>,
    pub file_cache: ScriptCache,
    pub rows: Vec<ConfigFileRow>,
    pub filtered_settings: Vec<SettingEntry>,
}

impl ConfigViewModel {
    pub fn rebuild(&mut self, filter: &str) {
        let needle = filter.trim();
        let primary_path_norm = self
            .raw_files
            .first()
            .map(|f| normalize_path(Path::new(&f.path)));

        let mut scored_rows: Vec<(ConfigFileRow, i32)> = Vec::new();
        for entry in &self.raw_files {
            let (match_indices, score) = if needle.is_empty() {
                (Vec::new(), 0)
            } else if let Some(m) = fuzzy_match(needle, &entry.path) {
                (m.matched_indices, m.score + 20)
            } else if entry
                .tools
                .iter()
                .any(|t| fuzzy_match(needle, t).is_some())
            {
                (Vec::new(), 10)
            } else {
                continue;
            };

            let (trusted, trust_status) = self.resolve_trust_status(&entry.path);
            let explain_warnings = self
                .explain_items_for_internal(&entry.path, primary_path_norm.as_deref())
                .iter()
                .filter(|item| item.deprecated || !item.known)
                .count();

            scored_rows.push((
                ConfigFileRow {
                    entry: entry.clone(),
                    trusted,
                    trust_status,
                    explain_warnings,
                    match_indices,
                },
                score,
            ));
        }

        if !needle.is_empty() {
            scored_rows.sort_by(|(_, sa), (_, sb)| sb.cmp(sa));
        }
        self.rows = scored_rows.into_iter().map(|(r, _)| r).collect();

        self.filtered_settings = self
            .search_settings(filter)
            .into_iter()
            .map(|m| m.entry)
            .collect();
    }

    /// Store `mise config explain --json` items scoped to a specific config file path.
    pub fn set_explain_for_file(
        &mut self,
        path: impl AsRef<Path>,
        items: Vec<ConfigExplainItem>,
    ) {
        let norm = normalize_path(path.as_ref());
        self.explain_by_file.insert(norm, items);
    }

    /// Return all explain items associated with `path` (falling back to top-level `explain_items`
    /// only for the primary config file `raw_files[0]`).
    pub fn explain_items_for(&self, path: &str) -> &[ConfigExplainItem] {
        let primary_norm = self
            .raw_files
            .first()
            .map(|f| normalize_path(Path::new(&f.path)));
        self.explain_items_for_internal(path, primary_norm.as_deref())
    }

    /// Return diagnostic items (unknown or deprecated keys) for `path`, sorted by line number.
    pub fn diagnostics_for(&self, path: &str) -> Vec<&ConfigExplainItem> {
        let mut diags: Vec<&ConfigExplainItem> = self
            .explain_items_for(path)
            .iter()
            .filter(|item| item.deprecated || !item.known)
            .collect();
        diags.sort_by_key(|item| item.line);
        diags
    }

    /// Fuzzy search across `settings` (matching key, description, source, and formatted value).
    pub fn search_settings(&self, query: &str) -> Vec<SettingMatch> {
        let needle = query.trim();
        if needle.is_empty() {
            return self
                .settings
                .iter()
                .cloned()
                .map(|entry| SettingMatch {
                    entry,
                    score: 0,
                    match_indices: Vec::new(),
                })
                .collect();
        }

        let mut matches = Vec::new();
        for setting in &self.settings {
            let mut best_score: Option<i32> = None;
            let mut key_indices = Vec::new();

            if let Some(m) = fuzzy_match(needle, &setting.key) {
                best_score = Some(m.score + 120);
                key_indices = m.matched_indices;
            }
            if let Some(m) = fuzzy_match(needle, &setting.description) {
                let s = m.score + 40;
                if best_score.is_none_or(|cur| s > cur) {
                    best_score = Some(s);
                }
            }
            if let Some(src) = setting.source.as_deref()
                && let Some(m) = fuzzy_match(needle, src)
            {
                let s = m.score + 20;
                if best_score.is_none_or(|cur| s > cur) {
                    best_score = Some(s);
                }
            }
            let val_str = setting.value.to_string();
            if let Some(m) = fuzzy_match(needle, &val_str)
                && best_score.is_none_or(|cur| m.score > cur)
            {
                best_score = Some(m.score);
            }

            if let Some(score) = best_score {
                matches.push(SettingMatch {
                    entry: setting.clone(),
                    score,
                    match_indices: key_indices,
                });
            }
        }

        matches.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.entry.key.cmp(&b.entry.key))
        });
        matches
    }

    /// Preload a config file into the mtime + size-capped cache (never call during render).
    pub fn preload_file(&mut self, path: PathBuf) {
        let _ = self.file_cache.preload(path);
    }

    /// Zero-I/O lookup for a preloaded config file's contents.
    pub fn cached_file_content(&self, path: &str) -> Option<&str> {
        self.file_cache.get_cached(Path::new(path))
    }

    pub fn doctor_error_count(&self) -> usize {
        self.doctor.as_ref().map(|d| d.errors.len()).unwrap_or(0)
    }

    pub fn doctor_warning_count(&self) -> usize {
        self.doctor.as_ref().map(|d| d.warnings.len()).unwrap_or(0)
    }

    /// Build a structured presentation summary from `DoctorReport`.
    pub fn doctor_summary(&self) -> Option<DoctorSummary> {
        let d = self.doctor.as_ref()?;
        let shell_display = d.shell.as_ref().map(|s| {
            if s.version.is_empty() {
                s.name.clone()
            } else {
                format!("{} {}", s.name, s.version)
            }
        });
        let dirs = d
            .dirs
            .iter()
            .map(|(k, v)| {
                let val = v
                    .as_str()
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| v.to_string());
                (k.clone(), val)
            })
            .collect();

        Some(DoctorSummary {
            version: d.version.clone(),
            activated: d.activated,
            shims_on_path: d.shims_on_path,
            self_update_available: d.self_update_available,
            shell_display,
            error_count: d.errors.len(),
            warning_count: d.warnings.len(),
            errors: d.errors.clone(),
            warnings: d.warnings.clone(),
            config_files: d.config_files.clone(),
            ignored_config_files: d.ignored_config_files.clone(),
            dirs,
        })
    }

    pub fn stats(&self) -> ConfigStats {
        let mut trusted_files = 0usize;
        let mut untrusted_files = 0usize;
        let mut explain_warnings = 0usize;
        let primary_norm = self
            .raw_files
            .first()
            .map(|f| normalize_path(Path::new(&f.path)));

        for entry in &self.raw_files {
            if let (Some(trusted), _) = self.resolve_trust_status(&entry.path) {
                if trusted {
                    trusted_files += 1;
                } else {
                    untrusted_files += 1;
                }
            }
            explain_warnings += self
                .explain_items_for_internal(&entry.path, primary_norm.as_deref())
                .iter()
                .filter(|item| item.deprecated || !item.known)
                .count();
        }

        ConfigStats {
            total_files: self.raw_files.len(),
            trusted_files,
            untrusted_files,
            explain_warnings,
            doctor_errors: self.doctor_error_count(),
            doctor_warnings: self.doctor_warning_count(),
            settings_count: self.settings.len(),
            daemons_count: self.daemons.len(),
            deps_count: self.deps.len(),
        }
    }

    pub fn has_services(&self) -> bool {
        !self.daemons.is_empty() || !self.deps.is_empty()
    }

    fn explain_items_for_internal(
        &self,
        path: &str,
        primary_norm: Option<&Path>,
    ) -> &[ConfigExplainItem] {
        let norm = normalize_path(Path::new(path));
        if let Some(items) = self.explain_by_file.get(&norm) {
            return items.as_slice();
        }
        if primary_norm.is_some_and(|p| p == norm) {
            return self.explain_items.as_slice();
        }
        &[]
    }

    /// Match a `ConfigFileEntry` path against `trust_entries` using canonicalized/normalized paths.
    /// Supports both exact file matches and directory-level trust entries (e.g. `~/proj: trusted`
    /// applying to `/Users/me/proj/mise.toml`), preferring the most specific (deepest) match.
    fn resolve_trust_for(&self, config_path: &str) -> Option<&TrustEntry> {
        let target = normalize_path(Path::new(config_path));
        let mut best: Option<(usize, &TrustEntry)> = None;

        for trust in &self.trust_entries {
            let trust_norm = normalize_path(Path::new(&trust.path));
            let specificity = if trust_norm == target {
                usize::MAX
            } else if target.starts_with(&trust_norm) {
                trust_norm.components().count()
            } else {
                continue;
            };

            if best.is_none_or(|(best_spec, _)| specificity > best_spec) {
                best = Some((specificity, trust));
            }
        }

        best.map(|(_, t)| t)
    }

    /// Resolve effective trust state for `config_path`: explicit `mise trust --show` entries take
    /// precedence, while files inside mise's global config directories (`~/.config/mise`,
    /// `$MISE_CONFIG_DIR`, `$XDG_CONFIG_HOME/mise`, `/etc/mise`) are implicitly trusted by mise.
    fn resolve_trust_status(&self, config_path: &str) -> (Option<bool>, String) {
        if let Some(trust) = self.resolve_trust_for(config_path) {
            return (Some(trust.trusted), trust.raw_status.clone());
        }
        let norm = normalize_path(Path::new(config_path));
        if is_global_mise_config_path(&norm) {
            return (Some(true), "trusted (global)".to_string());
        }
        (None, "unknown".to_string())
    }
}

/// Returns `true` if `norm_path` resides in mise's implicitly-trusted global config locations.
pub fn is_global_mise_config_path(norm_path: &Path) -> bool {
    if let Ok(global_file) = std::env::var("MISE_GLOBAL_CONFIG_FILE")
        && !global_file.trim().is_empty()
        && normalize_path(Path::new(global_file.trim())) == norm_path
    {
        return true;
    }

    let mut global_dirs: Vec<PathBuf> = Vec::new();
    if let Ok(cfg_dir) = std::env::var("MISE_CONFIG_DIR")
        && !cfg_dir.trim().is_empty()
    {
        global_dirs.push(normalize_path(Path::new(cfg_dir.trim())));
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME")
        && !xdg.trim().is_empty()
    {
        global_dirs.push(normalize_path(&PathBuf::from(xdg.trim()).join("mise")));
    }
    if let Ok(home) = std::env::var("HOME")
        && !home.trim().is_empty()
    {
        global_dirs.push(normalize_path(
            &PathBuf::from(home.trim()).join(".config").join("mise"),
        ));
    }
    global_dirs.push(PathBuf::from("/etc/mise"));

    global_dirs.iter().any(|dir| norm_path.starts_with(dir))
}

/// Expand leading `~`, canonicalize when path exists, and lexically normalize components.
pub fn normalize_path(path: &Path) -> PathBuf {
    let expanded = expand_tilde(path);
    if let Ok(canon) = std::fs::canonicalize(&expanded) {
        return strip_macos_private_owned(canon);
    }
    strip_macos_private_owned(lexical_normalize(&expanded))
}

fn expand_tilde(path: &Path) -> PathBuf {
    let Some(s) = path.to_str() else {
        return path.to_path_buf();
    };
    if s == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = s.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    path.to_path_buf()
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn strip_macos_private_owned(path: PathBuf) -> PathBuf {
    if let Ok(rest) = path.strip_prefix("/private")
        && (rest.starts_with("var") || rest.starts_with("tmp"))
    {
        return PathBuf::from("/").join(rest);
    }
    path
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::*;
    use crate::mise::schema::DoctorShell;

    #[test]
    fn resolves_trust_across_tilde_and_directory_hierarchy() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/test".to_string());
        let repo_file = format!("{home}/work/app/mise.toml");
        let global_file = format!("{home}/.config/mise/config.toml");
        let other_file = "/tmp/unmatched-project/mise.toml".to_string();

        let mut vm = ConfigViewModel {
            raw_files: vec![
                ConfigFileEntry {
                    path: repo_file.clone(),
                    tools: vec!["rust".to_string()],
                    is_override: false,
                },
                ConfigFileEntry {
                    path: global_file.clone(),
                    tools: vec!["node".to_string()],
                    is_override: true,
                },
                ConfigFileEntry {
                    path: other_file,
                    tools: vec![],
                    is_override: false,
                },
            ],
            trust_entries: vec![
                TrustEntry {
                    path: "~/work".to_string(),
                    trusted: false,
                    raw_status: "untrusted".to_string(),
                },
                TrustEntry {
                    path: "~/work/app".to_string(),
                    trusted: true,
                    raw_status: "trusted".to_string(),
                },
            ],
            ..Default::default()
        };

        vm.rebuild("");
        assert_eq!(vm.rows.len(), 3);
        // More specific ~/work/app overrides ~/work
        assert_eq!(vm.rows[0].trusted, Some(true));
        assert_eq!(vm.rows[0].trust_status, "trusted");
        // Global ~/.config/mise/* is implicitly trusted by mise
        assert_eq!(vm.rows[1].trusted, Some(true));
        assert_eq!(vm.rows[1].trust_status, "trusted (global)");
        // Unmatched non-global config reports unknown
        assert_eq!(vm.rows[2].trusted, None);
        assert_eq!(vm.rows[2].trust_status, "unknown");
    }

    #[test]
    fn groups_explain_diagnostics_per_file_and_sorts_by_line() {
        let mut vm = ConfigViewModel {
            raw_files: vec![
                ConfigFileEntry {
                    path: "/proj/mise.toml".to_string(),
                    tools: vec![],
                    is_override: false,
                },
                ConfigFileEntry {
                    path: "/global/config.toml".to_string(),
                    tools: vec![],
                    is_override: false,
                },
            ],
            // Unscoped explain_items apply only to primary config file (/proj/mise.toml)
            explain_items: vec![
                ConfigExplainItem {
                    key: "legacy_key".to_string(),
                    line: 20,
                    deprecated: true,
                    known: true,
                    ..Default::default()
                },
                ConfigExplainItem {
                    key: "typo_key".to_string(),
                    line: 5,
                    deprecated: false,
                    known: false,
                    suggestion: Some("tools".to_string()),
                    ..Default::default()
                },
                ConfigExplainItem {
                    key: "tools".to_string(),
                    line: 1,
                    deprecated: false,
                    known: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        vm.set_explain_for_file(
            "/global/config.toml",
            vec![ConfigExplainItem {
                key: "unknown_global".to_string(),
                line: 9,
                deprecated: false,
                known: false,
                ..Default::default()
            }],
        );

        vm.rebuild("");
        assert_eq!(vm.rows[0].explain_warnings, 2);
        assert_eq!(vm.rows[1].explain_warnings, 1);

        let proj_diags = vm.diagnostics_for("/proj/mise.toml");
        let lines: Vec<u32> = proj_diags.iter().map(|d| d.line).collect();
        assert_eq!(lines, vec![5, 20]);
    }

    #[test]
    fn summarizes_doctor_and_fuzzy_searches_settings() {
        let mut dirs = BTreeMap::new();
        dirs.insert("cache".to_string(), json!("/tmp/mise-cache"));
        let mut vm = ConfigViewModel {
            doctor: Some(DoctorReport {
                activated: true,
                version: "2026.11.13".to_string(),
                shell: Some(DoctorShell {
                    name: "zsh".to_string(),
                    version: "5.9".to_string(),
                }),
                dirs,
                shims_on_path: true,
                warnings: vec!["legacy plugin".to_string()],
                errors: vec!["broken shim".to_string(), "bad env".to_string()],
                ..Default::default()
            }),
            settings: vec![
                SettingEntry {
                    key: "experimental".to_string(),
                    value: json!(true),
                    kind: "bool".to_string(),
                    description: "Enable experimental features".to_string(),
                    source: Some("~/.config/mise/config.toml".to_string()),
                },
                SettingEntry {
                    key: "python.uv_venv_auto".to_string(),
                    value: json!("create|source"),
                    kind: "string".to_string(),
                    description: "Auto-manage uv virtualenvs".to_string(),
                    source: None,
                },
            ],
            daemons: vec![DaemonEntry {
                name: "postgres".to_string(),
                status: "running".to_string(),
                pid: Some(1234),
                ..Default::default()
            }],
            deps: vec![DepEntry {
                name: "npm".to_string(),
                status: "fresh".to_string(),
                detail: "node_modules up to date".to_string(),
            }],
            ..Default::default()
        };

        vm.rebuild("uv");
        assert_eq!(vm.doctor_error_count(), 2);
        assert_eq!(vm.doctor_warning_count(), 1);
        assert!(vm.has_services());

        let summary = vm.doctor_summary().unwrap();
        assert_eq!(summary.shell_display.as_deref(), Some("zsh 5.9"));
        assert_eq!(summary.error_count, 2);
        assert_eq!(summary.warning_count, 1);
        assert_eq!(
            summary.dirs,
            vec![("cache".to_string(), "/tmp/mise-cache".to_string())]
        );

        assert_eq!(vm.filtered_settings.len(), 1);
        assert_eq!(vm.filtered_settings[0].key, "python.uv_venv_auto");

        let matches = vm.search_settings("exp");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].entry.key, "experimental");
        assert_eq!(matches[0].match_indices, vec![0, 1, 2]);
    }
}
