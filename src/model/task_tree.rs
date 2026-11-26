use std::collections::{BTreeMap, BTreeSet};

use crate::mise::schema::MiseTask;

#[derive(Debug, Clone)]
pub enum TaskRow {
    Group {
        key: String,
        count: usize,
        collapsed: bool,
    },
    Task(Box<MiseTask>),
}

impl TaskRow {
    pub fn as_task(&self) -> Option<&MiseTask> {
        match self {
            TaskRow::Task(t) => Some(t),
            TaskRow::Group { .. } => None,
        }
    }

    pub fn group_key(&self) -> Option<&str> {
        match self {
            TaskRow::Group { key, .. } => Some(key.as_str()),
            TaskRow::Task(_) => None,
        }
    }
}

/// Build flattened rows for the Tasks tree:
/// - `[root]` first, then alphabetical project/monorepo namespaces, then `[global]` last.
/// - When `filter` is non-empty, groups auto-expand and only matching tasks are shown.
pub fn build_task_rows(
    tasks: &[MiseTask],
    collapsed: &BTreeSet<String>,
    filter: &str,
) -> Vec<TaskRow> {
    let needle = filter.trim().to_ascii_lowercase();
    let mut grouped: BTreeMap<String, Vec<MiseTask>> = BTreeMap::new();

    for task in tasks {
        if !needle.is_empty() {
            let hay = format!(
                "{} {} {}",
                task.name.to_ascii_lowercase(),
                task.description.to_ascii_lowercase(),
                task.aliases.join(" ").to_ascii_lowercase()
            );
            if !hay.contains(&needle) {
                continue;
            }
        }
        grouped
            .entry(task.group_key())
            .or_default()
            .push(task.clone());
    }

    // Order groups: [root] -> normal namespaces -> [global]
    let mut ordered_keys: Vec<String> = grouped.keys().cloned().collect();
    ordered_keys.sort_by(|a, b| group_rank(a).cmp(&group_rank(b)).then_with(|| a.cmp(b)));

    let mut rows = Vec::new();
    for key in ordered_keys {
        if let Some(mut items) = grouped.remove(&key) {
            items.sort_by(|a, b| a.name.cmp(&b.name));
            let is_collapsed = needle.is_empty() && collapsed.contains(&key);
            rows.push(TaskRow::Group {
                key: key.clone(),
                count: items.len(),
                collapsed: is_collapsed,
            });
            if !is_collapsed {
                for item in items {
                    rows.push(TaskRow::Task(Box::new(item)));
                }
            }
        }
    }
    rows
}

fn group_rank(key: &str) -> u8 {
    match key {
        "[root]" => 0,
        "[global]" => 2,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn mk(name: &str, global: bool) -> MiseTask {
        MiseTask {
            name: name.to_string(),
            global,
            ..Default::default()
        }
    }

    #[test]
    fn orders_root_then_namespaces_then_global() {
        let tasks = vec![
            mk("info", true),
            mk("exp:01-baseline", false),
            mk("check", false),
            mk("//packages/core:test", false),
        ];
        let mut collapsed = BTreeSet::new();
        collapsed.insert("[global]".to_string());

        let rows = build_task_rows(&tasks, &collapsed, "");
        let groups: Vec<&str> = rows.iter().filter_map(TaskRow::group_key).collect();
        assert_eq!(groups, vec!["[root]", "//packages/core", "exp", "[global]"]);
        // [global] is collapsed so `info` is hidden
        assert!(rows.iter().filter_map(TaskRow::as_task).all(|t| !t.global));

        // Filtering expands collapsed groups
        let filtered = build_task_rows(&tasks, &collapsed, "info");
        assert!(filtered.iter().filter_map(TaskRow::as_task).any(|t| t.name == "info"));
    }
}
