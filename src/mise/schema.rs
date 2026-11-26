use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MiseTask {
    pub name: String,
    pub aliases: Vec<String>,
    pub description: String,
    pub source: String,
    pub depends: Vec<Value>,
    pub depends_post: Vec<Value>,
    pub dir: Option<String>,
    pub hide: bool,
    pub global: bool,
    pub sources: Vec<String>,
    pub outputs: Value,
    pub run: Vec<String>,
    pub file: Option<String>,
}

impl MiseTask {
    pub fn depends_display(&self) -> Vec<String> {
        self.depends.iter().map(format_dep_value).collect()
    }

    pub fn outputs_display(&self) -> Vec<String> {
        match &self.outputs {
            Value::Array(items) => items
                .iter()
                .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                .collect(),
            Value::Object(map) => map
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect(),
            Value::String(s) if !s.is_empty() => vec![s.clone()],
            _ => Vec::new(),
        }
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
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        other => other.to_string(),
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MiseToolSource {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MiseToolEntry {
    pub version: String,
    pub requested_version: Option<String>,
    pub install_path: Option<String>,
    pub source: Option<MiseToolSource>,
    pub installed: bool,
    pub active: bool,
    pub scheduled_for_pruning: bool,
}

pub type MiseToolsMap = BTreeMap<String, Vec<MiseToolEntry>>;
pub type MiseEnvMap = BTreeMap<String, String>;
