use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::mise::schema::MiseTask;
use crate::model::fuzzy::fuzzy_match;

/// Default maximum bytes read per cached file (256 KiB) to protect UI memory & responsiveness.
pub const DEFAULT_MAX_FILE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub enum TaskRow {
    Group {
        key: String,
        count: usize,
        collapsed: bool,
    },
    Task {
        task: Box<MiseTask>,
        match_indices: Vec<usize>,
    },
}

impl TaskRow {
    pub fn as_task(&self) -> Option<&MiseTask> {
        match self {
            TaskRow::Task { task, .. } => Some(task),
            TaskRow::Group { .. } => None,
        }
    }

    pub fn group_key(&self) -> Option<&str> {
        match self {
            TaskRow::Group { key, .. } => Some(key.as_str()),
            TaskRow::Task { .. } => None,
        }
    }

    pub fn is_monorepo_group(&self) -> bool {
        self.group_key().is_some_and(is_monorepo_group_key)
    }
}

/// Returns true if a group key represents a mise monorepo path group (`//` or `//pkg/sub`).
pub fn is_monorepo_group_key(key: &str) -> bool {
    key.starts_with("//")
}

/// Returns true if a task belongs to a monorepo subpackage (`//...`).
pub fn is_monorepo_task(task: &MiseTask) -> bool {
    task.name.starts_with("//")
}

/// Derive canonical tree group key for a task:
/// - `global == true` -> `"[global]"`
/// - monorepo `//pkg/sub:build` or `//pkg/sub:ns:build` -> `"//pkg/sub"` (or `"//"` for `//:build`)
/// - namespaced `exp:01-baseline` -> `"exp"`
/// - root task `check` -> `"[root]"`
pub fn task_group_key(task: &MiseTask) -> String {
    if task.global {
        return "[global]".to_string();
    }
    if task.name.starts_with("//") {
        if let Some((pkg, _)) = task.name.split_once(':') {
            return pkg.to_string();
        }
        return task.name.clone();
    }
    task.group_key()
}

/// Leaf display name for a task inside its tree group.
pub fn task_leaf_name(task: &MiseTask) -> &str {
    if task.name.starts_with("//") {
        if let Some((_, leaf)) = task.name.split_once(':') {
            return leaf;
        }
        return &task.name;
    }
    task.short_name()
}

#[derive(Debug, Clone)]
struct CachedFile {
    content: String,
    mtime: Option<SystemTime>,
    truncated: bool,
}

/// Caches task script and config file contents in memory (keyed by path + mtime with size cap)
/// so UI rendering NEVER performs disk I/O.
#[derive(Debug, Clone)]
pub struct ScriptCache {
    entries: HashMap<PathBuf, CachedFile>,
    max_bytes: usize,
}

impl Default for ScriptCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptCache {
    pub fn new() -> Self {
        Self::with_max_bytes(DEFAULT_MAX_FILE_BYTES)
    }

    pub fn with_max_bytes(max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_bytes: max_bytes.max(1),
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn invalidate(&mut self, path: &Path) {
        self.entries.remove(path);
    }

    /// Fetch from memory cache strictly without disk I/O (safe to call during render).
    pub fn get_cached(&self, path: &Path) -> Option<&str> {
        self.entries.get(path).map(|e| e.content.as_str())
    }

    /// Returns true if the cached file was truncated due to the size cap.
    pub fn is_truncated(&self, path: &Path) -> bool {
        self.entries.get(path).is_some_and(|e| e.truncated)
    }

    /// Explicitly load (or reload if file mtime changed) a file path into the cache.
    /// Call on selection change / data reload, NEVER inside `ui::render`.
    pub fn preload(&mut self, path: PathBuf) -> Option<&str> {
        let meta = match fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => {
                self.entries.remove(&path);
                return None;
            }
        };
        let mtime = meta.modified().ok();

        let is_fresh = self
            .entries
            .get(&path)
            .is_some_and(|cached| cached.mtime.is_some() && cached.mtime == mtime);

        if !is_fresh {
            match read_bounded_utf8(&path, self.max_bytes) {
                Ok((content, truncated)) => {
                    self.entries.insert(
                        path.clone(),
                        CachedFile {
                            content,
                            mtime,
                            truncated,
                        },
                    );
                }
                Err(_) => {
                    self.entries.remove(&path);
                    return None;
                }
            }
        }

        self.entries.get(&path).map(|e| e.content.as_str())
    }

    /// Preload the file-based script for `task` if it is an external script (non-TOML).
    pub fn preload_task(&mut self, task: &MiseTask) -> Option<&str> {
        if !task.run.is_empty() {
            return None;
        }
        let path = task.editable_path()?;
        if path.ends_with(".toml") {
            return None;
        }
        self.preload(PathBuf::from(path))
    }

    /// Resolve task script text strictly from inline `run` or already-cached file content (zero render I/O).
    pub fn task_script_readonly(&self, task: &MiseTask) -> String {
        if !task.run.is_empty() {
            return task.run.join("\n");
        }
        if let Some(path) = task.editable_path()
            && !path.ends_with(".toml")
            && let Some(cached) = self.get_cached(Path::new(path))
        {
            return cached.to_string();
        }
        "(inline or external task definition)".to_string()
    }
}

fn read_bounded_utf8(path: &Path, max_bytes: usize) -> std::io::Result<(String, bool)> {
    let file = File::open(path)?;
    let mut buf = Vec::with_capacity(max_bytes.min(8192));
    file.take((max_bytes as u64).saturating_add(1))
        .read_to_end(&mut buf)?;

    if buf.len() > max_bytes {
        buf.truncate(max_bytes);
        // Trim any incomplete trailing UTF-8 sequence split at the byte boundary.
        while !buf.is_empty() && std::str::from_utf8(&buf).is_err() {
            if let Err(err) = std::str::from_utf8(&buf) {
                if err.error_len().is_none() {
                    buf.truncate(err.valid_up_to());
                    break;
                }
                // Genuine invalid UTF-8 earlier in file: fall back to lossy conversion.
                break;
            }
        }
        let mut text = String::from_utf8_lossy(&buf).into_owned();
        let limit_label = if max_bytes >= 1024 && max_bytes.is_multiple_of(1024) {
            format!("{} KB", max_bytes / 1024)
        } else {
            format!("{max_bytes} B")
        };
        text.push_str(&format!("\n... (truncated at {limit_label})"));
        Ok((text, true))
    } else {
        Ok((String::from_utf8_lossy(&buf).into_owned(), false))
    }
}

/// Stateful task tree controller preserving collapsed group state across reloads.
#[derive(Debug, Clone)]
pub struct TaskTree {
    pub collapsed: HashSet<String>,
    pub show_hidden: bool,
    pub monorepo_all: bool,
    pub rows: Vec<TaskRow>,
}

impl Default for TaskTree {
    fn default() -> Self {
        let mut collapsed = HashSet::new();
        collapsed.insert("[global]".to_string());
        Self {
            collapsed,
            show_hidden: false,
            monorepo_all: false,
            rows: Vec::new(),
        }
    }
}

impl TaskTree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_collapsed(&self, group_key: &str) -> bool {
        self.collapsed.contains(group_key)
    }

    pub fn toggle_group(&mut self, group_key: &str) -> bool {
        if self.collapsed.contains(group_key) {
            self.collapsed.remove(group_key);
            false
        } else {
            self.collapsed.insert(group_key.to_string());
            true
        }
    }

    pub fn set_collapsed(&mut self, group_key: impl Into<String>, collapsed: bool) {
        let key = group_key.into();
        if collapsed {
            self.collapsed.insert(key);
        } else {
            self.collapsed.remove(&key);
        }
    }

    /// Rebuild `self.rows` from `tasks` while preserving `self.collapsed` across reloads.
    pub fn rebuild(&mut self, tasks: &[MiseTask], filter: &str) {
        self.rows = build_task_rows_impl(
            tasks,
            |key| self.collapsed.contains(key),
            filter,
            self.show_hidden,
            self.monorepo_all,
        );
    }

    /// Returns true if any currently loaded task belongs to a monorepo path group (`//...`).
    pub fn has_monorepo_groups(&self) -> bool {
        self.rows.iter().any(TaskRow::is_monorepo_group)
    }
}

/// Build flattened rows for the Tasks tree:
/// - `[root]` first, then alphabetical workspace namespaces, then `//pkg/sub` monorepo groups,
///   then `[global]` last.
/// - Respects `show_hidden` and fuzzy `filter` (auto-expanding groups when filtering).
pub fn build_task_rows(
    tasks: &[MiseTask],
    collapsed: &BTreeSet<String>,
    filter: &str,
    show_hidden: bool,
) -> Vec<TaskRow> {
    build_task_rows_impl(tasks, |key| collapsed.contains(key), filter, show_hidden, true)
}

fn build_task_rows_impl<F>(
    tasks: &[MiseTask],
    is_group_collapsed: F,
    filter: &str,
    show_hidden: bool,
    include_monorepo_subpackages: bool,
) -> Vec<TaskRow>
where
    F: Fn(&str) -> bool,
{
    let needle = filter.trim();
    let mut grouped: BTreeMap<String, Vec<(MiseTask, Vec<usize>, i32)>> = BTreeMap::new();

    for task in tasks {
        if task.hide && !show_hidden {
            continue;
        }
        if !include_monorepo_subpackages && is_monorepo_task(task) {
            continue;
        }
        let leaf = task_leaf_name(task);
        let (indices, score) = if needle.is_empty() {
            (Vec::new(), 0)
        } else if let Some(m) = fuzzy_match(needle, leaf) {
            (m.matched_indices, m.score + 40)
        } else if let Some(m) = fuzzy_match(needle, &task.name) {
            let projected = project_leaf_indices(&task.name, leaf, &m.matched_indices);
            (projected, m.score + 20)
        } else {
            let hay = format!(
                "{} {} {}",
                task.description,
                task.aliases.join(" "),
                task.tags.join(" ")
            );
            if let Some(m) = fuzzy_match(needle, &hay) {
                (Vec::new(), m.score)
            } else {
                continue;
            }
        };
        grouped
            .entry(task_group_key(task))
            .or_default()
            .push((task.clone(), indices, score));
    }

    let mut ordered_keys: Vec<String> = grouped.keys().cloned().collect();
    ordered_keys.sort_by(|a, b| group_rank(a).cmp(&group_rank(b)).then_with(|| a.cmp(b)));

    let mut rows = Vec::new();
    for key in ordered_keys {
        if let Some(mut items) = grouped.remove(&key) {
            if needle.is_empty() {
                items.sort_by(|a, b| a.0.name.cmp(&b.0.name));
            } else {
                items.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.name.cmp(&b.0.name)));
            }
            let is_collapsed = needle.is_empty() && is_group_collapsed(&key);
            rows.push(TaskRow::Group {
                key: key.clone(),
                count: items.len(),
                collapsed: is_collapsed,
            });
            if !is_collapsed {
                for (task, match_indices, _) in items {
                    rows.push(TaskRow::Task {
                        task: Box::new(task),
                        match_indices,
                    });
                }
            }
        }
    }
    rows
}

/// Project full-name character match indices onto the leaf name suffix for UI highlighting.
fn project_leaf_indices(full_name: &str, leaf: &str, full_indices: &[usize]) -> Vec<usize> {
    let full_chars = full_name.chars().count();
    let leaf_chars = leaf.chars().count();
    let offset = full_chars.saturating_sub(leaf_chars);
    full_indices
        .iter()
        .filter_map(|&idx| idx.checked_sub(offset))
        .collect()
}

fn group_rank(key: &str) -> u8 {
    if key == "[root]" {
        0
    } else if key == "[global]" {
        3
    } else if is_monorepo_group_key(key) {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use pretty_assertions::assert_eq;

    use super::*;

    fn mk_task(name: &str, hide: bool, global: bool) -> MiseTask {
        MiseTask {
            name: name.to_string(),
            hide,
            global,
            ..Default::default()
        }
    }

    fn temp_test_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "lazymise-task-tree-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn groups_root_namespaces_monorepo_and_global_in_order() {
        let tasks = vec![
            mk_task("global-lint", false, true),
            mk_task("//packages/web:build", false, false),
            mk_task("ci:test", false, false),
            mk_task("check", false, false),
            mk_task("//packages/api:db:migrate", false, false),
        ];

        let rows = build_task_rows(&tasks, &BTreeSet::new(), "", false);
        let group_keys: Vec<&str> = rows.iter().filter_map(TaskRow::group_key).collect();
        assert_eq!(
            group_keys,
            vec!["[root]", "ci", "//packages/api", "//packages/web", "[global]"]
        );

        let monorepo_flags: Vec<bool> = rows
            .iter()
            .filter(|r| r.group_key().is_some())
            .map(TaskRow::is_monorepo_group)
            .collect();
        assert_eq!(monorepo_flags, vec![false, false, true, true, false]);
    }

    #[test]
    fn show_hidden_and_monorepo_all_toggles_work() {
        let tasks = vec![
            mk_task("check", false, false),
            mk_task("_internal", true, false),
            mk_task("//pkg/sub:dev", false, false),
        ];

        let mut tree = TaskTree::new();
        tree.rebuild(&tasks, "");
        let names: Vec<&str> = tree
            .rows
            .iter()
            .filter_map(|r| r.as_task().map(|t| t.name.as_str()))
            .collect();
        assert_eq!(names, vec!["check"]);
        assert!(!tree.has_monorepo_groups());

        tree.show_hidden = true;
        tree.monorepo_all = true;
        tree.rebuild(&tasks, "");
        let names: Vec<&str> = tree
            .rows
            .iter()
            .filter_map(|r| r.as_task().map(|t| t.name.as_str()))
            .collect();
        assert_eq!(names, vec!["_internal", "check", "//pkg/sub:dev"]);
        assert!(tree.has_monorepo_groups());
    }

    #[test]
    fn collapsed_state_persists_across_reloads_and_auto_expands_on_filter() {
        let tasks = vec![
            mk_task("build", false, false),
            mk_task("ci:lint", false, false),
            mk_task("ci:test", false, false),
            mk_task("global-sync", false, true),
        ];

        let mut tree = TaskTree::new();
        tree.toggle_group("ci");
        assert!(tree.is_collapsed("ci"));
        assert!(tree.is_collapsed("[global]"));

        // Rebuild with fresh task list (simulating reload)
        tree.rebuild(&tasks, "");
        let visible_tasks: Vec<&str> = tree
            .rows
            .iter()
            .filter_map(|r| r.as_task().map(|t| t.name.as_str()))
            .collect();
        assert_eq!(visible_tasks, vec!["build"]);

        // Filtering auto-expands matching collapsed groups with match_indices
        tree.rebuild(&tasks, "lint");
        let matched: Vec<(&str, &[usize])> = tree
            .rows
            .iter()
            .filter_map(|r| match r {
                TaskRow::Task {
                    task,
                    match_indices,
                } => Some((task.name.as_str(), match_indices.as_slice())),
                TaskRow::Group { .. } => None,
            })
            .collect();
        assert_eq!(matched, vec![("ci:lint", &[0, 1, 2, 3][..])]);

        // Clearing filter restores persisted collapsed state
        tree.rebuild(&tasks, "");
        assert!(tree.is_collapsed("ci"));
        let visible_after_clear: Vec<&str> = tree
            .rows
            .iter()
            .filter_map(|r| r.as_task().map(|t| t.name.as_str()))
            .collect();
        assert_eq!(visible_after_clear, vec!["build"]);
    }

    #[test]
    fn script_cache_hits_and_invalidates_on_mtime_change() {
        let dir = temp_test_dir("cache");
        let script_path = dir.join("deploy.sh");
        fs::write(&script_path, "#!/usr/bin/env bash\necho v1\n").unwrap();

        let mut cache = ScriptCache::new();
        assert_eq!(cache.get_cached(&script_path), None);

        let first = cache.preload(script_path.clone()).unwrap().to_string();
        assert_eq!(first, "#!/usr/bin/env bash\necho v1\n");
        assert_eq!(cache.len(), 1);

        // Read-only accessor resolves from cache without disk I/O
        let task = MiseTask {
            name: "deploy".to_string(),
            file: Some(script_path.to_string_lossy().into_owned()),
            ..Default::default()
        };
        assert_eq!(cache.task_script_readonly(&task), first);

        // Update file with strictly newer mtime to force invalidation
        fs::write(&script_path, "#!/usr/bin/env bash\necho v2\n").unwrap();
        let future = SystemTime::now() + Duration::from_secs(5);
        File::options()
            .write(true)
            .open(&script_path)
            .unwrap()
            .set_modified(future)
            .unwrap();

        let updated = cache.preload(script_path.clone()).unwrap().to_string();
        assert_eq!(updated, "#!/usr/bin/env bash\necho v2\n");

        // Deleting file evicts stale cache entry on preload
        fs::remove_file(&script_path).unwrap();
        assert_eq!(cache.preload(script_path.clone()), None);
        assert_eq!(cache.get_cached(&script_path), None);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn script_cache_truncates_oversized_unicode_files_safely() {
        let dir = temp_test_dir("truncate");
        let big_path = dir.join("big.sh");
        // Multi-byte UTF-8 Chinese characters exceeding a 32-byte cap
        let content = "你好世界".repeat(10); // 120 bytes
        fs::write(&big_path, &content).unwrap();

        let mut cache = ScriptCache::with_max_bytes(32);
        let loaded = cache.preload(big_path.clone()).unwrap().to_string();
        assert!(cache.is_truncated(&big_path));
        assert!(loaded.starts_with("你好"));
        assert!(loaded.contains("truncated at 32 B"));

        let _ = fs::remove_dir_all(&dir);
    }
}
