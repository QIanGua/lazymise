use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::{CommandExt, ExitStatusExt};

use crate::action::ProfileSpec;
use crate::events::{AppEvent, JobId, JobOutputLine, RunnerEvent};

const SIGKILL_GRACE: Duration = Duration::from_millis(400);
const KILL_POLL_INTERVAL: Duration = Duration::from_millis(20);
const MAX_LINE_BYTES: usize = 65_536;

#[derive(Debug)]
struct ActiveJob {
    pid: Option<u32>,
    killed: Arc<AtomicBool>,
}

/// Manages concurrent background processes with unique `JobId`s, Unix process-group isolation,
/// graceful kill (`SIGTERM` -> `SIGKILL`), UTF-8-safe streaming with `\r` progress overwrite,
/// and automatic cleanup on `Drop`.
pub struct BackgroundRunner {
    tx: Sender<AppEvent>,
    next_id: AtomicU64,
    active_jobs: Arc<Mutex<HashMap<JobId, ActiveJob>>>,
}

impl BackgroundRunner {
    pub fn new(tx: Sender<AppEvent>) -> Self {
        Self {
            tx,
            next_id: AtomicU64::new(1),
            active_jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn allocate_job_id(&self) -> JobId {
        JobId(self.next_id.fetch_add(1, Ordering::SeqCst))
    }

    /// Build `mise` CLI arguments with `-C <workdir>` and optional `-E <profile>` prepended.
    pub fn build_mise_args(
        workdir: &Path,
        profile: &ProfileSpec,
        args: &[String],
    ) -> Vec<String> {
        let mut full = Vec::with_capacity(args.len() + 4);
        full.push("-C".to_string());
        full.push(workdir.display().to_string());
        if let Some(env_name) = profile.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            full.push("-E".to_string());
            full.push(env_name.to_string());
        }
        full.extend(args.iter().cloned());
        full
    }

    /// Spawn a background `mise` command (e.g. `["run", "check"]`), injecting `-C <dir>` and optional `-E <profile>`.
    pub fn spawn_mise_command(
        &self,
        workdir: PathBuf,
        profile: ProfileSpec,
        args: Vec<String>,
    ) -> JobId {
        let full_args = Self::build_mise_args(&workdir, &profile, &args);
        self.spawn_command(workdir, "mise".to_string(), full_args, Vec::new())
    }

    /// Spawn a background `mise run <task> [extra_args...]` job.
    pub fn spawn_mise_task(
        &self,
        workdir: PathBuf,
        profile: ProfileSpec,
        task_name: String,
        extra_args: Vec<String>,
        force: bool,
    ) -> JobId {
        let mut args = vec!["run".to_string()];
        if force {
            args.push("--force".to_string());
        }
        args.push(task_name);
        if !extra_args.is_empty() {
            args.push("--".to_string());
            args.extend(extra_args);
        }
        self.spawn_mise_command(workdir, profile, args)
    }

    /// Spawn an arbitrary background command with process-group isolation and live output streaming.
    pub fn spawn_command(
        &self,
        workdir: PathBuf,
        program: String,
        args: Vec<String>,
        envs: Vec<(String, String)>,
    ) -> JobId {
        let job_id = self.allocate_job_id();
        self.spawn_command_with_id(job_id, workdir, program, args, envs);
        job_id
    }

    /// Spawn a command bound to a pre-allocated `JobId`.
    pub fn spawn_command_with_id(
        &self,
        job_id: JobId,
        workdir: PathBuf,
        program: String,
        args: Vec<String>,
        envs: Vec<(String, String)>,
    ) {
        let killed = Arc::new(AtomicBool::new(false));
        if let Ok(mut map) = self.active_jobs.lock() {
            map.insert(
                job_id,
                ActiveJob {
                    pid: None,
                    killed: Arc::clone(&killed),
                },
            );
        }

        let tx = self.tx.clone();
        let active_jobs = Arc::clone(&self.active_jobs);

        thread::spawn(move || {
            let start = Instant::now();

            if killed.load(Ordering::SeqCst) {
                remove_active_job(&active_jobs, job_id);
                let _ = tx.send(AppEvent::Runner(RunnerEvent::Started {
                    job_id,
                    pid: None,
                }));
                let _ = tx.send(AppEvent::Runner(RunnerEvent::Finished {
                    job_id,
                    exit_code: 143,
                    duration_ms: 0,
                    killed: true,
                }));
                return;
            }

            let mut cmd = Command::new(&program);
            cmd.args(&args)
                .current_dir(&workdir)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            for (k, v) in envs {
                cmd.env(k, v);
            }

            #[cfg(unix)]
            // Create a new session via `setsid()` so the child both becomes its own process-group
            // leader (for `killpg`) and completely detaches from the parent's controlling `/dev/tty`.
            // Without detaching, interactive programs that open `/dev/tty` receive SIGTTIN/SIGTTOU
            // from the kernel and freeze indefinitely in Stopped state.
            unsafe {
                cmd.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }

            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(err) => {
                    remove_active_job(&active_jobs, job_id);
                    let _ = tx.send(AppEvent::Runner(RunnerEvent::Started {
                        job_id,
                        pid: None,
                    }));
                    let _ = tx.send(AppEvent::Runner(RunnerEvent::Output {
                        job_id,
                        line: JobOutputLine {
                            text: format!(
                                "error: failed to spawn `{} {}`: {err}",
                                program,
                                args.join(" ")
                            ),
                            is_stderr: true,
                        },
                        overwrite_last: false,
                    }));
                    let _ = tx.send(AppEvent::Runner(RunnerEvent::Finished {
                        job_id,
                        exit_code: 127,
                        duration_ms: start.elapsed().as_millis(),
                        killed: false,
                    }));
                    return;
                }
            };

            let pid = child.id();
            if let Ok(mut map) = active_jobs.lock()
                && let Some(entry) = map.get_mut(&job_id)
            {
                entry.pid = Some(pid);
            }

            let _ = tx.send(AppEvent::Runner(RunnerEvent::Started {
                job_id,
                pid: Some(pid),
            }));

            // Handle kill requests that raced with `Command::spawn()`.
            if killed.load(Ordering::SeqCst) {
                send_process_group_signal(pid, SignalKind::Term);
                schedule_sigkill_escalation(Arc::clone(&active_jobs), job_id, pid);
            }

            let stdout = child.stdout.take();
            let stderr = child.stderr.take();

            let tx_out = tx.clone();
            let h_out = thread::spawn(move || {
                if let Some(out) = stdout {
                    stream_pipe(out, job_id, false, &tx_out);
                }
            });

            let tx_err = tx.clone();
            let h_err = thread::spawn(move || {
                if let Some(err) = stderr {
                    stream_pipe(err, job_id, true, &tx_err);
                }
            });

            let exit_code = match child.wait() {
                Ok(status) => exit_status_code(status),
                Err(_) => 1,
            };

            let _ = h_out.join();
            let _ = h_err.join();

            remove_active_job(&active_jobs, job_id);

            let was_killed = killed.load(Ordering::SeqCst);
            let _ = tx.send(AppEvent::Runner(RunnerEvent::Finished {
                job_id,
                exit_code,
                duration_ms: start.elapsed().as_millis(),
                killed: was_killed,
            }));
        });
    }

    /// Terminate a running job's process group cleanly (`SIGTERM`, escalating to `SIGKILL` after grace period).
    pub fn kill_job(&self, job_id: JobId) -> bool {
        let pid = {
            let Ok(map) = self.active_jobs.lock() else {
                return false;
            };
            let Some(entry) = map.get(&job_id) else {
                return false;
            };
            entry.killed.store(true, Ordering::SeqCst);
            entry.pid
        };

        if let Some(pid) = pid {
            send_process_group_signal(pid, SignalKind::Term);
            schedule_sigkill_escalation(Arc::clone(&self.active_jobs), job_id, pid);
        }
        true
    }

    /// Terminate all active background jobs synchronously (called on clean shutdown or `Drop`)
    /// so no child process trees survive as orphans.
    pub fn kill_all(&self) {
        let snapshot: Vec<(JobId, Option<u32>)> = {
            let Ok(map) = self.active_jobs.lock() else {
                return;
            };
            map.iter()
                .map(|(&id, entry)| {
                    entry.killed.store(true, Ordering::SeqCst);
                    (id, entry.pid)
                })
                .collect()
        };

        if snapshot.is_empty() {
            return;
        }

        for (_, pid) in &snapshot {
            if let Some(pid) = pid {
                send_process_group_signal(*pid, SignalKind::Term);
            }
        }

        let deadline = Instant::now() + SIGKILL_GRACE;
        while Instant::now() < deadline {
            if self.running_count() == 0 {
                return;
            }
            thread::sleep(KILL_POLL_INTERVAL);
        }

        let remaining: Vec<u32> = {
            let Ok(map) = self.active_jobs.lock() else {
                return;
            };
            map.values().filter_map(|e| e.pid).collect()
        };

        for pid in remaining {
            send_process_group_signal(pid, SignalKind::Kill);
        }

        let reap_deadline = Instant::now() + Duration::from_millis(150);
        while Instant::now() < reap_deadline {
            if self.running_count() == 0 {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn running_count(&self) -> usize {
        self.active_jobs.lock().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_running(&self, job_id: JobId) -> bool {
        self.active_jobs
            .lock()
            .map(|m| m.contains_key(&job_id))
            .unwrap_or(false)
    }
}

impl Drop for BackgroundRunner {
    fn drop(&mut self) {
        self.kill_all();
    }
}

fn remove_active_job(active_jobs: &Arc<Mutex<HashMap<JobId, ActiveJob>>>, job_id: JobId) {
    if let Ok(mut map) = active_jobs.lock() {
        map.remove(&job_id);
    }
}

#[derive(Clone, Copy)]
enum SignalKind {
    Term,
    Kill,
}

#[cfg(unix)]
fn send_process_group_signal(pid: u32, kind: SignalKind) {
    let Ok(raw_pid) = libc::pid_t::try_from(pid) else {
        return;
    };
    if raw_pid <= 0 {
        return;
    }
    let sig = match kind {
        SignalKind::Term => libc::SIGTERM,
        SignalKind::Kill => libc::SIGKILL,
    };
    // SAFETY: sending a signal to the dedicated process group `-raw_pid` created via `process_group(0)`.
    unsafe {
        let rc = libc::kill(-raw_pid, sig);
        if rc != 0 {
            // Fallback to direct pid if process group already dissolved.
            let _ = libc::kill(raw_pid, sig);
        }
    }
}

#[cfg(not(unix))]
fn send_process_group_signal(_pid: u32, _kind: SignalKind) {}

fn schedule_sigkill_escalation(
    active_jobs: Arc<Mutex<HashMap<JobId, ActiveJob>>>,
    job_id: JobId,
    pid: u32,
) {
    thread::spawn(move || {
        let deadline = Instant::now() + SIGKILL_GRACE;
        while Instant::now() < deadline {
            let still_same = active_jobs
                .lock()
                .ok()
                .and_then(|m| m.get(&job_id).and_then(|e| e.pid))
                == Some(pid);
            if !still_same {
                return;
            }
            thread::sleep(KILL_POLL_INTERVAL);
        }
        let still_same = active_jobs
            .lock()
            .ok()
            .and_then(|m| m.get(&job_id).and_then(|e| e.pid))
            == Some(pid);
        if still_same {
            send_process_group_signal(pid, SignalKind::Kill);
        }
    });
}

fn exit_status_code(status: std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    if let Some(sig) = status.signal() {
        return 128 + sig;
    }
    1
}

/// Stream arbitrary raw bytes from a child pipe without ever failing on invalid UTF-8,
/// preserving live `\r` carriage-return progress overwrites and `\r\n` CRLF semantics.
fn stream_pipe<R: Read>(mut reader: R, job_id: JobId, is_stderr: bool, tx: &Sender<AppEvent>) {
    let mut buf = [0u8; 4096];
    let mut pending = Vec::<u8>::with_capacity(256);
    let mut next_overwrites = false;
    let mut cr_emitted = false;

    let emit = |pending: &mut Vec<u8>, overwrite_last: bool| {
        let text = String::from_utf8_lossy(pending).into_owned();
        pending.clear();
        let _ = tx.send(AppEvent::Runner(RunnerEvent::Output {
            job_id,
            line: JobOutputLine { text, is_stderr },
            overwrite_last,
        }));
    };

    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                for &b in &buf[..n] {
                    match b {
                        b'\r' => {
                            if !pending.is_empty() {
                                emit(&mut pending, next_overwrites);
                                next_overwrites = true;
                                cr_emitted = true;
                            } else {
                                cr_emitted = false;
                            }
                        }
                        b'\n' => {
                            if cr_emitted {
                                cr_emitted = false;
                                next_overwrites = false;
                            } else {
                                emit(&mut pending, next_overwrites);
                                next_overwrites = false;
                            }
                        }
                        _ => {
                            cr_emitted = false;
                            pending.push(b);
                            if pending.len() >= MAX_LINE_BYTES {
                                emit(&mut pending, next_overwrites);
                                next_overwrites = false;
                            }
                        }
                    }
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }

    if !pending.is_empty() {
        emit(&mut pending, next_overwrites);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::events::{JobRecord, JobStatus};

    fn collect_until_finished(
        rx: &mpsc::Receiver<AppEvent>,
        expected: usize,
        timeout: Duration,
    ) -> HashMap<JobId, (JobRecord, i32, bool)> {
        let mut records: HashMap<JobId, JobRecord> = HashMap::new();
        let mut done: HashMap<JobId, (i32, bool)> = HashMap::new();
        let deadline = Instant::now() + timeout;

        while done.len() < expected && Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(remaining.min(Duration::from_millis(50))) {
                Ok(AppEvent::Runner(ev)) => match ev {
                    RunnerEvent::Started { job_id, pid } => {
                        records.entry(job_id).or_insert_with(|| JobRecord {
                            id: job_id,
                            label: String::new(),
                            task_name: None,
                            command_display: String::new(),
                            pid,
                            started_at: Instant::now(),
                            status: JobStatus::Running,
                            lines: Vec::new(),
                        });
                        if let Some(rec) = records.get_mut(&job_id) {
                            rec.pid = pid;
                        }
                    }
                    RunnerEvent::Output {
                        job_id,
                        line,
                        overwrite_last,
                    } => {
                        records
                            .entry(job_id)
                            .or_insert_with(|| JobRecord {
                                id: job_id,
                                label: String::new(),
                                task_name: None,
                                command_display: String::new(),
                                pid: None,
                                started_at: Instant::now(),
                                status: JobStatus::Running,
                                lines: Vec::new(),
                            })
                            .push_line(line, overwrite_last);
                    }
                    RunnerEvent::Finished {
                        job_id,
                        exit_code,
                        duration_ms,
                        killed,
                    } => {
                        let status = if killed {
                            JobStatus::Killed { duration_ms }
                        } else if exit_code == 0 {
                            JobStatus::Success { duration_ms }
                        } else {
                            JobStatus::Failed {
                                code: exit_code,
                                duration_ms,
                            }
                        };
                        records
                            .entry(job_id)
                            .or_insert_with(|| JobRecord {
                                id: job_id,
                                label: String::new(),
                                task_name: None,
                                command_display: String::new(),
                                pid: None,
                                started_at: Instant::now(),
                                status: JobStatus::Running,
                                lines: Vec::new(),
                            })
                            .status = status;
                        done.insert(job_id, (exit_code, killed));
                    }
                },
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        records
            .into_iter()
            .filter_map(|(id, rec)| {
                let (code, killed) = done.get(&id).copied()?;
                Some((id, (rec, code, killed)))
            })
            .collect()
    }

    #[test]
    fn builds_mise_args_with_dir_and_profile() {
        let dir = PathBuf::from("/tmp/demo");
        let args = BackgroundRunner::build_mise_args(
            &dir,
            &Some("prod".to_string()),
            &["run".to_string(), "check".to_string()],
        );
        assert_eq!(args, vec!["-C", "/tmp/demo", "-E", "prod", "run", "check"]);

        let args_no_profile =
            BackgroundRunner::build_mise_args(&dir, &None, &["ls".to_string()]);
        assert_eq!(args_no_profile, vec!["-C", "/tmp/demo", "ls"]);
    }

    #[test]
    fn isolates_concurrent_jobs_with_unique_ids() {
        let (tx, rx) = mpsc::channel();
        let runner = BackgroundRunner::new(tx);
        let cwd = std::env::temp_dir();

        let id1 = runner.spawn_command(
            cwd.clone(),
            "sh".into(),
            vec!["-c".into(), "echo first-a; sleep 0.05; echo first-b".into()],
            vec![],
        );
        let id2 = runner.spawn_command(
            cwd,
            "sh".into(),
            vec!["-c".into(), "echo second-a; echo second-b".into()],
            vec![],
        );

        assert_ne!(id1, id2);
        let results = collect_until_finished(&rx, 2, Duration::from_secs(5));
        assert_eq!(results.len(), 2);

        let lines1: Vec<&str> = results[&id1]
            .0
            .lines
            .iter()
            .map(|l| l.text.as_str())
            .collect();
        let lines2: Vec<&str> = results[&id2]
            .0
            .lines
            .iter()
            .map(|l| l.text.as_str())
            .collect();

        assert_eq!(lines1, vec!["first-a", "first-b"]);
        assert_eq!(lines2, vec!["second-a", "second-b"]);
        assert_eq!(results[&id1].1, 0);
        assert_eq!(results[&id2].1, 0);
    }

    #[test]
    fn streams_non_utf8_bytes_without_truncating() {
        let (tx, rx) = mpsc::channel();
        let runner = BackgroundRunner::new(tx);
        let cwd = std::env::temp_dir();

        let id = runner.spawn_command(
            cwd,
            "sh".into(),
            vec![
                "-c".into(),
                "printf 'alpha \\377\\376 beta\\ngamma\\n'".into(),
            ],
            vec![],
        );

        let results = collect_until_finished(&rx, 1, Duration::from_secs(5));
        let (rec, code, killed) = &results[&id];
        assert_eq!(*code, 0);
        assert!(!killed);
        assert_eq!(rec.lines.len(), 2);
        assert!(rec.lines[0].text.starts_with("alpha "));
        assert!(rec.lines[0].text.contains('\u{FFFD}'));
        assert!(rec.lines[0].text.ends_with(" beta"));
        assert_eq!(rec.lines[1].text, "gamma");
    }

    #[test]
    fn handles_carriage_return_progress_overwrites_and_crlf() {
        let (tx, rx) = mpsc::channel();
        let runner = BackgroundRunner::new(tx);
        let cwd = std::env::temp_dir();

        let id = runner.spawn_command(
            cwd,
            "sh".into(),
            vec![
                "-c".into(),
                "printf '10%%\\r50%%\\r100%%\\nline-crlf\\r\\nfinal\\n'".into(),
            ],
            vec![],
        );

        let results = collect_until_finished(&rx, 1, Duration::from_secs(5));
        let lines: Vec<&str> = results[&id]
            .0
            .lines
            .iter()
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(lines, vec!["100%", "line-crlf", "final"]);
    }

    #[cfg(unix)]
    #[test]
    fn kill_job_reaps_entire_process_group_without_orphans() {
        let (tx, rx) = mpsc::channel();
        let runner = BackgroundRunner::new(tx);
        let cwd = std::env::temp_dir();

        // Spawn a shell that forks a background child `sleep 30`, prints the child's PID, and waits.
        let id = runner.spawn_command(
            cwd,
            "sh".into(),
            vec!["-c".into(), "sleep 30 & echo $!; wait".into()],
            vec![],
        );

        let mut child_pid: Option<libc::pid_t> = None;
        let deadline = Instant::now() + Duration::from_secs(5);
        while child_pid.is_none() && Instant::now() < deadline {
            if let Ok(AppEvent::Runner(RunnerEvent::Output {
                job_id,
                line,
                ..
            })) = rx.recv_timeout(Duration::from_millis(50))
                && job_id == id
                && let Ok(parsed) = line.text.trim().parse::<libc::pid_t>()
            {
                child_pid = Some(parsed);
            }
        }

        let child_pid = child_pid.expect("should receive spawned grandchild PID");
        assert!(runner.kill_job(id));

        let mut finished_killed = false;
        let finish_deadline = Instant::now() + Duration::from_secs(5);
        while !finished_killed && Instant::now() < finish_deadline {
            if let Ok(AppEvent::Runner(RunnerEvent::Finished { job_id, killed, .. })) =
                rx.recv_timeout(Duration::from_millis(50))
                && job_id == id
            {
                finished_killed = killed;
            }
        }
        assert!(finished_killed, "job should report killed=true");
        assert_eq!(runner.running_count(), 0);

        // Give OS a brief moment to clean up the grandchild process entry, then assert ESRCH.
        let mut dead = false;
        for _ in 0..20 {
            let rc = unsafe { libc::kill(child_pid, 0) };
            if rc != 0 {
                dead = true;
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        assert!(dead, "grandchild process {child_pid} must not survive as orphan");
    }

    #[cfg(unix)]
    #[test]
    fn drop_kills_all_running_jobs() {
        let (tx, rx) = mpsc::channel();
        let cwd = std::env::temp_dir();
        let spawned_pid: libc::pid_t;

        {
            let runner = BackgroundRunner::new(tx);
            let id = runner.spawn_command(
                cwd,
                "sh".into(),
                vec!["-c".into(), "echo $$; exec sleep 30".into()],
                vec![],
            );

            let mut pid = None;
            let deadline = Instant::now() + Duration::from_secs(5);
            while pid.is_none() && Instant::now() < deadline {
                if let Ok(AppEvent::Runner(RunnerEvent::Output {
                    job_id,
                    line,
                    ..
                })) = rx.recv_timeout(Duration::from_millis(50))
                    && job_id == id
                    && let Ok(parsed) = line.text.trim().parse::<libc::pid_t>()
                {
                    pid = Some(parsed);
                }
            }
            spawned_pid = pid.expect("should capture pid");
            // `runner` drops here and must terminate the running process group.
        }

        let mut dead = false;
        for _ in 0..20 {
            let rc = unsafe { libc::kill(spawned_pid, 0) };
            if rc != 0 {
                dead = true;
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        assert!(dead, "process {spawned_pid} must be terminated on Drop");
    }

    #[cfg(unix)]
    #[test]
    fn kill_job_escalates_to_sigkill_when_sigterm_ignored() {
        let (tx, rx) = mpsc::channel();
        let runner = BackgroundRunner::new(tx);
        let cwd = std::env::temp_dir();

        let id = runner.spawn_command(
            cwd,
            "sh".into(),
            vec![
                "-c".into(),
                "trap '' TERM; echo ready; while true; do sleep 1; done".into(),
            ],
            vec![],
        );

        let mut ready = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready && Instant::now() < deadline {
            if let Ok(AppEvent::Runner(RunnerEvent::Output {
                job_id,
                line,
                ..
            })) = rx.recv_timeout(Duration::from_millis(50))
                && job_id == id
                && line.text.trim() == "ready"
            {
                ready = true;
            }
        }
        assert!(ready, "child should become ready with SIGTERM trapped");

        assert!(runner.kill_job(id));

        let mut finished = None;
        let finish_deadline = Instant::now() + Duration::from_secs(5);
        while finished.is_none() && Instant::now() < finish_deadline {
            if let Ok(AppEvent::Runner(RunnerEvent::Finished {
                job_id,
                exit_code,
                killed,
                ..
            })) = rx.recv_timeout(Duration::from_millis(50))
                && job_id == id
            {
                finished = Some((exit_code, killed));
            }
        }

        let (code, killed) = finished.expect("job ignoring SIGTERM must be reaped via SIGKILL");
        assert!(killed);
        assert_eq!(code, 128 + libc::SIGKILL);
        assert_eq!(runner.running_count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn background_job_detaches_from_controlling_tty_and_fails_fast_on_dev_tty() {
        let (tx, rx) = mpsc::channel();
        let runner = BackgroundRunner::new(tx);
        let id = runner.spawn_command(
            std::env::temp_dir(),
            "sh".to_string(),
            vec!["-c".to_string(), "cat </dev/tty".to_string()],
            Vec::new(),
        );

        let deadline = Instant::now() + Duration::from_secs(3);
        let mut finished_code = None;
        while finished_code.is_none() && Instant::now() < deadline {
            if let Ok(AppEvent::Runner(RunnerEvent::Finished {
                job_id, exit_code, ..
            })) = rx.recv_timeout(Duration::from_millis(50))
                && job_id == id
            {
                finished_code = Some(exit_code);
            }
        }

        let code = finished_code.expect("job touching /dev/tty must fail fast instead of hanging");
        assert_ne!(code, 0);
        assert_eq!(runner.running_count(), 0);
    }
}
