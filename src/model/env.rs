use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::action::EnvFilter;
use crate::mise::schema::{MiseEnvExtendedMap, MiseVarsMap};
use crate::model::fuzzy::fuzzy_match;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvSourceKind {
    Project,
    Local,
    Global,
    ToolInjected,
    System,
}

impl EnvSourceKind {
    pub fn label(self) -> &'static str {
        match self {
            EnvSourceKind::Project => "project",
            EnvSourceKind::Local => "local",
            EnvSourceKind::Global => "global",
            EnvSourceKind::ToolInjected => "tool",
            EnvSourceKind::System => "system",
        }
    }
}

#[derive(Debug, Clone)]
pub struct EnvRow {
    pub key: String,
    pub value: String,
    pub source: Option<String>,
    pub tool: Option<String>,
    pub source_kind: EnvSourceKind,
    pub sensitive: bool,
    pub officially_redacted: bool,
    pub match_indices: Vec<usize>,
}

/// Heuristic sensitive key patterns merged with mise's official `--redacted` key set.
const SENSITIVE_KEY_PATTERNS: &[&str] = &[
    "SECRET",
    "TOKEN",
    "PASSWORD",
    "PASSWD",
    "API_KEY",
    "APIKEY",
    "ACCESS_KEY",
    "PRIVATE_KEY",
    "SECRET_KEY",
    "CREDENTIAL",
    "AUTH",
    "JWT",
    "BEARER",
    "DATABASE_URL",
    "DB_URL",
    "REDIS_URL",
    "MONGO_URL",
    "DSN",
    "CONNECTION_STRING",
    "COOKIE",
    "SESSION_ID",
    "SESSION_KEY",
    "PRIVATE",
    "CERT",
    "SIGNING",
    "SSH_KEY",
    "GPG_KEY",
    "ENCRYPTION_KEY",
    "MASTER_KEY",
];

impl EnvRow {
    pub fn is_sensitive_key(key: &str) -> bool {
        let upper = key.to_ascii_uppercase();
        SENSITIVE_KEY_PATTERNS
            .iter()
            .any(|needle| upper.contains(needle))
    }

    pub fn display_value(&self, revealed: bool) -> String {
        if self.sensitive && !revealed {
            "•••••••• (press S to reveal)".to_string()
        } else {
            self.value.clone()
        }
    }

    /// Compact masked value without hint suffix (suitable for narrow sidebar cards / tables).
    pub fn masked_value(&self, revealed: bool) -> String {
        if self.sensitive && !revealed {
            mask_secret_value(&self.value)
        } else {
            self.value.clone()
        }
    }
}

/// Produce a fixed-safety masked string that never reveals secret characters.
pub fn mask_secret_value(value: &str) -> String {
    if value.is_empty() {
        "•••• (empty)".to_string()
    } else {
        let len = value.chars().count();
        format!("•••••••• ({len} chars)")
    }
}

/// Structured entry for the PATH Inspector sub-view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathEntry {
    pub path: PathBuf,
    pub display: String,
    pub exists: bool,
    pub is_dir: bool,
    pub is_duplicate: bool,
}

/// Summary counters for the Env panel badge.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EnvStats {
    pub total: usize,
    pub project: usize,
    pub local: usize,
    pub global: usize,
    pub tool_injected: usize,
    pub system: usize,
    pub redacted: usize,
    pub path_count: usize,
    pub missing_paths: usize,
    pub vars_count: usize,
}

/// Aggregated view model for Env panel (`--json-extended` + `--redacted` + `--paths` + `vars`).
#[derive(Debug, Default, Clone)]
pub struct EnvViewModel {
    pub raw_extended: MiseEnvExtendedMap,
    pub redacted_keys: BTreeSet<String>,
    pub paths: Vec<String>,
    pub path_entries: Vec<PathEntry>,
    pub vars: MiseVarsMap,
    pub rows: Vec<EnvRow>,
}

impl EnvViewModel {
    pub fn rebuild(&mut self, workdir: &Path, scope: EnvFilter, filter: &str) {
        let needle = filter.trim();
        let mut scored_rows: Vec<(EnvRow, i32)> = Vec::new();

        for (key, entry) in &self.raw_extended {
            let officially_redacted = self.redacted_keys.contains(key);
            let sensitive = officially_redacted || EnvRow::is_sensitive_key(key);
            let source_kind =
                classify_source(workdir, entry.source.as_deref(), entry.tool.as_deref());

            let matches_scope = match scope {
                EnvFilter::All => true,
                EnvFilter::Project => source_kind == EnvSourceKind::Project,
                EnvFilter::Local => source_kind == EnvSourceKind::Local,
                EnvFilter::Global => source_kind == EnvSourceKind::Global,
                EnvFilter::ToolInjected => source_kind == EnvSourceKind::ToolInjected,
                EnvFilter::Redacted => sensitive,
            };
            if !matches_scope {
                continue;
            }

            let (match_indices, score) = if needle.is_empty() {
                (Vec::new(), 0)
            } else if let Some(m) = fuzzy_match(needle, key) {
                (m.matched_indices, m.score + 40)
            } else if !sensitive && let Some(m) = fuzzy_match(needle, &entry.value) {
                (Vec::new(), m.score + 10)
            } else if let Some(tool) = entry.tool.as_deref().filter(|t| !t.is_empty())
                && let Some(m) = fuzzy_match(needle, tool)
            {
                (Vec::new(), m.score)
            } else {
                continue;
            };

            scored_rows.push((
                EnvRow {
                    key: key.clone(),
                    value: entry.value.clone(),
                    source: entry.source.clone().filter(|s| !s.is_empty()),
                    tool: entry.tool.clone().filter(|s| !s.is_empty()),
                    source_kind,
                    sensitive,
                    officially_redacted,
                    match_indices,
                },
                score,
            ));
        }

        if !needle.is_empty() {
            scored_rows.sort_by(|(a, sa), (b, sb)| sb.cmp(sa).then_with(|| a.key.cmp(&b.key)));
        }

        self.rows = scored_rows.into_iter().map(|(r, _)| r).collect();

        let fallback_path = self.raw_extended.get("PATH").map(|e| e.value.as_str());
        self.path_entries = build_path_entries(&self.paths, fallback_path);
    }

    pub fn redacted_count(&self) -> usize {
        self.raw_extended
            .keys()
            .filter(|k| self.redacted_keys.contains(*k) || EnvRow::is_sensitive_key(k))
            .count()
    }

    pub fn stats(&self, workdir: &Path) -> EnvStats {
        let mut stats = EnvStats {
            path_count: self.path_entries.len(),
            missing_paths: self.path_entries.iter().filter(|p| !p.exists).count(),
            vars_count: self.vars.len(),
            ..Default::default()
        };
        for (key, entry) in &self.raw_extended {
            stats.total += 1;
            if self.redacted_keys.contains(key) || EnvRow::is_sensitive_key(key) {
                stats.redacted += 1;
            }
            match classify_source(workdir, entry.source.as_deref(), entry.tool.as_deref()) {
                EnvSourceKind::Project => stats.project += 1,
                EnvSourceKind::Local => stats.local += 1,
                EnvSourceKind::Global => stats.global += 1,
                EnvSourceKind::ToolInjected => stats.tool_injected += 1,
                EnvSourceKind::System => stats.system += 1,
            }
        }
        stats
    }

    /// Filter `[vars]` template variables using fuzzy search.
    pub fn filtered_vars(&self, filter: &str) -> Vec<(&String, &Value)> {
        let needle = filter.trim();
        if needle.is_empty() {
            return self.vars.iter().collect();
        }
        let mut matched: Vec<((&String, &Value), i32)> = self
            .vars
            .iter()
            .filter_map(|(k, v)| {
                if let Some(m) = fuzzy_match(needle, k) {
                    Some(((k, v), m.score + 20))
                } else {
                    let v_str = v.to_string();
                    fuzzy_match(needle, &v_str).map(|m| ((k, v), m.score))
                }
            })
            .collect();
        matched.sort_by(|((ka, _), sa), ((kb, _), sb)| sb.cmp(sa).then_with(|| ka.cmp(kb)));
        matched.into_iter().map(|(pair, _)| pair).collect()
    }
}

/// Split `mise env --paths` or fallback `PATH` env string using `std::env::split_paths`
/// and annotate existence, directory status, and duplicate entries.
pub fn build_path_entries(raw_paths: &[String], fallback_path_env: Option<&str>) -> Vec<PathEntry> {
    let mut candidate_paths: Vec<PathBuf> = Vec::new();

    if !raw_paths.is_empty() {
        for raw in raw_paths {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            for split in std::env::split_paths(trimmed) {
                if !split.as_os_str().is_empty() {
                    candidate_paths.push(split);
                }
            }
        }
    } else if let Some(path_str) = fallback_path_env {
        for split in std::env::split_paths(path_str) {
            if !split.as_os_str().is_empty() {
                candidate_paths.push(split);
            }
        }
    }

    let mut seen = HashSet::new();
    let mut entries = Vec::with_capacity(candidate_paths.len());
    for path in candidate_paths {
        let is_duplicate = !seen.insert(path.clone());
        let meta = std::fs::metadata(&path);
        let exists = meta.is_ok();
        let is_dir = meta.is_ok_and(|m| m.is_dir());
        let display = path.to_string_lossy().into_owned();
        entries.push(PathEntry {
            path,
            display,
            exists,
            is_dir,
            is_duplicate,
        });
    }
    entries
}

pub fn classify_source(workdir: &Path, source: Option<&str>, tool: Option<&str>) -> EnvSourceKind {
    if tool.is_some_and(|t| !t.trim().is_empty()) {
        return EnvSourceKind::ToolInjected;
    }
    let Some(src) = source.map(str::trim).filter(|s| !s.is_empty()) else {
        return EnvSourceKind::System;
    };

    let src_path = Path::new(src);
    let file_name = src_path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or(src);

    if is_local_source_name(file_name) {
        return EnvSourceKind::Local;
    }
    if !src_path.is_absolute() || path_starts_with_workdir(src_path, workdir) {
        return EnvSourceKind::Project;
    }
    EnvSourceKind::Global
}

fn is_local_source_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains(".local.") || lower.ends_with(".local")
}

fn path_starts_with_workdir(path: &Path, workdir: &Path) -> bool {
    if path.starts_with(workdir) {
        return true;
    }
    normalize_macos_private(path).starts_with(normalize_macos_private(workdir))
}

fn normalize_macos_private(path: &Path) -> std::borrow::Cow<'_, Path> {
    if let Ok(rest) = path.strip_prefix("/private")
        && (rest.starts_with("var") || rest.starts_with("tmp"))
    {
        return std::borrow::Cow::Owned(Path::new("/").join(rest));
    }
    std::borrow::Cow::Borrowed(path)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::mise::schema::EnvExtendedEntry;

    fn sample_env_vm() -> EnvViewModel {
        let mut raw_extended = BTreeMap::new();
        raw_extended.insert(
            "OUT_DIR".to_string(),
            EnvExtendedEntry {
                value: "/repo/outputs".to_string(),
                source: Some("/repo/mise.toml".to_string()),
                tool: None,
            },
        );
        raw_extended.insert(
            "LOCAL_OVERRIDE".to_string(),
            EnvExtendedEntry {
                value: "1".to_string(),
                source: Some("/repo/mise.local.toml".to_string()),
                tool: None,
            },
        );
        raw_extended.insert(
            "GLOBAL_FLAG".to_string(),
            EnvExtendedEntry {
                value: "on".to_string(),
                source: Some("/Users/dev/.config/mise/config.toml".to_string()),
                tool: None,
            },
        );
        raw_extended.insert(
            "CARGO_HOME".to_string(),
            EnvExtendedEntry {
                value: "/Users/dev/.cargo".to_string(),
                source: Some(String::new()),
                tool: Some("rust".to_string()),
            },
        );
        raw_extended.insert(
            "DATABASE_URL".to_string(),
            EnvExtendedEntry {
                value: "postgres://user:supersecret@localhost/db".to_string(),
                source: Some("/repo/.env.local".to_string()),
                tool: None,
            },
        );
        raw_extended.insert(
            "CUSTOM_OFFICIAL_SECRET".to_string(),
            EnvExtendedEntry {
                value: "top-secret-val".to_string(),
                source: Some("/repo/mise.toml".to_string()),
                tool: None,
            },
        );
        raw_extended.insert(
            "PWD".to_string(),
            EnvExtendedEntry {
                value: "/repo".to_string(),
                source: None,
                tool: None,
            },
        );

        let mut redacted_keys = BTreeSet::new();
        redacted_keys.insert("CUSTOM_OFFICIAL_SECRET".to_string());

        let mut vars = BTreeMap::new();
        vars.insert("app_name".to_string(), Value::String("lazymise".to_string()));

        EnvViewModel {
            raw_extended,
            redacted_keys,
            vars,
            ..Default::default()
        }
    }

    #[test]
    fn classifies_all_five_env_sources() {
        let workdir = Path::new("/repo");
        assert_eq!(
            classify_source(workdir, Some("/repo/mise.toml"), None),
            EnvSourceKind::Project
        );
        assert_eq!(
            classify_source(workdir, Some("/repo/mise.local.toml"), None),
            EnvSourceKind::Local
        );
        assert_eq!(
            classify_source(workdir, Some("/repo/.env.local"), None),
            EnvSourceKind::Local
        );
        assert_eq!(
            classify_source(workdir, Some("/Users/dev/.config/mise/config.toml"), None),
            EnvSourceKind::Global
        );
        assert_eq!(
            classify_source(workdir, Some(""), Some("rust")),
            EnvSourceKind::ToolInjected
        );
        assert_eq!(classify_source(workdir, None, None), EnvSourceKind::System);
        // macOS /private/var <-> /var equivalence
        assert_eq!(
            classify_source(
                Path::new("/var/folders/demo"),
                Some("/private/var/folders/demo/mise.toml"),
                None
            ),
            EnvSourceKind::Project
        );
    }

    #[test]
    fn detects_extended_sensitive_keys_and_unions_official_redactions() {
        for key in [
            "JWT_SIGNING",
            "BEARER_HEADER",
            "DATABASE_URL",
            "SENTRY_DSN",
            "SESSION_COOKIE",
            "TLS_PRIVATE_PEM",
            "CLIENT_CERT",
            "APP_SIGNING_SECRET",
            "OPENAI_API_KEY",
            "AWS_ACCESS_KEY_ID",
        ] {
            assert!(EnvRow::is_sensitive_key(key), "expected sensitive: {key}");
        }
        // Standard non-secret keys must not be falsely redacted
        for safe in ["PWD", "OLDPWD", "RUST_LOG", "OUT_DIR", "PATH", "EDITOR"] {
            assert!(!EnvRow::is_sensitive_key(safe), "unexpected sensitive: {safe}");
        }

        let mut vm = sample_env_vm();
        vm.rebuild(Path::new("/repo"), EnvFilter::Redacted, "");
        let keys: Vec<&str> = vm.rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, vec!["CUSTOM_OFFICIAL_SECRET", "DATABASE_URL"]);
        assert!(vm.rows[0].officially_redacted);
        assert!(!vm.rows[1].officially_redacted);

        // Masking hides raw value until revealed
        assert!(!vm.rows[1].display_value(false).contains("supersecret"));
        assert_eq!(
            vm.rows[1].display_value(true),
            "postgres://user:supersecret@localhost/db"
        );

        // Searching by secret value must NOT leak sensitive rows
        vm.rebuild(Path::new("/repo"), EnvFilter::All, "supersecret");
        assert!(vm.rows.is_empty());
    }

    #[test]
    fn splits_paths_and_marks_existence_and_duplicates() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let existing_dir = std::env::temp_dir().join(format!(
            "lazymise-env-path-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&existing_dir).unwrap();
        let missing_dir = existing_dir.join("does-not-exist");

        let joined = std::env::join_paths([&existing_dir, &missing_dir, &existing_dir])
            .unwrap()
            .to_string_lossy()
            .into_owned();

        let entries = build_path_entries(&[], Some(&joined));
        assert_eq!(entries.len(), 3);
        assert!(entries[0].exists && entries[0].is_dir && !entries[0].is_duplicate);
        assert!(!entries[1].exists && !entries[1].is_dir && !entries[1].is_duplicate);
        assert!(entries[2].exists && entries[2].is_duplicate);

        let _ = fs::remove_dir_all(&existing_dir);
    }
}
