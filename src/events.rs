use std::collections::BTreeSet;
use std::time::{Instant, SystemTime};

use crossterm::event::{KeyEvent, MouseEvent};

use crate::action::Domain;
use crate::mise::schema::{
    AuditReport, ConfigExplainItem, ConfigFileEntry, DaemonEntry, DepEntry, DoctorReport,
    MiseEnvExtendedMap, MiseTask, MiseToolsMap, MiseVarsMap, OutdatedMap, RegistryEntry,
    SettingEntry, TaskHistoryEntry, ToolDetail, TrustEntry,
};
use crate::model::template::WorkspaceTemplate;

/// Monotonically increasing identifier for each background job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(pub u64);

/// Lifecycle status of a background or foreground job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobStatus {
    Running,
    Success { duration_ms: u128 },
    Failed { code: i32, duration_ms: u128 },
    Killed { duration_ms: u128 },
}

impl JobStatus {
    pub fn is_running(&self) -> bool {
        matches!(self, JobStatus::Running)
    }
}

/// Single streamed output line from a job (preserving whether it came from stderr and `\r` overwrite).
#[derive(Debug, Clone)]
pub struct JobOutputLine {
    pub text: String,
    pub is_stderr: bool,
}

/// Full execution record for a job (tasks or CLI mutations).
#[derive(Debug, Clone)]
pub struct JobRecord {
    pub id: JobId,
    pub label: String,
    pub task_name: Option<String>,
    pub command_display: String,
    pub pid: Option<u32>,
    pub started_at: Instant,
    pub status: JobStatus,
    pub lines: Vec<JobOutputLine>,
}

impl JobRecord {
    pub const MAX_LINES: usize = 4000;

    pub fn push_line(&mut self, line: JobOutputLine, overwrite_last: bool) {
        if overwrite_last && let Some(last) = self.lines.last_mut() {
            *last = line;
            return;
        }
        if self.lines.len() >= Self::MAX_LINES {
            let drain = self.lines.len() - Self::MAX_LINES + 1;
            self.lines.drain(0..drain);
        }
        self.lines.push(line);
    }
}

/// Distinguishes user-triggered commands from internal read/refresh queries in the Command Log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSource {
    User,
    Internal,
}

/// Entry stored in the lazygit-style Command Log ring buffer.
#[derive(Debug, Clone)]
pub struct CommandLogEntry {
    pub id: u64,
    pub timestamp: SystemTime,
    pub source: CommandSource,
    pub command: String,
    pub job_id: Option<JobId>,
    pub status: JobStatus,
    pub summary: Option<String>,
}

/// Events emitted by `BackgroundRunner`.
#[derive(Debug, Clone)]
pub enum RunnerEvent {
    Started {
        job_id: JobId,
        pid: Option<u32>,
    },
    Output {
        job_id: JobId,
        line: JobOutputLine,
        overwrite_last: bool,
    },
    Finished {
        job_id: JobId,
        exit_code: i32,
        duration_ms: u128,
        killed: bool,
    },
}

/// Domain payloads delivered asynchronously by `MiseLoader`.
#[derive(Debug, Clone)]
pub enum MisePayload {
    Tasks(Vec<MiseTask>),
    TaskInfo {
        task_name: String,
        task: Box<MiseTask>,
    },
    TaskDeps {
        task_name: String,
        tree_lines: Vec<String>,
    },
    TaskHistory {
        task_name: String,
        entries: Vec<TaskHistoryEntry>,
    },
    Tools(MiseToolsMap),
    ToolDetail {
        tool_name: String,
        detail: Box<ToolDetail>,
    },
    ToolVersions {
        tool_name: String,
        versions: Vec<String>,
    },
    Registry(Vec<RegistryEntry>),
    Outdated(OutdatedMap),
    Audit(AuditReport),
    LockDryRun(String),
    Env {
        extended: MiseEnvExtendedMap,
        redacted_keys: BTreeSet<String>,
        paths: Vec<String>,
        vars: MiseVarsMap,
    },
    Config {
        files: Vec<ConfigFileEntry>,
        trust: Vec<TrustEntry>,
    },
    ConfigExplain(Vec<ConfigExplainItem>),
    Doctor(Box<DoctorReport>),
    Settings(Vec<SettingEntry>),
    Daemons(Vec<DaemonEntry>),
    Deps(Vec<DepEntry>),
    Templates(Vec<WorkspaceTemplate>),
    Error {
        domain: Domain,
        message: String,
    },
}

/// Asynchronous data response tagged with `generation` for stale-response cancellation.
#[derive(Debug, Clone)]
pub struct MiseDataEvent {
    pub generation: u64,
    pub domain: Domain,
    pub payload: MisePayload,
    pub log_entry: Option<CommandLogEntry>,
}

/// Top-level unified event bus consumed by the main loop and `App`.
#[derive(Debug, Clone)]
pub enum AppEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
    Tick,
    MiseData(MiseDataEvent),
    Runner(RunnerEvent),
}
