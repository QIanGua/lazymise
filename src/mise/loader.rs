use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Instant;

use crate::action::{Domain, ProfileSpec};
use crate::events::{AppEvent, CommandLogEntry, CommandSource, MiseDataEvent, MisePayload};
use crate::mise::{MiseClient, MiseExecOutput};
use crate::model::template::discover_templates;

/// Non-blocking, generation-aware asynchronous loader for `mise` CLI domains.
///
/// Tier-1 hot domains (`Tasks`, `Tools`, `Env`, `Config`, `Templates`) fan out concurrently
/// and stream progressive `AppEvent::MiseData` messages as each finishes.
/// Tier-2 slow/detail domains (`Outdated`, `Audit`, `Doctor`, `Registry`, `TaskInfo`, etc.)
/// load on demand without blocking the UI thread.
#[derive(Clone)]
pub struct MiseLoader {
    workdir: PathBuf,
    tx: Sender<AppEvent>,
    current_gen: Arc<AtomicU64>,
    log_seq: Arc<AtomicU64>,
}

impl MiseLoader {
    pub fn new(workdir: PathBuf, tx: Sender<AppEvent>) -> Self {
        Self {
            workdir,
            tx,
            current_gen: Arc::new(AtomicU64::new(0)),
            log_seq: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn next_generation(&self) -> u64 {
        self.current_gen.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn current_generation(&self) -> u64 {
        self.current_gen.load(Ordering::SeqCst)
    }

    pub fn is_stale(&self, generation: u64) -> bool {
        self.current_gen.load(Ordering::SeqCst) > generation
    }

    pub fn workdir(&self) -> &PathBuf {
        &self.workdir
    }

    pub fn client(&self, profile: ProfileSpec) -> MiseClient {
        MiseClient::with_profile(&self.workdir, profile)
    }

    fn next_log_id(&self) -> u64 {
        self.log_seq.fetch_add(1, Ordering::Relaxed)
    }

    /// Emit a single `MiseDataEvent` unconditionally (kept for compatibility).
    pub fn emit(&self, event: MiseDataEvent) {
        let _ = self.tx.send(AppEvent::MiseData(event));
    }

    /// Emit a `MiseDataEvent` only if its generation has not been superseded.
    pub fn emit_if_current(&self, event: MiseDataEvent) -> bool {
        if self.is_stale(event.generation) {
            return false;
        }
        self.tx.send(AppEvent::MiseData(event)).is_ok()
    }

    /// Trigger Tier-1 hot-path parallel reload (`Tasks`, `Tools`, `Env`, `Config`, `Templates`).
    pub fn request_hot_reload(
        &self,
        generation: u64,
        profile: ProfileSpec,
        include_hidden_tasks: bool,
        all_monorepo_tasks: bool,
    ) {
        self.current_gen.fetch_max(generation, Ordering::SeqCst);

        // 1. Tasks
        {
            let loader = self.clone();
            let prof = profile.clone();
            thread::spawn(move || {
                if loader.is_stale(generation) {
                    return;
                }
                loader.fetch_tasks(generation, prof, include_hidden_tasks, all_monorepo_tasks);
            });
        }

        // 2. Tools
        {
            let loader = self.clone();
            let prof = profile.clone();
            thread::spawn(move || {
                if loader.is_stale(generation) {
                    return;
                }
                loader.fetch_tools(generation, prof);
            });
        }

        // 3. Env (extended + redacted + paths + vars concurrently)
        {
            let loader = self.clone();
            let prof = profile.clone();
            thread::spawn(move || {
                if loader.is_stale(generation) {
                    return;
                }
                loader.fetch_env(generation, prof);
            });
        }

        // 4. Config + Trust concurrently
        {
            let loader = self.clone();
            let prof = profile.clone();
            thread::spawn(move || {
                if loader.is_stale(generation) {
                    return;
                }
                loader.fetch_config(generation, prof);
            });
        }

        // 5. Workspace Templates (filesystem discovery)
        {
            let loader = self.clone();
            thread::spawn(move || {
                if loader.is_stale(generation) {
                    return;
                }
                let templates = discover_templates();
                loader.emit_if_current(MiseDataEvent {
                    generation,
                    domain: Domain::Templates,
                    payload: MisePayload::Templates(templates),
                    log_entry: None,
                });
            });
        }
    }

    /// Trigger an on-demand / Tier-2 domain fetch (e.g. `Outdated`, `Audit`, `Doctor`, `Registry`, `TaskInfo`).
    pub fn request_domain(
        &self,
        generation: u64,
        profile: ProfileSpec,
        domain: Domain,
        target: Option<String>,
    ) {
        let loader = self.clone();
        thread::spawn(move || {
            if loader.is_stale(generation) {
                return;
            }
            match domain {
                Domain::Tasks => loader.fetch_tasks(generation, profile, false, false),
                Domain::TaskInfo => loader.fetch_task_info(generation, profile, target),
                Domain::TaskDeps => loader.fetch_task_deps(generation, profile, target),
                Domain::TaskHistory => loader.fetch_task_history(generation, profile, target),
                Domain::Tools => loader.fetch_tools(generation, profile),
                Domain::ToolDetail => loader.fetch_tool_detail(generation, profile, target),
                Domain::ToolVersions => loader.fetch_tool_versions(generation, profile, target),
                Domain::Registry => loader.fetch_registry(generation, profile),
                Domain::Outdated => loader.fetch_outdated(generation, profile),
                Domain::Audit => loader.fetch_audit(generation, profile),
                Domain::LockDryRun => loader.fetch_lock_dry_run(generation, profile),
                Domain::Env => loader.fetch_env(generation, profile),
                Domain::Config => loader.fetch_config(generation, profile),
                Domain::ConfigExplain => loader.fetch_config_explain(generation, profile),
                Domain::Doctor => loader.fetch_doctor(generation, profile),
                Domain::Settings => loader.fetch_settings(generation, profile),
                Domain::Daemons => loader.fetch_daemons(generation, profile),
                Domain::Deps => loader.fetch_deps(generation, profile),
                Domain::Templates => {
                    let templates = discover_templates();
                    loader.emit_if_current(MiseDataEvent {
                        generation,
                        domain: Domain::Templates,
                        payload: MisePayload::Templates(templates),
                        log_entry: None,
                    });
                }
            }
        });
    }

    // -------------------------------------------------------------------------
    // Per-domain worker implementations
    // -------------------------------------------------------------------------

    fn fetch_tasks(
        &self,
        generation: u64,
        profile: ProfileSpec,
        include_hidden: bool,
        all_monorepo: bool,
    ) {
        let client = self.client(profile);
        let start = Instant::now();
        let mut cmd_args = vec!["tasks", "ls", "--json"];
        if include_hidden {
            cmd_args.push("--hidden");
        }
        if all_monorepo {
            cmd_args.push("--all");
        }
        let fallback_cmd = format!("mise {}", client.build_args(&cmd_args).join(" "));

        match client.tasks_ls_with_output(include_hidden, all_monorepo) {
            Ok((tasks, out)) => {
                let summary = Some(format!("{} tasks", tasks.len()));
                let log = out.to_log_entry_with_summary(
                    self.next_log_id(),
                    CommandSource::Internal,
                    summary,
                );
                self.emit_if_current(MiseDataEvent {
                    generation,
                    domain: Domain::Tasks,
                    payload: MisePayload::Tasks(tasks),
                    log_entry: Some(log),
                });
            }
            Err(err) => {
                self.emit_error(generation, Domain::Tasks, fallback_cmd, start, err);
            }
        }
    }

    fn fetch_task_info(&self, generation: u64, profile: ProfileSpec, target: Option<String>) {
        let Some(task_name) = target.filter(|s| !s.trim().is_empty()) else {
            self.emit_missing_target(generation, Domain::TaskInfo);
            return;
        };
        let client = self.client(profile);
        let start = Instant::now();
        let fallback_cmd = format!(
            "mise {}",
            client
                .build_args(&["tasks", "info", &task_name, "--json"])
                .join(" ")
        );

        match client.task_info_with_output(&task_name) {
            Ok((task, out)) => {
                let log = out.to_log_entry(self.next_log_id(), CommandSource::Internal);
                self.emit_if_current(MiseDataEvent {
                    generation,
                    domain: Domain::TaskInfo,
                    payload: MisePayload::TaskInfo {
                        task_name,
                        task: Box::new(task),
                    },
                    log_entry: Some(log),
                });
            }
            Err(err) => {
                self.emit_error(generation, Domain::TaskInfo, fallback_cmd, start, err);
            }
        }
    }

    fn fetch_task_deps(&self, generation: u64, profile: ProfileSpec, target: Option<String>) {
        let Some(task_name) = target.filter(|s| !s.trim().is_empty()) else {
            self.emit_missing_target(generation, Domain::TaskDeps);
            return;
        };
        let client = self.client(profile);
        let start = Instant::now();
        let fallback_cmd = format!(
            "mise {}",
            client.build_args(&["tasks", "deps", &task_name]).join(" ")
        );

        match client.task_deps_with_output(&task_name) {
            Ok((tree_lines, out)) => {
                let log = out.to_log_entry(self.next_log_id(), CommandSource::Internal);
                self.emit_if_current(MiseDataEvent {
                    generation,
                    domain: Domain::TaskDeps,
                    payload: MisePayload::TaskDeps {
                        task_name,
                        tree_lines,
                    },
                    log_entry: Some(log),
                });
            }
            Err(err) => {
                self.emit_error(generation, Domain::TaskDeps, fallback_cmd, start, err);
            }
        }
    }

    fn fetch_task_history(&self, generation: u64, profile: ProfileSpec, target: Option<String>) {
        let client = self.client(profile);
        let start = Instant::now();
        let task_name = target.unwrap_or_default();
        let task_opt = (!task_name.trim().is_empty()).then_some(task_name.as_str());
        let fallback_cmd = match task_opt {
            Some(t) => format!(
                "mise {}",
                client
                    .build_args(&["tasks", "history", "--json", t])
                    .join(" ")
            ),
            None => format!(
                "mise {}",
                client.build_args(&["tasks", "history", "--json"]).join(" ")
            ),
        };

        match client.task_history_with_output(task_opt) {
            Ok((entries, out)) => {
                let summary = Some(format!("{} runs", entries.len()));
                let log = out.to_log_entry_with_summary(
                    self.next_log_id(),
                    CommandSource::Internal,
                    summary,
                );
                self.emit_if_current(MiseDataEvent {
                    generation,
                    domain: Domain::TaskHistory,
                    payload: MisePayload::TaskHistory { task_name, entries },
                    log_entry: Some(log),
                });
            }
            Err(err) => {
                self.emit_error(generation, Domain::TaskHistory, fallback_cmd, start, err);
            }
        }
    }

    fn fetch_tools(&self, generation: u64, profile: ProfileSpec) {
        let client = self.client(profile);
        let start = Instant::now();
        let fallback_cmd = format!("mise {}", client.build_args(&["ls", "--json"]).join(" "));

        match client.tools_ls_with_output() {
            Ok((tools, out)) => {
                let summary = Some(format!("{} tools", tools.len()));
                let log = out.to_log_entry_with_summary(
                    self.next_log_id(),
                    CommandSource::Internal,
                    summary,
                );
                self.emit_if_current(MiseDataEvent {
                    generation,
                    domain: Domain::Tools,
                    payload: MisePayload::Tools(tools),
                    log_entry: Some(log),
                });
            }
            Err(err) => {
                self.emit_error(generation, Domain::Tools, fallback_cmd, start, err);
            }
        }
    }

    fn fetch_tool_detail(&self, generation: u64, profile: ProfileSpec, target: Option<String>) {
        let Some(tool_name) = target.filter(|s| !s.trim().is_empty()) else {
            self.emit_missing_target(generation, Domain::ToolDetail);
            return;
        };
        let client = self.client(profile);
        let start = Instant::now();
        let fallback_cmd = format!(
            "mise {}",
            client.build_args(&["tool", &tool_name, "--json"]).join(" ")
        );

        match client.tool_detail_with_output(&tool_name) {
            Ok((detail, out)) => {
                let log = out.to_log_entry(self.next_log_id(), CommandSource::Internal);
                self.emit_if_current(MiseDataEvent {
                    generation,
                    domain: Domain::ToolDetail,
                    payload: MisePayload::ToolDetail {
                        tool_name,
                        detail: Box::new(detail),
                    },
                    log_entry: Some(log),
                });
            }
            Err(err) => {
                self.emit_error(generation, Domain::ToolDetail, fallback_cmd, start, err);
            }
        }
    }

    fn fetch_tool_versions(&self, generation: u64, profile: ProfileSpec, target: Option<String>) {
        let Some(tool_name) = target.filter(|s| !s.trim().is_empty()) else {
            self.emit_missing_target(generation, Domain::ToolVersions);
            return;
        };
        let client = self.client(profile);
        let start = Instant::now();
        let fallback_cmd = format!(
            "mise {}",
            client.build_args(&["ls-remote", &tool_name]).join(" ")
        );

        match client.tool_versions_with_output(&tool_name) {
            Ok((versions, out)) => {
                let summary = Some(format!("{} versions", versions.len()));
                let log = out.to_log_entry_with_summary(
                    self.next_log_id(),
                    CommandSource::Internal,
                    summary,
                );
                self.emit_if_current(MiseDataEvent {
                    generation,
                    domain: Domain::ToolVersions,
                    payload: MisePayload::ToolVersions {
                        tool_name,
                        versions,
                    },
                    log_entry: Some(log),
                });
            }
            Err(err) => {
                self.emit_error(generation, Domain::ToolVersions, fallback_cmd, start, err);
            }
        }
    }

    fn fetch_registry(&self, generation: u64, profile: ProfileSpec) {
        let client = self.client(profile);
        let start = Instant::now();
        let fallback_cmd = format!(
            "mise {}",
            client.build_args(&["registry", "--json"]).join(" ")
        );

        match client.registry_with_output() {
            Ok((entries, out)) => {
                let summary = Some(format!("{} registry tools", entries.len()));
                let log = out.to_log_entry_with_summary(
                    self.next_log_id(),
                    CommandSource::Internal,
                    summary,
                );
                self.emit_if_current(MiseDataEvent {
                    generation,
                    domain: Domain::Registry,
                    payload: MisePayload::Registry(entries),
                    log_entry: Some(log),
                });
            }
            Err(err) => {
                self.emit_error(generation, Domain::Registry, fallback_cmd, start, err);
            }
        }
    }

    fn fetch_outdated(&self, generation: u64, profile: ProfileSpec) {
        let client = self.client(profile);
        let start = Instant::now();
        let fallback_cmd = format!(
            "mise {}",
            client.build_args(&["outdated", "--json"]).join(" ")
        );

        match client.outdated_with_output() {
            Ok((outdated, out)) => {
                let summary = Some(format!("{} outdated", outdated.len()));
                let log = out.to_log_entry_with_summary(
                    self.next_log_id(),
                    CommandSource::Internal,
                    summary,
                );
                self.emit_if_current(MiseDataEvent {
                    generation,
                    domain: Domain::Outdated,
                    payload: MisePayload::Outdated(outdated),
                    log_entry: Some(log),
                });
            }
            Err(err) => {
                self.emit_error(generation, Domain::Outdated, fallback_cmd, start, err);
            }
        }
    }

    fn fetch_audit(&self, generation: u64, profile: ProfileSpec) {
        let client = self.client(profile);
        let start = Instant::now();
        let fallback_cmd = format!("mise {}", client.build_args(&["audit", "--json"]).join(" "));

        match client.audit_with_output() {
            Ok((report, out)) => {
                let summary = Some(format!(
                    "{} vulns, {} unchecked",
                    report.vulnerabilities.len(),
                    report.unchecked.len()
                ));
                let log = out.to_log_entry_with_summary(
                    self.next_log_id(),
                    CommandSource::Internal,
                    summary,
                );
                self.emit_if_current(MiseDataEvent {
                    generation,
                    domain: Domain::Audit,
                    payload: MisePayload::Audit(report),
                    log_entry: Some(log),
                });
            }
            Err(err) => {
                self.emit_error(generation, Domain::Audit, fallback_cmd, start, err);
            }
        }
    }

    fn fetch_lock_dry_run(&self, generation: u64, profile: ProfileSpec) {
        let client = self.client(profile);
        let start = Instant::now();
        let fallback_cmd = format!(
            "mise {}",
            client.build_args(&["lock", "--dry-run"]).join(" ")
        );

        match client.lock_dry_run_with_output() {
            Ok((text, out)) => {
                let log = out.to_log_entry(self.next_log_id(), CommandSource::Internal);
                self.emit_if_current(MiseDataEvent {
                    generation,
                    domain: Domain::LockDryRun,
                    payload: MisePayload::LockDryRun(text),
                    log_entry: Some(log),
                });
            }
            Err(err) => {
                self.emit_error(generation, Domain::LockDryRun, fallback_cmd, start, err);
            }
        }
    }

    fn fetch_env(&self, generation: u64, profile: ProfileSpec) {
        let client = self.client(profile);
        let start = Instant::now();
        let fallback_cmd = format!(
            "mise {}",
            client.build_args(&["env", "--json-extended"]).join(" ")
        );

        let (ext_res, redacted_res, paths_res, vars_res) = thread::scope(|s| {
            let h_ext = s.spawn(|| client.env_extended_with_output());
            let h_redacted = s.spawn(|| client.env_redacted());
            let h_paths = s.spawn(|| client.env_paths());
            let h_vars = s.spawn(|| client.vars());
            (
                h_ext.join(),
                h_redacted.join(),
                h_paths.join(),
                h_vars.join(),
            )
        });

        let ext_res = match ext_res {
            Ok(r) => r,
            Err(_) => Err(anyhow::anyhow!("env loader thread panicked")),
        };

        match ext_res {
            Ok((extended, out)) => {
                let redacted_keys: BTreeSet<String> = redacted_res
                    .ok()
                    .and_then(Result::ok)
                    .unwrap_or_default();
                let paths = paths_res.ok().and_then(Result::ok).unwrap_or_default();
                let vars = vars_res.ok().and_then(Result::ok).unwrap_or_default();
                let summary = Some(format!(
                    "{} vars, {} paths",
                    extended.len(),
                    paths.len()
                ));
                let log = out.to_log_entry_with_summary(
                    self.next_log_id(),
                    CommandSource::Internal,
                    summary,
                );
                self.emit_if_current(MiseDataEvent {
                    generation,
                    domain: Domain::Env,
                    payload: MisePayload::Env {
                        extended,
                        redacted_keys,
                        paths,
                        vars,
                    },
                    log_entry: Some(log),
                });
            }
            Err(err) => {
                self.emit_error(generation, Domain::Env, fallback_cmd, start, err);
            }
        }
    }

    fn fetch_config(&self, generation: u64, profile: ProfileSpec) {
        let client = self.client(profile);
        let start = Instant::now();
        let fallback_cmd = format!(
            "mise {}",
            client.build_args(&["config", "ls", "--json"]).join(" ")
        );

        let (cfg_res, trust_res) = thread::scope(|s| {
            let h_cfg = s.spawn(|| client.config_ls_with_output());
            let h_trust = s.spawn(|| client.trust_show());
            (h_cfg.join(), h_trust.join())
        });

        let cfg_res = match cfg_res {
            Ok(r) => r,
            Err(_) => Err(anyhow::anyhow!("config loader thread panicked")),
        };

        match cfg_res {
            Ok((files, out)) => {
                let trust = trust_res.ok().and_then(Result::ok).unwrap_or_default();
                let summary = Some(format!("{} configs", files.len()));
                let log = out.to_log_entry_with_summary(
                    self.next_log_id(),
                    CommandSource::Internal,
                    summary,
                );
                self.emit_if_current(MiseDataEvent {
                    generation,
                    domain: Domain::Config,
                    payload: MisePayload::Config { files, trust },
                    log_entry: Some(log),
                });
            }
            Err(err) => {
                self.emit_error(generation, Domain::Config, fallback_cmd, start, err);
            }
        }
    }

    fn fetch_config_explain(&self, generation: u64, profile: ProfileSpec) {
        let client = self.client(profile);
        let start = Instant::now();
        let fallback_cmd = format!(
            "mise {}",
            client
                .build_args(&["config", "explain", "--json"])
                .join(" ")
        );

        match client.config_explain_with_output() {
            Ok((items, out)) => {
                let summary = Some(format!("{} keys", items.len()));
                let log = out.to_log_entry_with_summary(
                    self.next_log_id(),
                    CommandSource::Internal,
                    summary,
                );
                self.emit_if_current(MiseDataEvent {
                    generation,
                    domain: Domain::ConfigExplain,
                    payload: MisePayload::ConfigExplain(items),
                    log_entry: Some(log),
                });
            }
            Err(err) => {
                self.emit_error(generation, Domain::ConfigExplain, fallback_cmd, start, err);
            }
        }
    }

    fn fetch_doctor(&self, generation: u64, profile: ProfileSpec) {
        let client = self.client(profile);
        let start = Instant::now();
        let fallback_cmd = format!(
            "mise {}",
            client.build_args(&["doctor", "--json"]).join(" ")
        );

        match client.doctor_with_output() {
            Ok((report, out)) => {
                let summary = Some(format!(
                    "{} errors, {} warnings",
                    report.errors.len(),
                    report.warnings.len()
                ));
                let log = out.to_log_entry_with_summary(
                    self.next_log_id(),
                    CommandSource::Internal,
                    summary,
                );
                self.emit_if_current(MiseDataEvent {
                    generation,
                    domain: Domain::Doctor,
                    payload: MisePayload::Doctor(Box::new(report)),
                    log_entry: Some(log),
                });
            }
            Err(err) => {
                self.emit_error(generation, Domain::Doctor, fallback_cmd, start, err);
            }
        }
    }

    fn fetch_settings(&self, generation: u64, profile: ProfileSpec) {
        let client = self.client(profile);
        let start = Instant::now();
        let fallback_cmd = format!(
            "mise {}",
            client
                .build_args(&["settings", "ls", "--json-extended"])
                .join(" ")
        );

        match client.settings_extended_with_output() {
            Ok((entries, out)) => {
                let summary = Some(format!("{} settings", entries.len()));
                let log = out.to_log_entry_with_summary(
                    self.next_log_id(),
                    CommandSource::Internal,
                    summary,
                );
                self.emit_if_current(MiseDataEvent {
                    generation,
                    domain: Domain::Settings,
                    payload: MisePayload::Settings(entries),
                    log_entry: Some(log),
                });
            }
            Err(err) => {
                self.emit_error(generation, Domain::Settings, fallback_cmd, start, err);
            }
        }
    }

    fn fetch_daemons(&self, generation: u64, profile: ProfileSpec) {
        let client = self.client(profile);
        let start = Instant::now();
        let fallback_cmd = format!(
            "mise {}",
            client.build_args(&["daemons", "--json"]).join(" ")
        );

        match client.daemons_with_output() {
            Ok((daemons, out)) => {
                let summary = if daemons.is_empty() {
                    Some("no project daemons".to_string())
                } else {
                    Some(format!("{} daemons", daemons.len()))
                };
                let log = CommandLogEntry {
                    id: self.next_log_id(),
                    timestamp: std::time::SystemTime::now(),
                    source: CommandSource::Internal,
                    command: out.command_display,
                    job_id: None,
                    status: crate::events::JobStatus::Success {
                        duration_ms: out.duration_ms,
                    },
                    summary,
                };
                self.emit_if_current(MiseDataEvent {
                    generation,
                    domain: Domain::Daemons,
                    payload: MisePayload::Daemons(daemons),
                    log_entry: Some(log),
                });
            }
            Err(err) => {
                self.emit_error(generation, Domain::Daemons, fallback_cmd, start, err);
            }
        }
    }

    fn fetch_deps(&self, generation: u64, profile: ProfileSpec) {
        let client = self.client(profile);
        let start = Instant::now();
        let fallback_cmd = format!("mise {}", client.build_args(&["deps", "--list"]).join(" "));

        match client.deps_list_with_output() {
            Ok((deps, out)) => {
                let summary = if deps.is_empty() {
                    Some("no deps providers".to_string())
                } else {
                    Some(format!("{} providers", deps.len()))
                };
                let log = out.to_log_entry_with_summary(
                    self.next_log_id(),
                    CommandSource::Internal,
                    summary,
                );
                self.emit_if_current(MiseDataEvent {
                    generation,
                    domain: Domain::Deps,
                    payload: MisePayload::Deps(deps),
                    log_entry: Some(log),
                });
            }
            Err(err) => {
                self.emit_error(generation, Domain::Deps, fallback_cmd, start, err);
            }
        }
    }

    fn emit_error(
        &self,
        generation: u64,
        domain: Domain,
        command_display: String,
        start: Instant,
        err: anyhow::Error,
    ) {
        let message = format!("{err:#}");
        let log = MiseExecOutput::error_log_entry(
            self.next_log_id(),
            CommandSource::Internal,
            command_display,
            start.elapsed().as_millis(),
            message.clone(),
        );
        self.emit_if_current(MiseDataEvent {
            generation,
            domain,
            payload: MisePayload::Error { domain, message },
            log_entry: Some(log),
        });
    }

    fn emit_missing_target(&self, generation: u64, domain: Domain) {
        self.emit_if_current(MiseDataEvent {
            generation,
            domain,
            payload: MisePayload::Error {
                domain,
                message: format!("missing target argument for `{domain}`"),
            },
            log_entry: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::mpsc;

    use pretty_assertions::assert_eq;

    use crate::action::Domain;
    use crate::events::{AppEvent, MiseDataEvent, MisePayload};

    use super::MiseLoader;

    #[test]
    fn generation_cancellation_drops_stale_events() {
        let (tx, rx) = mpsc::channel();
        let loader = MiseLoader::new(PathBuf::from("."), tx);

        let gen1 = loader.next_generation();
        let gen2 = loader.next_generation();
        assert_eq!(gen1, 1);
        assert_eq!(gen2, 2);
        assert!(loader.is_stale(gen1));
        assert!(!loader.is_stale(gen2));

        let delivered_stale = loader.emit_if_current(MiseDataEvent {
            generation: gen1,
            domain: Domain::Tasks,
            payload: MisePayload::Tasks(Vec::new()),
            log_entry: None,
        });
        assert!(!delivered_stale);
        assert!(rx.try_recv().is_err());

        let delivered_current = loader.emit_if_current(MiseDataEvent {
            generation: gen2,
            domain: Domain::Tasks,
            payload: MisePayload::Tasks(Vec::new()),
            log_entry: None,
        });
        assert!(delivered_current);

        match rx.try_recv().unwrap() {
            AppEvent::MiseData(ev) => {
                assert_eq!(ev.generation, gen2);
                assert_eq!(ev.domain, Domain::Tasks);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn missing_target_emits_error_without_spawning() {
        let (tx, rx) = mpsc::channel();
        let loader = MiseLoader::new(PathBuf::from("."), tx);
        let generation = loader.next_generation();

        loader.request_domain(generation, None, Domain::TaskInfo, None);
        let ev = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("should receive missing-target error");
        match ev {
            AppEvent::MiseData(data) => {
                assert_eq!(data.domain, Domain::TaskInfo);
                assert!(matches!(data.payload, MisePayload::Error { .. }));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
