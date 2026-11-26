use std::collections::HashMap;
use std::path::Path;

use crate::action::ToolScope;
use crate::mise::schema::{
    AuditReport, MiseToolEntry, MiseToolsMap, OutdatedEntry, OutdatedMap, RegistryEntry,
    ToolDetail, Vulnerability,
};
use crate::model::fuzzy::fuzzy_match;

#[derive(Debug, Clone)]
pub struct ToolRow {
    pub name: String,
    pub entry: MiseToolEntry,
    pub outdated: Option<OutdatedEntry>,
    pub vulnerabilities: Vec<Vulnerability>,
    pub is_project_local: bool,
    pub match_indices: Vec<usize>,
}

impl ToolRow {
    /// Returns true when `d` on this row should uninstall only this installed version
    /// (`mise uninstall <tool>@<version>`) instead of editing `mise.toml` (`mise unuse <tool>`).
    pub fn should_uninstall_version_only(&self) -> bool {
        self.entry.installed
            && (self.entry.source.is_none()
                || !self.entry.active
                || self.entry.scheduled_for_pruning)
    }
}

/// Real-time badge counters for the Tools panel.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ToolStats {
    pub total: usize,
    pub active: usize,
    pub project: usize,
    pub outdated: usize,
    pub vulnerable: usize,
    pub missing: usize,
    pub prunable: usize,
}

/// Fuzzy-matched item from `mise registry --json`.
#[derive(Debug, Clone)]
pub struct RegistryMatch {
    pub entry: RegistryEntry,
    pub score: i32,
    pub match_indices: Vec<usize>,
}

/// Aggregated view model for Tools (merging `ls`, `outdated`, `audit`, `tool <name>`, `registry`).
#[derive(Debug, Default, Clone)]
pub struct ToolViewModel {
    pub raw_tools: MiseToolsMap,
    pub outdated: OutdatedMap,
    pub audit: Option<AuditReport>,
    pub details_cache: HashMap<String, ToolDetail>,
    pub registry: Vec<RegistryEntry>,
    pub lock_preview: Option<String>,
    pub rows: Vec<ToolRow>,
}

impl ToolViewModel {
    pub fn rebuild(&mut self, workdir: &Path, scope: ToolScope, filter: &str) {
        let needle = filter.trim();
        let mut scored_rows: Vec<(ToolRow, i32)> = Vec::new();

        for (name, entries) in &self.raw_tools {
            for entry in entries {
                let is_project_local = is_entry_project_local(entry, workdir);
                let matches_scope = match scope {
                    ToolScope::All => true,
                    ToolScope::Active => entry.active,
                    ToolScope::Project => is_project_local,
                    ToolScope::Missing => !entry.installed,
                    ToolScope::Prunable => entry.scheduled_for_pruning,
                };
                if !matches_scope {
                    continue;
                }

                let (match_indices, score) = if needle.is_empty() {
                    (Vec::new(), 0)
                } else if let Some(m) = fuzzy_match(needle, name) {
                    (m.matched_indices, m.score + 40)
                } else if let Some(m) = fuzzy_match(needle, &entry.version) {
                    (Vec::new(), m.score + 20)
                } else if let Some(req) = entry.requested_version.as_deref()
                    && let Some(m) = fuzzy_match(needle, req)
                {
                    (Vec::new(), m.score + 10)
                } else if let Some(desc) = self
                    .details_cache
                    .get(name)
                    .and_then(|d| d.description.as_deref())
                    && let Some(m) = fuzzy_match(needle, desc)
                {
                    (Vec::new(), m.score)
                } else {
                    continue;
                };

                let outdated = self.outdated.get(name).and_then(|out| {
                    if entry.active
                        || out.current.as_deref() == Some(entry.version.as_str())
                        || (!entry.installed && out.requested == entry.requested_version)
                    {
                        Some(out.clone())
                    } else {
                        None
                    }
                });

                let vulnerabilities = self
                    .audit
                    .as_ref()
                    .map(|a| {
                        a.vulnerabilities
                            .iter()
                            .filter(|v| {
                                v.tool == *name
                                    && (v.version == entry.version || v.version.is_empty())
                            })
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();

                scored_rows.push((
                    ToolRow {
                        name: name.clone(),
                        entry: entry.clone(),
                        outdated,
                        vulnerabilities,
                        is_project_local,
                        match_indices,
                    },
                    score,
                ));
            }
        }

        if needle.is_empty() {
            scored_rows.sort_by(|(a, _), (b, _)| {
                b.entry
                    .active
                    .cmp(&a.entry.active)
                    .then_with(|| a.name.cmp(&b.name))
                    .then_with(|| a.entry.version.cmp(&b.entry.version))
            });
        } else {
            scored_rows.sort_by(|(a, sa), (b, sb)| {
                sb.cmp(sa)
                    .then_with(|| b.entry.active.cmp(&a.entry.active))
                    .then_with(|| a.name.cmp(&b.name))
                    .then_with(|| a.entry.version.cmp(&b.entry.version))
            });
        }

        self.rows = scored_rows.into_iter().map(|(row, _)| row).collect();
    }

    /// Compute real-time summary badge counts across all loaded tools.
    pub fn stats(&self, workdir: &Path) -> ToolStats {
        let mut stats = ToolStats {
            outdated: self.outdated_count(),
            vulnerable: self.vulnerability_count(),
            ..Default::default()
        };
        for entries in self.raw_tools.values() {
            for entry in entries {
                stats.total += 1;
                if entry.active {
                    stats.active += 1;
                }
                if is_entry_project_local(entry, workdir) {
                    stats.project += 1;
                }
                if !entry.installed {
                    stats.missing += 1;
                }
                if entry.scheduled_for_pruning {
                    stats.prunable += 1;
                }
            }
        }
        stats
    }

    /// Instant fuzzy search over `self.registry` (1,079+ official mise registry entries).
    pub fn filter_registry(&self, query: &str) -> Vec<RegistryMatch> {
        filter_registry_entries(&self.registry, query)
    }

    pub fn detail_for(&self, tool_name: &str) -> Option<&ToolDetail> {
        self.details_cache.get(tool_name)
    }

    pub fn outdated_count(&self) -> usize {
        self.outdated.len()
    }

    pub fn vulnerability_count(&self) -> usize {
        self.audit
            .as_ref()
            .map(|a| a.vulnerabilities.len())
            .unwrap_or(0)
    }

    pub fn total_installed_rows(&self) -> usize {
        self.raw_tools.values().map(Vec::len).sum()
    }

    pub fn active_count(&self) -> usize {
        self.raw_tools
            .values()
            .flat_map(|v| v.iter())
            .filter(|e| e.active)
            .count()
    }

    pub fn missing_count(&self) -> usize {
        self.raw_tools
            .values()
            .flat_map(|v| v.iter())
            .filter(|e| !e.installed)
            .count()
    }

    pub fn prunable_count(&self) -> usize {
        self.raw_tools
            .values()
            .flat_map(|v| v.iter())
            .filter(|e| e.scheduled_for_pruning)
            .count()
    }
}

/// Fuzzy-filter any slice of `RegistryEntry` by `short`, `aliases`, `bins`, `backends`, and `description`.
pub fn filter_registry_entries(entries: &[RegistryEntry], query: &str) -> Vec<RegistryMatch> {
    let needle = query.trim();
    if needle.is_empty() {
        let mut all: Vec<RegistryMatch> = entries
            .iter()
            .cloned()
            .map(|entry| RegistryMatch {
                entry,
                score: 0,
                match_indices: Vec::new(),
            })
            .collect();
        all.sort_by(|a, b| a.entry.short.cmp(&b.entry.short));
        return all;
    }

    let mut matches = Vec::new();
    for entry in entries {
        let mut best_score: Option<i32> = None;
        let mut short_indices = Vec::new();

        if let Some(m) = fuzzy_match(needle, &entry.short) {
            best_score = Some(m.score + 200);
            short_indices = m.matched_indices;
        }

        for alias in &entry.aliases {
            if let Some(m) = fuzzy_match(needle, alias) {
                let s = m.score + 150;
                if best_score.is_none_or(|cur| s > cur) {
                    best_score = Some(s);
                }
            }
        }

        for bin in &entry.bins {
            if let Some(m) = fuzzy_match(needle, bin) {
                let s = m.score + 120;
                if best_score.is_none_or(|cur| s > cur) {
                    best_score = Some(s);
                }
            }
        }

        for backend in &entry.backends {
            if let Some(m) = fuzzy_match(needle, backend) {
                let s = m.score + 80;
                if best_score.is_none_or(|cur| s > cur) {
                    best_score = Some(s);
                }
            }
        }

        if let Some(desc) = entry.description.as_deref()
            && let Some(m) = fuzzy_match(needle, desc)
        {
            let s = m.score + 40;
            if best_score.is_none_or(|cur| s > cur) {
                best_score = Some(s);
            }
        }

        if let Some(score) = best_score {
            matches.push(RegistryMatch {
                entry: entry.clone(),
                score,
                match_indices: short_indices,
            });
        }
    }

    matches.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.entry.short.cmp(&b.entry.short))
    });
    matches
}

fn is_entry_project_local(entry: &MiseToolEntry, workdir: &Path) -> bool {
    let Some(path_str) = entry.source.as_ref().and_then(|s| s.path.as_deref()) else {
        return false;
    };
    path_starts_with_normalized(Path::new(path_str), workdir)
}

fn path_starts_with_normalized(path: &Path, base: &Path) -> bool {
    if path.starts_with(base) {
        return true;
    }
    normalize_macos_private(path).starts_with(normalize_macos_private(base))
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

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::mise::schema::MiseToolSource;

    fn sample_vm() -> ToolViewModel {
        let mut raw_tools: MiseToolsMap = BTreeMap::new();
        raw_tools.insert(
            "node".to_string(),
            vec![
                MiseToolEntry {
                    version: "22.1.0".to_string(),
                    requested_version: Some("22".to_string()),
                    source: Some(MiseToolSource {
                        kind: Some("mise.toml".to_string()),
                        path: Some("/repo/project/mise.toml".to_string()),
                    }),
                    installed: true,
                    active: true,
                    scheduled_for_pruning: false,
                    ..Default::default()
                },
                MiseToolEntry {
                    version: "18.0.0".to_string(),
                    installed: true,
                    active: false,
                    scheduled_for_pruning: true,
                    ..Default::default()
                },
            ],
        );
        raw_tools.insert(
            "python".to_string(),
            vec![MiseToolEntry {
                version: "3.13.0".to_string(),
                requested_version: Some("3.13".to_string()),
                source: Some(MiseToolSource {
                    kind: Some("mise.toml".to_string()),
                    path: Some("/Users/dev/.config/mise/config.toml".to_string()),
                }),
                installed: false,
                active: true,
                scheduled_for_pruning: false,
                ..Default::default()
            }],
        );
        raw_tools.insert(
            "rust".to_string(),
            vec![MiseToolEntry {
                version: "1.90.0".to_string(),
                requested_version: Some("stable".to_string()),
                source: Some(MiseToolSource {
                    kind: Some("mise.toml".to_string()),
                    path: Some("/repo/project/mise.toml".to_string()),
                }),
                installed: true,
                active: true,
                scheduled_for_pruning: false,
                ..Default::default()
            }],
        );

        let mut outdated: OutdatedMap = BTreeMap::new();
        outdated.insert(
            "node".to_string(),
            OutdatedEntry {
                name: "node".to_string(),
                requested: Some("22".to_string()),
                current: Some("22.1.0".to_string()),
                latest: "22.12.0".to_string(),
                ..Default::default()
            },
        );

        let audit = Some(AuditReport {
            vulnerabilities: vec![Vulnerability {
                tool: "node".to_string(),
                version: "22.1.0".to_string(),
                id: "CVE-2026-0001".to_string(),
                severity: Some("HIGH".to_string()),
                summary: "Test CVE".to_string(),
                ..Default::default()
            }],
            unchecked: Vec::new(),
        });

        let registry = vec![
            RegistryEntry {
                short: "ripgrep".to_string(),
                backends: vec!["aqua:BurntSushi/ripgrep".to_string()],
                bins: vec!["rg".to_string()],
                description: Some("Fast line-oriented search tool".to_string()),
                aliases: vec!["rg".to_string()],
            },
            RegistryEntry {
                short: "fd".to_string(),
                backends: vec!["aqua:sharkdp/fd".to_string()],
                bins: vec!["fd".to_string()],
                description: Some("Simple fast alternative to find".to_string()),
                aliases: vec!["fd-find".to_string()],
            },
            RegistryEntry {
                short: "jq".to_string(),
                backends: vec!["aqua:jqlang/jq".to_string()],
                bins: vec!["jq".to_string()],
                description: Some("Command-line JSON processor".to_string()),
                aliases: vec![],
            },
        ];

        ToolViewModel {
            raw_tools,
            outdated,
            audit,
            registry,
            ..Default::default()
        }
    }

    #[test]
    fn aggregates_outdated_audit_and_stats_accurately() {
        let mut vm = sample_vm();
        let workdir = Path::new("/repo/project");
        vm.rebuild(workdir, ToolScope::All, "");

        let stats = vm.stats(workdir);
        assert_eq!(
            stats,
            ToolStats {
                total: 4,
                active: 3,
                project: 2,
                outdated: 1,
                vulnerable: 1,
                missing: 1,
                prunable: 1,
            }
        );

        let node_active = vm
            .rows
            .iter()
            .find(|r| r.name == "node" && r.entry.version == "22.1.0")
            .unwrap();
        assert!(node_active.is_project_local);
        assert_eq!(
            node_active.outdated.as_ref().map(|o| o.latest.as_str()),
            Some("22.12.0")
        );
        assert_eq!(node_active.vulnerabilities.len(), 1);
        assert_eq!(node_active.vulnerabilities[0].id, "CVE-2026-0001");

        // Inactive older node version does not get active version's outdated/CVE attached
        let node_old = vm
            .rows
            .iter()
            .find(|r| r.name == "node" && r.entry.version == "18.0.0")
            .unwrap();
        assert!(node_old.outdated.is_none());
        assert!(node_old.vulnerabilities.is_empty());
    }

    #[test]
    fn filters_all_five_tool_scopes_and_fuzzy_query() {
        let mut vm = sample_vm();
        let workdir = Path::new("/repo/project");

        vm.rebuild(workdir, ToolScope::Active, "");
        assert_eq!(vm.rows.len(), 3);

        vm.rebuild(workdir, ToolScope::Project, "");
        let project_names: Vec<&str> = vm.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(project_names, vec!["node", "rust"]);

        vm.rebuild(workdir, ToolScope::Missing, "");
        assert_eq!(vm.rows.len(), 1);
        assert_eq!(vm.rows[0].name, "python");

        vm.rebuild(workdir, ToolScope::Prunable, "");
        assert_eq!(vm.rows.len(), 1);
        assert_eq!(vm.rows[0].entry.version, "18.0.0");

        vm.rebuild(workdir, ToolScope::All, "rst");
        assert_eq!(vm.rows.len(), 1);
        assert_eq!(vm.rows[0].name, "rust");
        assert_eq!(vm.rows[0].match_indices, vec![0, 2, 3]);
    }

    #[test]
    fn filters_registry_by_short_aliases_bins_backends_and_description() {
        let vm = sample_vm();

        // Match by alias/bin "rg"
        let by_alias = vm.filter_registry("rg");
        assert_eq!(by_alias.first().map(|m| m.entry.short.as_str()), Some("ripgrep"));

        // Match by description "JSON"
        let by_desc = vm.filter_registry("json");
        assert_eq!(by_desc.len(), 1);
        assert_eq!(by_desc[0].entry.short, "jq");

        // Match by backend "sharkdp"
        let by_backend = vm.filter_registry("sharkdp");
        assert_eq!(by_backend.len(), 1);
        assert_eq!(by_backend[0].entry.short, "fd");

        // Empty query returns sorted catalog
        let all = vm.filter_registry("");
        let shorts: Vec<&str> = all.iter().map(|m| m.entry.short.as_str()).collect();
        assert_eq!(shorts, vec!["fd", "jq", "ripgrep"]);

        // macOS /private/var <-> /var project-local detection
        assert!(path_starts_with_normalized(
            Path::new("/private/var/tmp/repo/mise.toml"),
            Path::new("/var/tmp/repo")
        ));
    }
}
