use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::de::Deserializer;
use serde::Deserialize;
use serde_json::Value;

/// Task cache configuration: mise may emit either a boolean or a structured object.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TaskCacheConfig {
    Bool(bool),
    Object {
        #[serde(default)]
        enabled: bool,
        #[serde(default)]
        audit: Option<Value>,
        #[serde(default)]
        env: Vec<String>,
        #[serde(default)]
        command_inputs: Vec<Value>,
    },
}

impl TaskCacheConfig {
    pub fn is_enabled(&self) -> bool {
        match self {
            TaskCacheConfig::Bool(b) => *b,
            TaskCacheConfig::Object { enabled, .. } => *enabled,
        }
    }
}

/// Embedded run history summary returned inside `mise tasks ls --json`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TaskHistorySummary {
    pub task: String,
    pub runs: u64,
    pub failures: u64,
    pub median_ms: Option<u64>,
    pub last_ms: Option<u64>,
    pub last_status: Option<String>,
    pub last_started_at: Option<String>,
}

/// Detailed run history item returned by `mise tasks history --json`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TaskHistoryEntry {
    pub task: String,
    pub args: Vec<String>,
    pub started_at: String,
    pub duration_ms: u64,
    pub status: String,
    pub exit_code: Option<i32>,
}

/// Parsed `#USAGE` / `usage_spec` returned by `mise tasks info <task> --json`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TaskUsageSpec {
    pub name: String,
    pub bin: String,
    pub cmd: UsageCmd,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UsageCmd {
    pub args: Vec<UsageArg>,
    pub flags: Vec<UsageFlag>,
    pub help: Option<String>,
    pub examples: Vec<Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UsageArg {
    pub name: String,
    pub usage: String,
    pub help: Option<String>,
    pub required: bool,
    pub var: bool,
    #[serde(default, deserialize_with = "deserialize_opt_string")]
    pub default: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UsageFlag {
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_string_vec")]
    pub short: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_string_vec")]
    pub long: Vec<String>,
    pub usage: String,
    pub help: Option<String>,
    pub required: bool,
    pub arg: Option<Box<UsageArg>>,
    #[serde(default, deserialize_with = "deserialize_opt_string")]
    pub default: Option<String>,
}

impl TaskUsageSpec {
    pub fn has_required_inputs(&self) -> bool {
        self.cmd.args.iter().any(|a| a.required) || self.cmd.flags.iter().any(|f| f.required)
    }

    pub fn has_any_inputs(&self) -> bool {
        !self.cmd.args.is_empty() || !self.cmd.flags.is_empty()
    }
}

/// Full task definition from `mise tasks ls --json` / `mise tasks info <t> --json`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MiseTask {
    pub name: String,
    pub aliases: Vec<String>,
    pub tags: Vec<String>,
    pub description: String,
    pub source: String,
    pub config_sources: Vec<String>,
    pub depends: Vec<Value>,
    pub depends_post: Vec<Value>,
    pub wait_for: Vec<Value>,
    pub daemons: Option<Value>,
    pub secrets: Vec<String>,
    pub env: Value,
    pub dir: Option<String>,
    pub hide: bool,
    pub global: bool,
    pub raw: bool,
    pub interactive: bool,
    pub sources: Vec<String>,
    pub outputs: Value,
    pub cache: Option<TaskCacheConfig>,
    pub shell: Option<String>,
    pub skip_if: Option<Value>,
    pub lock: Option<Value>,
    pub quiet: bool,
    pub silent: Value,
    #[serde(default, deserialize_with = "deserialize_string_map")]
    pub tools: BTreeMap<String, String>,
    pub usage: String,
    pub timeout: Option<Value>,
    #[serde(default, deserialize_with = "deserialize_run_commands")]
    pub run: Vec<String>,
    pub args: Vec<Value>,
    pub file: Option<String>,
    pub history: Option<TaskHistorySummary>,
    pub usage_spec: Option<TaskUsageSpec>,
}

impl MiseTask {
    pub fn depends_display(&self) -> Vec<String> {
        self.depends.iter().map(format_dep_value).collect()
    }

    pub fn depends_post_display(&self) -> Vec<String> {
        self.depends_post.iter().map(format_dep_value).collect()
    }

    pub fn wait_for_display(&self) -> Vec<String> {
        self.wait_for.iter().map(format_dep_value).collect()
    }

    pub fn outputs_display(&self) -> Vec<String> {
        match &self.outputs {
            Value::Array(items) => items
                .iter()
                .filter_map(|v| match v {
                    Value::String(s) if !s.is_empty() => Some(s.clone()),
                    Value::Null => None,
                    other => Some(other.to_string()),
                })
                .collect(),
            Value::Object(map) => map
                .iter()
                .map(|(k, v)| match v {
                    Value::String(s) => format!("{k}: {s}"),
                    other => format!("{k}: {other}"),
                })
                .collect(),
            Value::String(s) if !s.is_empty() => vec![s.clone()],
            _ => Vec::new(),
        }
    }

    pub fn cache_enabled(&self) -> bool {
        self.cache.as_ref().is_some_and(TaskCacheConfig::is_enabled)
    }

    pub fn is_silent(&self) -> bool {
        match &self.silent {
            Value::Bool(b) => *b,
            Value::String(s) => !s.is_empty() && !s.eq_ignore_ascii_case("false"),
            _ => false,
        }
    }

    pub fn timeout_display(&self) -> Option<String> {
        match self.timeout.as_ref()? {
            Value::Null => None,
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(format!("{n}s")),
            other => Some(other.to_string()),
        }
    }

    pub fn has_daemons(&self) -> bool {
        match self.daemons.as_ref() {
            None | Some(Value::Null) => false,
            Some(Value::Array(a)) => !a.is_empty(),
            Some(Value::Object(m)) => !m.is_empty(),
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => !s.is_empty(),
            Some(_) => true,
        }
    }

    /// Best editable file path for this task, skipping empty `file` strings.
    pub fn editable_path(&self) -> Option<&str> {
        self.file
            .as_deref()
            .filter(|p| !p.is_empty())
            .or_else(|| (!self.source.is_empty()).then_some(self.source.as_str()))
    }

    /// Derive grouping namespace for tree display:
    /// - global tasks -> "[global]"
    /// - monorepo tasks `//pkg/sub:name` -> "//pkg/sub"
    /// - namespaced tasks `exp:01-baseline` -> "exp"
    /// - root tasks -> "[root]"
    pub fn group_key(&self) -> String {
        if self.global {
            return "[global]".to_string();
        }
        if let Some((prefix, _)) = self.name.rsplit_once(':') {
            return prefix.to_string();
        }
        "[root]".to_string()
    }

    pub fn short_name(&self) -> &str {
        if let Some((_, leaf)) = self.name.rsplit_once(':') {
            leaf
        } else {
            &self.name
        }
    }
}

fn format_dep_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(arr) => arr
            .iter()
            .filter_map(|item| match item {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                Value::Bool(b) => Some(b.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" "),
        Value::Object(map) => {
            if let Some(task) = map.get("task").and_then(Value::as_str) {
                if let Some(args) = map.get("args").and_then(Value::as_array) {
                    let arg_str = args
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" ");
                    if !arg_str.is_empty() {
                        return format!("{task} {arg_str}");
                    }
                }
                return task.to_string();
            }
            v.to_string()
        }
        other => other.to_string(),
    }
}

fn deserialize_run_commands<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let val = Option::<Value>::deserialize(deserializer)?;
    let Some(val) = val else {
        return Ok(Vec::new());
    };
    match val {
        Value::Null => Ok(Vec::new()),
        Value::String(s) => {
            if s.is_empty() {
                Ok(Vec::new())
            } else {
                Ok(vec![s])
            }
        }
        Value::Array(items) => Ok(items
            .into_iter()
            .filter_map(|item| match item {
                Value::Null => None,
                Value::String(s) => Some(s),
                Value::Object(map) => {
                    if let Some(task) = map.get("task").and_then(Value::as_str) {
                        Some(format!("task: {task}"))
                    } else if let Some(tasks) = map.get("tasks").and_then(Value::as_array) {
                        let joined = tasks
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ");
                        Some(format!("tasks: [{joined}]"))
                    } else if let Some(script) = map.get("script").and_then(Value::as_str) {
                        Some(script.to_string())
                    } else {
                        Some(Value::Object(map).to_string())
                    }
                }
                other => Some(other.to_string()),
            })
            .collect()),
        other => Ok(vec![other.to_string()]),
    }
}

fn deserialize_string_map<'de, D>(
    deserializer: D,
) -> std::result::Result<BTreeMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    let val = Option::<Value>::deserialize(deserializer)?;
    let Some(Value::Object(map)) = val else {
        return Ok(BTreeMap::new());
    };
    let mut out = BTreeMap::new();
    for (k, v) in map {
        let s = match v {
            Value::String(s) => s,
            Value::Array(arr) => arr
                .into_iter()
                .map(|x| match x {
                    Value::String(s) => s,
                    other => other.to_string(),
                })
                .collect::<Vec<_>>()
                .join(", "),
            Value::Object(obj) => obj
                .get("version")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| Value::Object(obj).to_string()),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        out.insert(k, s);
    }
    Ok(out)
}

fn deserialize_opt_string<'de, D>(deserializer: D) -> std::result::Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let val = Option::<Value>::deserialize(deserializer)?;
    Ok(match val {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s),
        Some(Value::Array(arr)) => {
            let joined = arr
                .into_iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s),
                    Value::Null => None,
                    other => Some(other.to_string()),
                })
                .collect::<Vec<_>>()
                .join(", ");
            if joined.is_empty() {
                None
            } else {
                Some(joined)
            }
        }
        Some(other) => Some(other.to_string()),
    })
}

fn deserialize_string_vec<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let val = Option::<Value>::deserialize(deserializer)?;
    Ok(match val {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(s)) => vec![s],
        Some(Value::Array(arr)) => arr
            .into_iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(s),
                Value::Null => None,
                other => Some(other.to_string()),
            })
            .collect(),
        Some(other) => vec![other.to_string()],
    })
}

/// Tool config source from `mise ls --json` / `mise outdated --json` / `mise tool --json`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MiseToolSource {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub path: Option<String>,
}

/// Tool version entry from `mise ls --json`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MiseToolEntry {
    pub version: String,
    pub requested_version: Option<String>,
    pub install_path: Option<String>,
    pub source: Option<MiseToolSource>,
    pub installed: bool,
    pub active: bool,
    pub scheduled_for_pruning: bool,
    pub prune_after: Option<String>,
}

pub type MiseToolsMap = BTreeMap<String, Vec<MiseToolEntry>>;

/// Single tool deep metadata from `mise tool <name> --json`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ToolDetail {
    pub backend: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
    pub installed_versions: Vec<String>,
    pub requested_versions: Vec<String>,
    pub active_versions: Vec<String>,
    pub config_source: Option<MiseToolSource>,
    pub tool_options: Option<Value>,
    pub security: Vec<Value>,
}

/// Outdated tool entry from `mise outdated --json`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct OutdatedEntry {
    pub name: String,
    pub requested: Option<String>,
    pub current: Option<String>,
    pub bump: Option<String>,
    pub latest: String,
    pub release_url: Option<String>,
    pub source: Option<MiseToolSource>,
}

pub type OutdatedMap = BTreeMap<String, OutdatedEntry>;

/// Vulnerability item inside `mise audit --json`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Vulnerability {
    pub tool: String,
    pub version: String,
    pub id: String,
    pub aliases: Vec<String>,
    pub severity: Option<String>,
    pub summary: String,
    pub fixed: Vec<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct UncheckedTool {
    pub tool: String,
    pub version: String,
}

/// Security audit report from `mise audit --json` (NOTE: exit code may be 1 on findings!).
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AuditReport {
    pub vulnerabilities: Vec<Vulnerability>,
    pub unchecked: Vec<UncheckedTool>,
}

/// Extended env var entry from `mise env --json-extended`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct EnvExtendedEntry {
    pub value: String,
    pub source: Option<String>,
    pub tool: Option<String>,
}

pub type MiseEnvMap = BTreeMap<String, String>;
pub type MiseEnvExtendedMap = BTreeMap<String, EnvExtendedEntry>;
pub type MiseVarsMap = BTreeMap<String, Value>;

/// Config file entry from `mise config ls --json`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ConfigFileEntry {
    pub path: String,
    pub tools: Vec<String>,
    #[serde(rename = "override")]
    pub is_override: bool,
}

/// Config trust status parsed from `mise trust --show`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustEntry {
    pub path: String,
    pub trusted: bool,
    pub raw_status: String,
}

/// Config schema explanation item from `mise config explain --json`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ConfigExplainItem {
    pub key: String,
    pub path: Vec<String>,
    pub line: u32,
    pub description: String,
    pub deprecated: bool,
    pub known: bool,
    pub suggestion: Option<String>,
}

/// Shell sub-object inside `mise doctor --json`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DoctorShell {
    pub name: String,
    pub version: String,
}

/// Health report from `mise doctor --json` (NOTE: exit code may be 1 when errors exist!).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DoctorReport {
    pub activated: bool,
    pub version: String,
    pub build_info: Value,
    pub shell: Option<DoctorShell>,
    pub dirs: BTreeMap<String, Value>,
    pub config_files: Vec<String>,
    pub ignored_config_files: Vec<String>,
    pub env_files: Vec<String>,
    pub env_vars: Value,
    pub paths: Vec<String>,
    pub shims_on_path: bool,
    pub self_update_available: bool,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

/// Flattened setting entry derived from `mise settings ls --json-extended`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SettingEntry {
    pub key: String,
    pub value: Value,
    pub kind: String,
    pub description: String,
    pub source: Option<String>,
}

/// Registry entry from `mise registry --json`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RegistryEntry {
    pub short: String,
    pub backends: Vec<String>,
    pub bins: Vec<String>,
    pub description: Option<String>,
    pub aliases: Vec<String>,
}

/// Experimental daemon entry from `mise daemons --json`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DaemonEntry {
    pub name: String,
    pub status: String,
    pub pid: Option<u32>,
    pub url: Option<String>,
    pub command: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Experimental project dependency entry parsed from `mise deps --list`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DepEntry {
    pub name: String,
    pub status: String,
    pub detail: String,
}

/// Parse `mise trust --show` plain-text output (e.g. `~/MyCli/lazymise: trusted`).
pub fn parse_trust_show(raw: &str) -> Vec<TrustEntry> {
    raw.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            let (path, status) = trimmed.rsplit_once(':')?;
            let status_trim = status.trim();
            if path.trim().is_empty() || status_trim.is_empty() {
                return None;
            }
            Some(TrustEntry {
                path: path.trim().to_string(),
                trusted: status_trim.eq_ignore_ascii_case("trusted"),
                raw_status: status_trim.to_string(),
            })
        })
        .collect()
}

/// Parse `mise tasks deps <task>` plain-text tree output, preserving tree indentation.
pub fn parse_task_deps(raw: &str) -> Vec<String> {
    let mut lines: Vec<String> = raw
        .lines()
        .map(|l| l.trim_end_matches(['\r', ' ', '\t']).to_string())
        .collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

/// Parse `mise env --paths` newline-delimited PATH entries.
pub fn parse_env_paths(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Parse `mise deps --list` plain-text output; silently returns empty when unconfigured.
pub fn parse_deps_list(raw: &str) -> Vec<DepEntry> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("no deps providers found")
            || lower.starts_with("no providers found")
            || lower.starts_with("provider ")
            || lower == "provider"
            || lower.starts_with("name ")
            || trimmed.chars().all(|c| matches!(c, '-' | '=' | '─' | ' '))
        {
            continue;
        }

        // Handle multi-column table lines separated by 2+ spaces or tabs.
        let cols: Vec<&str> = split_columns(trimmed);
        if cols.len() >= 2 {
            let name = cols[0].to_string();
            let status = cols[1].to_string();
            let detail = if cols.len() > 2 {
                cols[2..].join("  ")
            } else {
                String::new()
            };
            out.push(DepEntry {
                name,
                status,
                detail,
            });
            continue;
        }

        // Fallback: `name: status` or single-token provider name.
        if let Some((name, rest)) = trimmed.split_once(':') {
            out.push(DepEntry {
                name: name.trim().to_string(),
                status: rest.trim().to_string(),
                detail: String::new(),
            });
        } else {
            out.push(DepEntry {
                name: trimmed.to_string(),
                status: "configured".to_string(),
                detail: String::new(),
            });
        }
    }
    out
}

fn split_columns(line: &str) -> Vec<&str> {
    let mut cols = Vec::new();
    let mut start = None;
    let mut space_run = 0usize;
    let bytes = line.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\t' {
            if let Some(s) = start.take() {
                let piece = line[s..i].trim();
                if !piece.is_empty() {
                    cols.push(piece);
                }
            }
            space_run = 0;
        } else if b == b' ' {
            space_run += 1;
            if space_run >= 2
                && let Some(s) = start.take()
            {
                let piece = line[s..i - space_run + 1].trim();
                if !piece.is_empty() {
                    cols.push(piece);
                }
            }
        } else {
            if start.is_none() {
                start = Some(i);
            }
            space_run = 0;
        }
        i += 1;
    }

    if let Some(s) = start {
        let piece = line[s..].trim();
        if !piece.is_empty() {
            cols.push(piece);
        }
    }
    cols
}

/// Parse `mise daemons --json` across array, `{ "daemons": [...] }`, or map-by-name shapes.
pub fn parse_daemons_json(raw: &str) -> Vec<DaemonEntry> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
        return Vec::new();
    };
    match val {
        Value::Array(arr) => arr
            .into_iter()
            .filter_map(|v| serde_json::from_value::<DaemonEntry>(v).ok())
            .collect(),
        Value::Object(mut map) => {
            if let Some(Value::Array(arr)) = map.remove("daemons") {
                return arr
                    .into_iter()
                    .filter_map(|v| serde_json::from_value::<DaemonEntry>(v).ok())
                    .collect();
            }
            let mut entries = Vec::new();
            for (name, v) in map {
                if let Ok(mut entry) = serde_json::from_value::<DaemonEntry>(v) {
                    if entry.name.is_empty() {
                        entry.name = name;
                    }
                    entries.push(entry);
                }
            }
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            entries
        }
        _ => Vec::new(),
    }
}

/// Parse `mise config --tracked-configs --json` (supports both JSON array and plain-text lines).
pub fn parse_tracked_configs(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.starts_with('[')
        && let Ok(list) = serde_json::from_str::<Vec<String>>(trimmed)
    {
        return list;
    }
    trimmed
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Flatten nested `mise settings ls --json-extended` objects into dot-separated `SettingEntry` items.
pub fn flatten_settings(prefix: &str, val: &Value, out: &mut Vec<SettingEntry>) {
    let Some(map) = val.as_object() else {
        return;
    };
    let is_leaf = !prefix.is_empty()
        && map.contains_key("value")
        && map.get("type").is_some_and(Value::is_string)
        && (map.contains_key("description") || map.contains_key("source") || map.len() <= 4);

    if is_leaf {
        out.push(SettingEntry {
            key: prefix.to_string(),
            value: map.get("value").cloned().unwrap_or(Value::Null),
            kind: map
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            description: map
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            source: map
                .get("source")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        });
        return;
    }

    for (k, v) in map {
        let next = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        flatten_settings(&next, v, out);
    }
}

/// Parse and flatten `mise settings ls --json-extended` output sorted by key.
pub fn parse_settings_extended_json(raw: &str) -> Result<Vec<SettingEntry>> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let val: Value =
        serde_json::from_str(raw).context("failed to parse `mise settings ls --json-extended`")?;
    let mut entries = Vec::new();
    flatten_settings("", &val, &mut entries);
    entries.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(entries)
}
