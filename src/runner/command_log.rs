use std::collections::VecDeque;
use std::time::SystemTime;

use crate::events::{CommandLogEntry, CommandSource, JobId, JobStatus};

/// Bounded ring buffer of executed `mise` commands (lazygit-style Command Log).
/// Entries are ordered newest-first (`front` = most recent).
#[derive(Debug, Clone)]
pub struct CommandLog {
    entries: VecDeque<CommandLogEntry>,
    max_entries: usize,
    next_id: u64,
}

impl Default for CommandLog {
    fn default() -> Self {
        Self::new(200)
    }
}

impl CommandLog {
    pub fn new(max_entries: usize) -> Self {
        let cap = max_entries.max(1);
        Self {
            entries: VecDeque::with_capacity(cap.min(256)),
            max_entries: cap,
            next_id: 1,
        }
    }

    pub fn push(&mut self, mut entry: CommandLogEntry) -> u64 {
        if entry.id == 0 {
            entry.id = self.next_id;
            self.next_id += 1;
        } else {
            self.next_id = self.next_id.max(entry.id + 1);
        }
        let id = entry.id;
        if self.entries.len() >= self.max_entries {
            self.entries.pop_back();
        }
        self.entries.push_front(entry);
        id
    }

    /// Convenience helper to create and insert a `CommandLogEntry` timestamped with `SystemTime::now()`.
    pub fn record(
        &mut self,
        source: CommandSource,
        command: impl Into<String>,
        job_id: Option<JobId>,
        status: JobStatus,
        summary: Option<String>,
    ) -> u64 {
        self.push(CommandLogEntry {
            id: 0,
            timestamp: SystemTime::now(),
            source,
            command: command.into(),
            job_id,
            status,
            summary,
        })
    }

    pub fn update_job_status(&mut self, job_id: JobId, status: JobStatus) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.job_id == Some(job_id)) {
            entry.status = status;
        }
    }

    pub fn update_by_id(&mut self, id: u64, status: JobStatus, summary: Option<String>) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.status = status;
            if summary.is_some() {
                entry.summary = summary;
            }
            true
        } else {
            false
        }
    }

    pub fn get(&self, id: u64) -> Option<&CommandLogEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn find_by_job(&self, job_id: JobId) -> Option<&CommandLogEntry> {
        self.entries.iter().find(|e| e.job_id == Some(job_id))
    }

    pub fn entries(&self, show_internal: bool) -> Vec<&CommandLogEntry> {
        self.entries
            .iter()
            .filter(|e| show_internal || e.source == CommandSource::User)
            .collect()
    }

    /// Alias for `entries(show_internal)` returning newest-first filtered entries.
    pub fn filtered(&self, show_internal: bool) -> Vec<&CommandLogEntry> {
        self.entries(show_internal)
    }

    pub fn all_entries(&self) -> &VecDeque<CommandLogEntry> {
        &self.entries
    }

    pub fn clear_finished(&mut self) {
        self.entries.retain(|e| e.status.is_running());
    }

    /// Alias for `clear_finished()`.
    pub fn clear_completed(&mut self) {
        self.clear_finished();
    }

    pub fn max_entries(&self) -> usize {
        self.max_entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn enforces_ring_buffer_capacity_newest_first() {
        let mut log = CommandLog::new(3);
        for i in 1..=5 {
            log.record(
                CommandSource::User,
                format!("mise run task-{i}"),
                None,
                JobStatus::Success { duration_ms: 10 },
                None,
            );
        }

        assert_eq!(log.len(), 3);
        let cmds: Vec<&str> = log.filtered(true).iter().map(|e| e.command.as_str()).collect();
        assert_eq!(
            cmds,
            vec!["mise run task-5", "mise run task-4", "mise run task-3"]
        );
    }

    #[test]
    fn filters_internal_commands_and_updates_job_status() {
        let mut log = CommandLog::default();
        log.record(
            CommandSource::Internal,
            "mise tasks ls --json",
            None,
            JobStatus::Success { duration_ms: 20 },
            None,
        );
        let user_id = log.record(
            CommandSource::User,
            "mise run check",
            Some(JobId(7)),
            JobStatus::Running,
            None,
        );

        assert_eq!(log.filtered(false).len(), 1);
        assert_eq!(log.filtered(true).len(), 2);
        assert_eq!(log.filtered(false)[0].id, user_id);

        log.update_job_status(JobId(7), JobStatus::Success { duration_ms: 420 });
        assert_eq!(
            log.find_by_job(JobId(7)).map(|e| &e.status),
            Some(&JobStatus::Success { duration_ms: 420 })
        );

        log.record(
            CommandSource::User,
            "mise watch -t test",
            Some(JobId(8)),
            JobStatus::Running,
            None,
        );
        log.clear_completed();
        assert_eq!(log.len(), 1);
        assert_eq!(log.all_entries()[0].job_id, Some(JobId(8)));
    }
}
