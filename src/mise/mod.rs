pub mod loader;
pub mod schema;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;

use crate::action::ProfileSpec;
use crate::events::{CommandLogEntry, CommandSource, JobStatus};

pub use self::schema::{
    flatten_settings, parse_daemons_json, parse_deps_list, parse_env_paths,
    parse_settings_extended_json, parse_task_deps, parse_tracked_configs, parse_trust_show,
};
use self::schema::{
    AuditReport, ConfigExplainItem, ConfigFileEntry, DaemonEntry, DepEntry, DoctorReport,
    MiseEnvExtendedMap, MiseTask, MiseToolsMap, MiseVarsMap, OutdatedMap, RegistryEntry,
    SettingEntry, TaskHistoryEntry, ToolDetail, TrustEntry,
};

pub const HOT_TIMEOUT: Duration = Duration::from_secs(5);
pub const SLOW_TIMEOUT: Duration = Duration::from_secs(20);

/// Result of executing a `mise` CLI command along with its audit metadata.
#[derive(Debug, Clone)]
pub struct MiseExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u128,
    pub command_display: String,
}

impl MiseExecOutput {
    pub fn to_log_entry(&self, id: u64, source: CommandSource) -> CommandLogEntry {
        self.to_log_entry_with_summary(id, source, None)
    }

    pub fn to_log_entry_with_summary(
        &self,
        id: u64,
        source: CommandSource,
        summary: Option<String>,
    ) -> CommandLogEntry {
        let status = if self.exit_code == 0 {
            JobStatus::Success {
                duration_ms: self.duration_ms,
            }
        } else {
            JobStatus::Failed {
                code: self.exit_code,
                duration_ms: self.duration_ms,
            }
        };
        CommandLogEntry {
            id,
            timestamp: SystemTime::now(),
            source,
            command: self.command_display.clone(),
            job_id: None,
            status,
            summary,
        }
    }

    /// Construct a `CommandLogEntry` for a command that timed out or failed to spawn.
    pub fn error_log_entry(
        id: u64,
        source: CommandSource,
        command_display: String,
        duration_ms: u128,
        summary: String,
    ) -> CommandLogEntry {
        CommandLogEntry {
            id,
            timestamp: SystemTime::now(),
            source,
            command: command_display,
            job_id: None,
            status: JobStatus::Failed {
                code: -1,
                duration_ms,
            },
            summary: Some(summary),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MiseClient {
    workdir: PathBuf,
    profile: ProfileSpec,
}

impl MiseClient {
    pub fn new(workdir: impl AsRef<Path>) -> Self {
        Self {
            workdir: workdir.as_ref().to_path_buf(),
            profile: None,
        }
    }

    pub fn with_profile(workdir: impl AsRef<Path>, profile: ProfileSpec) -> Self {
        Self {
            workdir: workdir.as_ref().to_path_buf(),
            profile: profile.filter(|s| !s.trim().is_empty()),
        }
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    pub fn profile(&self) -> Option<&str> {
        self.profile.as_deref()
    }

    pub fn set_profile(&mut self, profile: ProfileSpec) {
        self.profile = profile.filter(|s| !s.trim().is_empty());
    }

    /// Build full argument list prefixed with `-C <workdir>` and optional `-E <profile>`.
    pub fn build_args(&self, args: &[&str]) -> Vec<String> {
        let mut full = Vec::with_capacity(args.len() + 4);
        full.push("-C".to_string());
        full.push(self.workdir.to_string_lossy().into_owned());
        if let Some(env) = &self.profile {
            full.push("-E".to_string());
            full.push(env.clone());
        }
        full.extend(args.iter().map(|s| (*s).to_string()));
        full
    }

    /// Discover candidate `MISE_ENV` profiles in the current workspace directory.
    pub fn discover_profiles(&self) -> Vec<String> {
        discover_profiles_in(&self.workdir)
    }

    // -------------------------------------------------------------------------
    // Tasks domain
    // -------------------------------------------------------------------------

    pub fn tasks_ls_with_output(
        &self,
        include_hidden: bool,
        all_monorepo: bool,
    ) -> Result<(Vec<MiseTask>, MiseExecOutput)> {
        let mut args = vec!["tasks", "ls", "--json"];
        if include_hidden {
            args.push("--hidden");
        }
        if all_monorepo {
            args.push("--all");
        }
        let out = self.run_raw(&args, HOT_TIMEOUT)?;
        if out.exit_code != 0 {
            bail!(
                "`{}` exited {}: {}",
                out.command_display,
                out.exit_code,
                out.stderr.trim()
            );
        }
        if out.stdout.trim().is_empty() {
            return Ok((Vec::new(), out));
        }
        let tasks =
            serde_json::from_str(&out.stdout).context("failed to parse `mise tasks ls --json`")?;
        Ok((tasks, out))
    }

    pub fn tasks_ls(&self, include_hidden: bool, all_monorepo: bool) -> Result<Vec<MiseTask>> {
        self.tasks_ls_with_output(include_hidden, all_monorepo)
            .map(|(t, _)| t)
    }

    pub fn list_tasks(&self, include_hidden: bool, all_monorepo: bool) -> Result<Vec<MiseTask>> {
        self.tasks_ls(include_hidden, all_monorepo)
    }

    pub fn task_info_with_output(&self, task_name: &str) -> Result<(MiseTask, MiseExecOutput)> {
        self.run_json(
            &["tasks", "info", task_name, "--json"],
            HOT_TIMEOUT,
            "mise tasks info --json",
        )
    }

    pub fn task_info(&self, task_name: &str) -> Result<MiseTask> {
        self.task_info_with_output(task_name).map(|(t, _)| t)
    }

    pub fn task_deps_with_output(&self, task_name: &str) -> Result<(Vec<String>, MiseExecOutput)> {
        let out = self.run_raw(&["tasks", "deps", task_name], HOT_TIMEOUT)?;
        if out.exit_code != 0 {
            bail!(
                "`{}` exited {}: {}",
                out.command_display,
                out.exit_code,
                out.stderr.trim()
            );
        }
        let lines = parse_task_deps(&out.stdout);
        Ok((lines, out))
    }

    pub fn task_deps(&self, task_name: &str) -> Result<Vec<String>> {
        self.task_deps_with_output(task_name).map(|(d, _)| d)
    }

    pub fn task_history_with_output(
        &self,
        task_name: Option<&str>,
    ) -> Result<(Vec<TaskHistoryEntry>, MiseExecOutput)> {
        let mut args = vec!["tasks", "history", "--json"];
        if let Some(t) = task_name.filter(|s| !s.is_empty()) {
            args.push(t);
        }
        let out = self.run_raw(&args, HOT_TIMEOUT)?;
        if out.exit_code != 0 || out.stdout.trim().is_empty() {
            return Ok((Vec::new(), out));
        }
        let entries = serde_json::from_str(&out.stdout)
            .context("failed to parse `mise tasks history --json`")?;
        Ok((entries, out))
    }

    pub fn task_history(&self, task_name: Option<&str>) -> Result<Vec<TaskHistoryEntry>> {
        self.task_history_with_output(task_name).map(|(h, _)| h)
    }

    // -------------------------------------------------------------------------
    // Tools domain
    // -------------------------------------------------------------------------

    pub fn tools_ls_with_output(&self) -> Result<(MiseToolsMap, MiseExecOutput)> {
        let out = self.run_raw(&["ls", "--json"], HOT_TIMEOUT)?;
        if out.exit_code != 0 {
            bail!(
                "`{}` exited {}: {}",
                out.command_display,
                out.exit_code,
                out.stderr.trim()
            );
        }
        if out.stdout.trim().is_empty() {
            return Ok((MiseToolsMap::new(), out));
        }
        let tools = serde_json::from_str(&out.stdout).context("failed to parse `mise ls --json`")?;
        Ok((tools, out))
    }

    pub fn tools_ls(&self) -> Result<MiseToolsMap> {
        self.tools_ls_with_output().map(|(t, _)| t)
    }

    pub fn list_tools(&self) -> Result<MiseToolsMap> {
        self.tools_ls()
    }

    /// List only non-global tools using `--no-global` (never deprecated `--local`).
    pub fn tools_ls_no_global(&self) -> Result<(MiseToolsMap, MiseExecOutput)> {
        let out = self.run_raw(&["ls", "--no-global", "--json"], HOT_TIMEOUT)?;
        if out.exit_code != 0 {
            bail!(
                "`{}` exited {}: {}",
                out.command_display,
                out.exit_code,
                out.stderr.trim()
            );
        }
        if out.stdout.trim().is_empty() {
            return Ok((MiseToolsMap::new(), out));
        }
        let tools = serde_json::from_str(&out.stdout)
            .context("failed to parse `mise ls --no-global --json`")?;
        Ok((tools, out))
    }

    pub fn tool_detail_with_output(&self, name: &str) -> Result<(ToolDetail, MiseExecOutput)> {
        self.run_json(&["tool", name, "--json"], HOT_TIMEOUT, "mise tool --json")
    }

    pub fn tool_detail(&self, name: &str) -> Result<ToolDetail> {
        self.tool_detail_with_output(name).map(|(d, _)| d)
    }

    pub fn registry_with_output(&self) -> Result<(Vec<RegistryEntry>, MiseExecOutput)> {
        let out = self.run_raw(&["registry", "--json"], HOT_TIMEOUT)?;
        if out.exit_code != 0 || out.stdout.trim().is_empty() {
            return Ok((Vec::new(), out));
        }
        let list =
            serde_json::from_str(&out.stdout).context("failed to parse `mise registry --json`")?;
        Ok((list, out))
    }

    pub fn registry(&self) -> Result<Vec<RegistryEntry>> {
        self.registry_with_output().map(|(r, _)| r)
    }

    pub fn list_registry(&self) -> Result<Vec<RegistryEntry>> {
        self.registry()
    }

    pub fn tool_versions_with_output(&self, tool: &str) -> Result<(Vec<String>, MiseExecOutput)> {
        let out = self.run_raw(&["ls-remote", tool], SLOW_TIMEOUT)?;
        if out.exit_code != 0 {
            bail!(
                "`{}` exited {}: {}",
                out.command_display,
                out.exit_code,
                out.stderr.trim()
            );
        }
        let versions = out
            .stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        Ok((versions, out))
    }

    pub fn tool_versions(&self, tool: &str) -> Result<Vec<String>> {
        self.tool_versions_with_output(tool).map(|(v, _)| v)
    }

    pub fn ls_remote(&self, tool: &str) -> Result<Vec<String>> {
        self.tool_versions(tool)
    }

    pub fn outdated_with_output(&self) -> Result<(OutdatedMap, MiseExecOutput)> {
        let out = self.run_raw(&["outdated", "--json"], SLOW_TIMEOUT)?;
        if out.exit_code != 0 {
            bail!(
                "`{}` exited {}: {}",
                out.command_display,
                out.exit_code,
                out.stderr.trim()
            );
        }
        if out.stdout.trim().is_empty() {
            return Ok((OutdatedMap::new(), out));
        }
        let map =
            serde_json::from_str(&out.stdout).context("failed to parse `mise outdated --json`")?;
        Ok((map, out))
    }

    pub fn outdated(&self) -> Result<OutdatedMap> {
        self.outdated_with_output().map(|(o, _)| o)
    }

    pub fn list_outdated(&self) -> Result<OutdatedMap> {
        self.outdated()
    }

    /// Run `mise audit --json`. Note: exit code 1 still produces valid JSON when CVEs exist.
    pub fn audit_with_output(&self) -> Result<(AuditReport, MiseExecOutput)> {
        self.run_json_allowing_nonzero(&["audit", "--json"], SLOW_TIMEOUT, "mise audit --json")
    }

    pub fn audit(&self) -> Result<AuditReport> {
        self.audit_with_output().map(|(a, _)| a)
    }

    pub fn run_audit(&self) -> Result<AuditReport> {
        self.audit()
    }

    pub fn lock_dry_run_with_output(&self) -> Result<(String, MiseExecOutput)> {
        let out = self.run_raw(&["lock", "--dry-run"], SLOW_TIMEOUT)?;
        if out.exit_code != 0 && out.stdout.trim().is_empty() {
            bail!(
                "`{}` exited {}: {}",
                out.command_display,
                out.exit_code,
                out.stderr.trim()
            );
        }
        let text = if out.stdout.trim().is_empty() {
            out.stderr.clone()
        } else {
            out.stdout.clone()
        };
        Ok((text, out))
    }

    pub fn lock_dry_run(&self) -> Result<String> {
        self.lock_dry_run_with_output().map(|(s, _)| s)
    }

    // -------------------------------------------------------------------------
    // Env domain
    // -------------------------------------------------------------------------

    pub fn env_extended_with_output(&self) -> Result<(MiseEnvExtendedMap, MiseExecOutput)> {
        let out = self.run_raw(&["env", "--json-extended"], HOT_TIMEOUT)?;
        if out.exit_code != 0 {
            bail!(
                "`{}` exited {}: {}",
                out.command_display,
                out.exit_code,
                out.stderr.trim()
            );
        }
        if out.stdout.trim().is_empty() {
            return Ok((MiseEnvExtendedMap::new(), out));
        }
        let map = serde_json::from_str(&out.stdout)
            .context("failed to parse `mise env --json-extended`")?;
        Ok((map, out))
    }

    pub fn env_extended(&self) -> Result<MiseEnvExtendedMap> {
        self.env_extended_with_output().map(|(m, _)| m)
    }

    pub fn list_env_extended(&self) -> Result<MiseEnvExtendedMap> {
        self.env_extended()
    }

    pub fn env_redacted_with_output(&self) -> Result<(BTreeSet<String>, MiseExecOutput)> {
        let out = self.run_raw(&["env", "--redacted", "--json"], HOT_TIMEOUT)?;
        if out.exit_code != 0 || out.stdout.trim().is_empty() {
            return Ok((BTreeSet::new(), out));
        }
        let map: BTreeMap<String, String> = serde_json::from_str(&out.stdout).unwrap_or_default();
        Ok((map.into_keys().collect(), out))
    }

    pub fn env_redacted(&self) -> Result<BTreeSet<String>> {
        self.env_redacted_with_output().map(|(s, _)| s)
    }

    pub fn list_env_redacted_keys(&self) -> Result<BTreeSet<String>> {
        self.env_redacted()
    }

    pub fn env_paths_with_output(&self) -> Result<(Vec<String>, MiseExecOutput)> {
        let out = self.run_raw(&["env", "--paths"], HOT_TIMEOUT)?;
        if out.exit_code != 0 {
            return Ok((Vec::new(), out));
        }
        let paths = parse_env_paths(&out.stdout);
        Ok((paths, out))
    }

    pub fn env_paths(&self) -> Result<Vec<String>> {
        self.env_paths_with_output().map(|(p, _)| p)
    }

    pub fn list_env_paths(&self) -> Result<Vec<String>> {
        self.env_paths()
    }

    pub fn vars_with_output(&self) -> Result<(MiseVarsMap, MiseExecOutput)> {
        let out = self.run_raw(&["vars", "--json"], HOT_TIMEOUT)?;
        if out.exit_code != 0 || out.stdout.trim().is_empty() {
            return Ok((MiseVarsMap::new(), out));
        }
        let map = serde_json::from_str(&out.stdout).context("failed to parse `mise vars --json`")?;
        Ok((map, out))
    }

    pub fn vars(&self) -> Result<MiseVarsMap> {
        self.vars_with_output().map(|(v, _)| v)
    }

    pub fn list_vars(&self) -> Result<MiseVarsMap> {
        self.vars()
    }

    // -------------------------------------------------------------------------
    // Config, Doctor, Settings, Daemons, Deps domains
    // -------------------------------------------------------------------------

    pub fn config_ls_with_output(&self) -> Result<(Vec<ConfigFileEntry>, MiseExecOutput)> {
        let out = self.run_raw(&["config", "ls", "--json"], HOT_TIMEOUT)?;
        if out.exit_code != 0 || out.stdout.trim().is_empty() {
            return Ok((Vec::new(), out));
        }
        let files =
            serde_json::from_str(&out.stdout).context("failed to parse `mise config ls --json`")?;
        Ok((files, out))
    }

    pub fn config_ls(&self) -> Result<Vec<ConfigFileEntry>> {
        self.config_ls_with_output().map(|(c, _)| c)
    }

    pub fn list_configs(&self) -> Result<Vec<ConfigFileEntry>> {
        self.config_ls()
    }

    pub fn trust_show_with_output(&self) -> Result<(Vec<TrustEntry>, MiseExecOutput)> {
        let out = self.run_raw(&["trust", "--show"], HOT_TIMEOUT)?;
        if out.exit_code != 0 {
            return Ok((Vec::new(), out));
        }
        let entries = parse_trust_show(&out.stdout);
        Ok((entries, out))
    }

    pub fn trust_show(&self) -> Result<Vec<TrustEntry>> {
        self.trust_show_with_output().map(|(t, _)| t)
    }

    pub fn list_trust(&self) -> Result<Vec<TrustEntry>> {
        self.trust_show()
    }

    pub fn config_explain_with_output(&self) -> Result<(Vec<ConfigExplainItem>, MiseExecOutput)> {
        let out = self.run_raw(&["config", "explain", "--json"], HOT_TIMEOUT)?;
        if out.exit_code != 0 || out.stdout.trim().is_empty() {
            return Ok((Vec::new(), out));
        }
        let items = serde_json::from_str(&out.stdout)
            .context("failed to parse `mise config explain --json`")?;
        Ok((items, out))
    }

    pub fn config_explain(&self) -> Result<Vec<ConfigExplainItem>> {
        self.config_explain_with_output().map(|(items, _)| items)
    }

    /// Run `mise doctor --json`. Note: exit code 1 still produces valid JSON when errors exist.
    pub fn doctor_with_output(&self) -> Result<(DoctorReport, MiseExecOutput)> {
        self.run_json_allowing_nonzero(&["doctor", "--json"], SLOW_TIMEOUT, "mise doctor --json")
    }

    pub fn doctor(&self) -> Result<DoctorReport> {
        self.doctor_with_output().map(|(d, _)| d)
    }

    pub fn run_doctor(&self) -> Result<DoctorReport> {
        self.doctor()
    }

    pub fn settings_extended_with_output(&self) -> Result<(Vec<SettingEntry>, MiseExecOutput)> {
        let out = self.run_raw(&["settings", "ls", "--json-extended"], HOT_TIMEOUT)?;
        if out.exit_code != 0 || out.stdout.trim().is_empty() {
            return Ok((Vec::new(), out));
        }
        let entries = parse_settings_extended_json(&out.stdout)?;
        Ok((entries, out))
    }

    pub fn settings_extended(&self) -> Result<Vec<SettingEntry>> {
        self.settings_extended_with_output().map(|(s, _)| s)
    }

    pub fn list_settings(&self) -> Result<Vec<SettingEntry>> {
        self.settings_extended()
    }

    /// Query `mise daemons --json`; returns an empty list silently when `[daemons]` is unconfigured.
    pub fn daemons_with_output(&self) -> Result<(Vec<DaemonEntry>, MiseExecOutput)> {
        let out = self.run_raw(&["daemons", "--json"], HOT_TIMEOUT)?;
        if out.exit_code != 0 || out.stdout.trim().is_empty() {
            return Ok((Vec::new(), out));
        }
        let entries = parse_daemons_json(&out.stdout);
        Ok((entries, out))
    }

    pub fn daemons(&self) -> Result<Vec<DaemonEntry>> {
        self.daemons_with_output().map(|(d, _)| d)
    }

    pub fn list_daemons(&self) -> Result<Vec<DaemonEntry>> {
        self.daemons()
    }

    /// Query `mise deps --list`; returns an empty list silently when no providers are configured.
    pub fn deps_list_with_output(&self) -> Result<(Vec<DepEntry>, MiseExecOutput)> {
        let out = self.run_raw(&["deps", "--list"], HOT_TIMEOUT)?;
        if out.exit_code != 0 {
            return Ok((Vec::new(), out));
        }
        let entries = parse_deps_list(&out.stdout);
        Ok((entries, out))
    }

    pub fn deps_list(&self) -> Result<Vec<DepEntry>> {
        self.deps_list_with_output().map(|(d, _)| d)
    }

    pub fn list_deps(&self) -> Result<Vec<DepEntry>> {
        self.deps_list()
    }

    // -------------------------------------------------------------------------
    // Core subprocess + JSON helpers
    // -------------------------------------------------------------------------

    pub fn run_json<T: DeserializeOwned>(
        &self,
        args: &[&str],
        timeout: Duration,
        label: &str,
    ) -> Result<(T, MiseExecOutput)> {
        let out = self.run_raw(args, timeout)?;
        if out.exit_code != 0 {
            bail!(
                "`{}` exited {}: {}",
                out.command_display,
                out.exit_code,
                out.stderr.trim()
            );
        }
        let parsed =
            serde_json::from_str(&out.stdout).with_context(|| format!("failed to parse `{label}`"))?;
        Ok((parsed, out))
    }

    /// Execute a `mise` command and parse stdout JSON even when exit code is non-zero
    /// (required for `mise doctor --json` and `mise audit --json`).
    pub fn run_json_allowing_nonzero<T: DeserializeOwned>(
        &self,
        args: &[&str],
        timeout: Duration,
        label: &str,
    ) -> Result<(T, MiseExecOutput)> {
        let out = self.run_raw(args, timeout)?;
        let parsed = self.parse_json_allowing_nonzero(&out, label)?;
        Ok((parsed, out))
    }

    pub fn parse_json_allowing_nonzero<T: DeserializeOwned>(
        &self,
        out: &MiseExecOutput,
        label: &str,
    ) -> Result<T> {
        let trimmed = out.stdout.trim();
        if !trimmed.is_empty()
            && let Ok(parsed) = serde_json::from_str::<T>(trimmed)
        {
            return Ok(parsed);
        }
        bail!(
            "`{}` exited {} (failed to parse `{label}`): {}",
            out.command_display,
            out.exit_code,
            out.stderr.trim()
        );
    }

    pub fn run_raw(&self, args: &[&str], timeout: Duration) -> Result<MiseExecOutput> {
        let full_args = self.build_args(args);
        run_command_with_timeout("mise", &full_args, &self.workdir, timeout)
    }
}

/// Spawn an external command with concurrent stdout/stderr draining and strict timeout enforcement.
/// On Unix, places the child into its own process group so timeout kills the entire subtree cleanly.
pub fn run_command_with_timeout(
    program: &str,
    args: &[String],
    workdir: &Path,
    timeout: Duration,
) -> Result<MiseExecOutput> {
    let command_display = if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", args.join(" "))
    };

    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let start = Instant::now();
    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to execute `{command_display}`"))?;

    let child_pid = child.id();
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let stdout_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stdout_pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });
    let stderr_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stderr_pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        buf
    });

    let mut sleep_ms = 2u64;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    kill_child_tree(&mut child, child_pid);
                    break Err(anyhow::anyhow!(
                        "`{command_display}` timed out after {}ms",
                        timeout.as_millis()
                    ));
                }
                thread::sleep(Duration::from_millis(sleep_ms));
                sleep_ms = (sleep_ms * 2).min(15);
            }
            Err(err) => {
                kill_child_tree(&mut child, child_pid);
                break Err(anyhow::anyhow!("failed waiting for `{command_display}`: {err}"));
            }
        }
    };

    let stdout_bytes = stdout_handle.join().unwrap_or_default();
    let stderr_bytes = stderr_handle.join().unwrap_or_default();
    let exit_status = status?;

    Ok(MiseExecOutput {
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        exit_code: exit_status.code().unwrap_or(1),
        duration_ms: start.elapsed().as_millis(),
        command_display,
    })
}

fn kill_child_tree(child: &mut std::process::Child, pid: u32) {
    #[cfg(unix)]
    {
        if pid > 0 {
            unsafe {
                libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
            }
        }
    }
    let _ = pid;
    let _ = child.kill();
    let _ = child.wait();
}

/// Scan `dir` (and optional `.mise/` / `mise/` subdirectories) for candidate `MISE_ENV` profiles:
/// - `mise.<env>.toml`, `.mise.<env>.toml`, `mise.<env>.local.toml`, `.mise.<env>.local.toml`
/// - `.mise/config.<env>.toml`, `mise/config.<env>.toml` (+ `.local.toml`)
///
/// Excludes non-profile files such as `mise.local.toml`, `mise.lock`, and `config.override.toml`.
pub fn discover_profiles_in(dir: &Path) -> Vec<String> {
    let mut profiles = BTreeSet::new();
    scan_profile_dir(dir, &["mise.", ".mise."], &mut profiles);
    scan_profile_dir(&dir.join(".mise"), &["config.", "mise."], &mut profiles);
    scan_profile_dir(&dir.join("mise"), &["config.", "mise."], &mut profiles);
    profiles.into_iter().collect()
}

fn scan_profile_dir(dir: &Path, prefixes: &[&str], out: &mut BTreeSet<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(fname) = name.to_str() else {
            continue;
        };
        if let Some(profile) = extract_profile_from_filename(fname, prefixes) {
            out.insert(profile);
        }
    }
}

fn extract_profile_from_filename(fname: &str, prefixes: &[&str]) -> Option<String> {
    let body = fname.strip_suffix(".toml")?;
    for prefix in prefixes {
        let Some(rest) = body.strip_prefix(prefix) else {
            continue;
        };
        let rest = rest.strip_suffix(".local").unwrap_or(rest);
        if rest.is_empty()
            || rest.eq_ignore_ascii_case("local")
            || rest.eq_ignore_ascii_case("override")
            || rest.eq_ignore_ascii_case("lock")
            || rest.contains('.')
            || rest.chars().any(char::is_whitespace)
        {
            return None;
        }
        return Some(rest.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use pretty_assertions::assert_eq;

    use super::schema::{
        AuditReport, ConfigExplainItem, ConfigFileEntry, DoctorReport, MiseEnvExtendedMap,
        MiseTask, MiseToolsMap, OutdatedMap, RegistryEntry, TaskHistoryEntry, ToolDetail,
        TrustEntry,
    };
    use super::*;

    fn temp_test_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("lazymise-{prefix}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn build_args_injects_cd_and_optional_profile() {
        let client = MiseClient::new("/tmp/my-project");
        assert_eq!(
            client.build_args(&["tasks", "ls", "--json"]),
            vec!["-C", "/tmp/my-project", "tasks", "ls", "--json"]
        );

        let with_env = MiseClient::with_profile("/tmp/my-project", Some("staging".to_string()));
        assert_eq!(
            with_env.build_args(&["ls", "--no-global", "--json"]),
            vec![
                "-C",
                "/tmp/my-project",
                "-E",
                "staging",
                "ls",
                "--no-global",
                "--json"
            ]
        );

        let blank_env = MiseClient::with_profile("/tmp/my-project", Some("   ".to_string()));
        assert_eq!(blank_env.profile(), None);
    }

    #[test]
    fn parses_real_tasks_ls_json_with_polymorphic_fields() {
        let raw = r#"[
          {
            "name": "build",
            "aliases": ["b"],
            "tags": ["release"],
            "description": "Build release binary",
            "source": "/Users/qlong/MyCli/lazymise/mise.toml",
            "config_sources": ["/Users/qlong/MyCli/lazymise/mise.toml"],
            "depends": ["tidy", ["lint", "--fast"], {"task": "gen", "args": ["--check"]}],
            "daemons": null,
            "secrets": ["GITHUB_TOKEN"],
            "depends_post": ["notify"],
            "wait_for": ["db:ready"],
            "env": [],
            "dir": "/Users/qlong/MyCli/lazymise",
            "hide": false,
            "global": false,
            "raw": false,
            "interactive": false,
            "sources": ["Cargo.toml", "src/**/*.rs"],
            "outputs": ["target/release/lazymise"],
            "cache": {"enabled": true, "audit": false, "env": ["RUSTFLAGS"], "command_inputs": []},
            "shell": null,
            "skip_if": null,
            "lock": null,
            "quiet": false,
            "silent": "stdout",
            "tools": {"rust": "stable", "node": ["20", "22"]},
            "usage": "flag \"--release\"",
            "timeout": "5m",
            "run": ["cargo build --release", {"task": "post-check"}],
            "args": [],
            "file": null,
            "history": {
              "task": "build",
              "runs": 3,
              "failures": 1,
              "median_ms": 32568,
              "last_ms": 31000,
              "last_status": "ok",
              "last_started_at": "2026-11-26T05:05:49.238Z"
            }
          },
          {
            "name": "//apps/web:dev",
            "description": "Web dev server",
            "source": "/repo/apps/web/mise.toml",
            "outputs": {"auto": true},
            "cache": true,
            "run": "pnpm dev"
          }
        ]"#;

        let tasks: Vec<MiseTask> = serde_json::from_str(raw).unwrap();
        assert_eq!(tasks.len(), 2);

        let build = &tasks[0];
        assert_eq!(build.name, "build");
        assert_eq!(build.tags, vec!["release"]);
        assert_eq!(
            build.depends_display(),
            vec!["tidy", "lint --fast", "gen --check"]
        );
        assert_eq!(build.depends_post_display(), vec!["notify"]);
        assert_eq!(build.wait_for_display(), vec!["db:ready"]);
        assert_eq!(build.outputs_display(), vec!["target/release/lazymise"]);
        assert!(build.cache_enabled());
        assert!(build.is_silent());
        assert_eq!(build.timeout_display().as_deref(), Some("5m"));
        assert_eq!(build.tools["rust"], "stable");
        assert_eq!(build.tools["node"], "20, 22");
        assert_eq!(build.run, vec!["cargo build --release", "task: post-check"]);
        assert_eq!(build.history.as_ref().unwrap().runs, 3);
        assert_eq!(build.editable_path(), Some("/Users/qlong/MyCli/lazymise/mise.toml"));

        let web = &tasks[1];
        assert_eq!(web.group_key(), "//apps/web");
        assert_eq!(web.short_name(), "dev");
        assert_eq!(web.outputs_display(), vec!["auto: true"]);
        assert!(web.cache_enabled());
        assert_eq!(web.run, vec!["pnpm dev"]);
    }

    #[test]
    fn parses_real_task_info_and_history_json() {
        let info_raw = r#"{
          "name": "deploy",
          "description": "Deploy service",
          "source": "/repo/.mise/tasks/deploy",
          "file": "/repo/.mise/tasks/deploy",
          "usage_spec": {
            "name": "deploy",
            "bin": "deploy",
            "cmd": {
              "args": [
                {"name": "env", "usage": "<env>", "help": "Target env", "required": true, "var": false, "default": "staging"}
              ],
              "flags": [
                {"name": "dry-run", "short": ["n"], "long": ["dry-run"], "usage": "-n --dry-run", "required": false, "default": null}
              ],
              "help": "Deploy service"
            }
          }
        }"#;
        let task: MiseTask = serde_json::from_str(info_raw).unwrap();
        let spec = task.usage_spec.as_ref().unwrap();
        assert!(spec.has_required_inputs());
        assert!(spec.has_any_inputs());
        assert_eq!(spec.cmd.args[0].default.as_deref(), Some("staging"));
        assert_eq!(spec.cmd.flags[0].short, vec!["n"]);

        let hist_raw = r#"[
          {
            "task": "tidy",
            "args": ["--strict"],
            "started_at": "2026-11-26T05:00:32.181Z",
            "duration_ms": 764,
            "status": "ok",
            "exit_code": 0
          }
        ]"#;
        let hist: Vec<TaskHistoryEntry> = serde_json::from_str(hist_raw).unwrap();
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].task, "tidy");
        assert_eq!(hist[0].duration_ms, 764);
        assert_eq!(hist[0].exit_code, Some(0));
    }

    #[test]
    fn parses_real_tools_detail_outdated_registry_json() {
        let ls_raw = r#"{
          "antigravity-cli": [
            {
              "version": "1.5.16",
              "install_path": "/Users/qlong/.local/share/mise/installs/antigravity-cli/1.5.16",
              "scheduled_for_pruning": true,
              "prune_after": "2026-11-26T09:38:37Z",
              "installed": true,
              "active": false
            },
            {
              "version": "1.5.17",
              "requested_version": "latest",
              "install_path": "/Users/qlong/.local/share/mise/installs/antigravity-cli/1.5.17",
              "source": {"type": "mise.toml", "path": "/Users/qlong/.config/mise/config.toml"},
              "installed": true,
              "active": true
            }
          ]
        }"#;
        let tools: MiseToolsMap = serde_json::from_str(ls_raw).unwrap();
        assert_eq!(tools["antigravity-cli"].len(), 2);
        assert!(tools["antigravity-cli"][0].scheduled_for_pruning);
        assert_eq!(
            tools["antigravity-cli"][0].prune_after.as_deref(),
            Some("2026-11-26T09:38:37Z")
        );

        let detail_raw = r#"{
          "backend": "core:rust",
          "description": null,
          "url": null,
          "installed_versions": ["stable", "1.96.0"],
          "requested_versions": ["stable"],
          "active_versions": ["stable"],
          "config_source": {"type": "mise.toml", "path": "/Users/qlong/MyCli/lazymise/mise.toml"},
          "tool_options": {"os": null, "lazy": null},
          "security": []
        }"#;
        let detail: ToolDetail = serde_json::from_str(detail_raw).unwrap();
        assert_eq!(detail.backend.as_deref(), Some("core:rust"));
        assert_eq!(detail.installed_versions, vec!["stable", "1.96.0"]);

        let outdated_raw = r#"{
          "uv": {
            "name": "uv",
            "requested": "0.13.9",
            "current": "0.13.9",
            "bump": "0.13.10",
            "latest": "0.13.10",
            "release_url": "https://github.com/astral-sh/uv/releases/tag/0.13.10",
            "source": {"type": "unknown"}
          }
        }"#;
        let outdated: OutdatedMap = serde_json::from_str(outdated_raw).unwrap();
        assert_eq!(outdated["uv"].latest, "0.13.10");
        assert_eq!(outdated["uv"].bump.as_deref(), Some("0.13.10"));

        let reg_raw = r#"[
          {
            "short": "1password",
            "backends": ["vfox:jdx/vfox-1password", "aqua:1password/cli"],
            "bins": ["op"],
            "description": "Password manager",
            "aliases": ["1password-cli", "op"]
          }
        ]"#;
        let reg: Vec<RegistryEntry> = serde_json::from_str(reg_raw).unwrap();
        assert_eq!(reg[0].short, "1password");
        assert_eq!(reg[0].bins, vec!["op"]);
    }

    #[test]
    fn parses_audit_and_doctor_json_even_with_exit_code_1() {
        let client = MiseClient::new("/tmp");

        let audit_out = MiseExecOutput {
            stdout: r#"{
              "vulnerabilities": [
                {
                  "tool": "python",
                  "version": "3.15.2",
                  "id": "CVE-2026-124249",
                  "aliases": [],
                  "severity": null,
                  "summary": "cookiejar issue",
                  "fixed": [],
                  "url": "https://osv.dev/vulnerability/CVE-2026-124249"
                }
              ],
              "unchecked": [{"tool": "rust", "version": "stable"}]
            }"#
            .to_string(),
            stderr: String::new(),
            exit_code: 1,
            duration_ms: 2500,
            command_display: "mise audit --json".to_string(),
        };
        let audit: AuditReport = client
            .parse_json_allowing_nonzero(&audit_out, "mise audit --json")
            .unwrap();
        assert_eq!(audit.vulnerabilities.len(), 1);
        assert_eq!(audit.vulnerabilities[0].id, "CVE-2026-124249");
        assert_eq!(audit.unchecked.len(), 1);

        let doctor_out = MiseExecOutput {
            stdout: r#"{
              "activated": true,
              "version": "2026.11.13 macos-arm64 (2026-11-23)",
              "shell": {"name": "zsh", "version": "5.9"},
              "config_files": ["/Users/qlong/MyCli/lazymise/mise.toml"],
              "shims_on_path": true,
              "self_update_available": true,
              "warnings": ["new mise version available"],
              "errors": ["shims are missing, run mise reshim"]
            }"#
            .to_string(),
            stderr: String::new(),
            exit_code: 1,
            duration_ms: 210,
            command_display: "mise doctor --json".to_string(),
        };
        let doctor: DoctorReport = client
            .parse_json_allowing_nonzero(&doctor_out, "mise doctor --json")
            .unwrap();
        assert!(doctor.activated);
        assert_eq!(doctor.errors.len(), 1);
        assert_eq!(doctor.warnings.len(), 1);
        assert_eq!(doctor.shell.as_ref().unwrap().name, "zsh");
    }

    #[test]
    fn parses_env_extended_config_ls_and_config_explain_json() {
        let env_raw = r#"{
          "CARGO_HOME": {"source": "", "tool": "rust", "value": "/Users/qlong/.cargo"},
          "OUT_DIR": {"source": "/Users/qlong/MyCli/lazymise/mise.toml", "value": "/Users/qlong/MyCli/lazymise/outputs"},
          "PATH": {"value": "/usr/bin:/bin"}
        }"#;
        let env: MiseEnvExtendedMap = serde_json::from_str(env_raw).unwrap();
        assert_eq!(env["CARGO_HOME"].tool.as_deref(), Some("rust"));
        assert_eq!(
            env["OUT_DIR"].source.as_deref(),
            Some("/Users/qlong/MyCli/lazymise/mise.toml")
        );
        assert_eq!(env["PATH"].source, None);

        let cfg_raw = r#"[
          {"path": "/Users/qlong/MyCli/lazymise/mise.toml", "tools": ["rust"], "override": false},
          {"path": "/Users/qlong/.config/mise/config.override.toml", "tools": [], "override": true}
        ]"#;
        let configs: Vec<ConfigFileEntry> = serde_json::from_str(cfg_raw).unwrap();
        assert_eq!(configs.len(), 2);
        assert!(!configs[0].is_override);
        assert!(configs[1].is_override);

        let explain_raw = r#"[
          {"key": "tools.rust", "path": ["tools", "rust"], "line": 5, "description": "core rust", "deprecated": false, "known": true, "suggestion": null}
        ]"#;
        let explain: Vec<ConfigExplainItem> = serde_json::from_str(explain_raw).unwrap();
        assert_eq!(explain[0].line, 5);
        assert!(explain[0].known);
    }

    #[test]
    fn flattens_nested_settings_json_extended() {
        let raw = r#"{
          "auto_install": {
            "value": true,
            "type": "boolean",
            "description": "Install missing tools automatically.",
            "source": "/Users/qlong/.config/mise/config.toml"
          },
          "python": {
            "uv_venv_auto": {
              "value": "source",
              "type": "boolean | string",
              "description": "Activate uv venv.",
              "source": "/Users/qlong/.config/mise/config.toml"
            }
          },
          "self_update": {
            "auto": {
              "value": true,
              "type": "boolean | string",
              "description": "Update mise automatically.",
              "source": "/Users/qlong/.config/mise/config.toml"
            }
          }
        }"#;
        let entries = parse_settings_extended_json(raw).unwrap();
        let keys: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["auto_install", "python.uv_venv_auto", "self_update.auto"]
        );
        assert_eq!(entries[1].value, serde_json::Value::String("source".to_string()));
        assert_eq!(entries[1].kind, "boolean | string");
    }

    #[test]
    fn parses_plain_text_outputs_trust_deps_paths_and_daemons() {
        let trust = parse_trust_show(
            "~/MyCli/lazymise: trusted\n/tmp/a:b/mise.toml: untrusted\n/tmp/c: ignored\n",
        );
        assert_eq!(
            trust,
            vec![
                TrustEntry {
                    path: "~/MyCli/lazymise".to_string(),
                    trusted: true,
                    raw_status: "trusted".to_string(),
                },
                TrustEntry {
                    path: "/tmp/a:b/mise.toml".to_string(),
                    trusted: false,
                    raw_status: "untrusted".to_string(),
                },
                TrustEntry {
                    path: "/tmp/c".to_string(),
                    trusted: false,
                    raw_status: "ignored".to_string(),
                },
            ]
        );

        let tree = parse_task_deps("check\n├── lint\n├── test\n└── tidy\n\n");
        assert_eq!(tree, vec!["check", "├── lint", "├── test", "└── tidy"]);

        let paths = parse_env_paths("/Users/qlong/.cargo/bin\n  \n/usr/local/bin\n");
        assert_eq!(paths, vec!["/Users/qlong/.cargo/bin", "/usr/local/bin"]);

        assert!(parse_deps_list("No deps providers found for this project\n").is_empty());
        let deps = parse_deps_list(
            "Provider   Status   Detail\n--------   ------   ------\nuv         fresh    uv.lock -> .venv\nnpm        stale    package-lock.json\n",
        );
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "uv");
        assert_eq!(deps[0].status, "fresh");
        assert_eq!(deps[0].detail, "uv.lock -> .venv");
        assert_eq!(deps[1].name, "npm");
        assert_eq!(deps[1].status, "stale");

        let daemons_map =
            parse_daemons_json(r#"{"postgres":{"status":"running","pid":4242,"url":"postgres://localhost:5432"}}"#);
        assert_eq!(daemons_map.len(), 1);
        assert_eq!(daemons_map[0].name, "postgres");
        assert_eq!(daemons_map[0].pid, Some(4242));
    }

    #[test]
    fn discovers_candidate_profiles_excluding_local_and_lock() {
        let dir = temp_test_dir("profiles");
        fs::create_dir_all(dir.join(".mise")).unwrap();

        for fname in [
            "mise.toml",
            "mise.local.toml",
            ".mise.local.toml",
            "mise.lock",
            "mise.dev.toml",
            "mise.prod.toml",
            ".mise.staging.toml",
            "mise.ci.local.toml",
            "mise.foo.bar.toml",
        ] {
            fs::write(dir.join(fname), "").unwrap();
        }
        fs::write(dir.join(".mise/config.test.toml"), "").unwrap();
        fs::write(dir.join(".mise/config.override.toml"), "").unwrap();

        let profiles = discover_profiles_in(&dir);
        assert_eq!(profiles, vec!["ci", "dev", "prod", "staging", "test"]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_command_with_timeout_kills_hanging_process() {
        let dir = std::env::temp_dir();
        let args = vec!["-c".to_string(), "sleep 5".to_string()];
        let start = Instant::now();
        let err = run_command_with_timeout("sh", &args, &dir, Duration::from_millis(120))
            .expect_err("sleeping command must time out");
        let elapsed = start.elapsed();

        assert!(
            err.to_string().contains("timed out"),
            "unexpected error: {err}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "timeout took too long: {elapsed:?}"
        );
    }
}
