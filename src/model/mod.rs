pub mod task_tree;
pub mod template;

use crate::mise::schema::MiseToolEntry;

#[derive(Debug, Clone)]
pub struct ToolRow {
    pub name: String,
    pub entry: MiseToolEntry,
}

#[derive(Debug, Clone)]
pub struct EnvRow {
    pub key: String,
    pub value: String,
    pub sensitive: bool,
}

impl EnvRow {
    pub fn is_sensitive_key(key: &str) -> bool {
        let upper = key.to_ascii_uppercase();
        [
            "SECRET",
            "TOKEN",
            "PASSWORD",
            "PASSWD",
            "API_KEY",
            "PRIVATE_KEY",
            "ACCESS_KEY",
            "CREDENTIAL",
            "AUTH",
        ]
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
}
