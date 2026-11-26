use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use anyhow::{Context, Result};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

#[derive(Debug, Clone)]
pub enum RunStatus {
    Running,
    Success { duration_ms: u128 },
    Failed { code: i32, duration_ms: u128 },
}

#[derive(Debug, Clone)]
pub struct TaskRunRecord {
    pub status: RunStatus,
    pub lines: Vec<String>,
}

impl TaskRunRecord {
    pub fn push_line(&mut self, line: String) {
        const MAX_LINES: usize = 2000;
        if self.lines.len() >= MAX_LINES {
            let drain = self.lines.len() - MAX_LINES + 1;
            self.lines.drain(0..drain);
        }
        self.lines.push(line);
    }
}

#[derive(Debug)]
pub enum RunnerEvent {
    Line {
        task: String,
        line: String,
    },
    Finished {
        task: String,
        exit_code: i32,
        duration_ms: u128,
    },
}

pub struct BackgroundRunner {
    tx: Sender<RunnerEvent>,
    rx: Receiver<RunnerEvent>,
}

impl Default for BackgroundRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl BackgroundRunner {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx }
    }

    pub fn spawn_mise_task(&self, workdir: PathBuf, task_name: String) {
        let tx = self.tx.clone();
        thread::spawn(move || {
            let start = Instant::now();
            let child = Command::new("mise")
                .args(["run", &task_name])
                .current_dir(&workdir)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn();

            let mut child = match child {
                Ok(c) => c,
                Err(err) => {
                    let _ = tx.send(RunnerEvent::Line {
                        task: task_name.clone(),
                        line: format!("error: failed to spawn `mise run {task_name}`: {err}"),
                    });
                    let _ = tx.send(RunnerEvent::Finished {
                        task: task_name,
                        exit_code: 127,
                        duration_ms: start.elapsed().as_millis(),
                    });
                    return;
                }
            };

            let stdout = child.stdout.take();
            let stderr = child.stderr.take();

            let tx_out = tx.clone();
            let task_out = task_name.clone();
            let h_out = thread::spawn(move || {
                if let Some(out) = stdout {
                    let reader = BufReader::new(out);
                    for line in reader.lines().map_while(Result::ok) {
                        let _ = tx_out.send(RunnerEvent::Line {
                            task: task_out.clone(),
                            line,
                        });
                    }
                }
            });

            let tx_err = tx.clone();
            let task_err = task_name.clone();
            let h_err = thread::spawn(move || {
                if let Some(err) = stderr {
                    let reader = BufReader::new(err);
                    for line in reader.lines().map_while(Result::ok) {
                        let _ = tx_err.send(RunnerEvent::Line {
                            task: task_err.clone(),
                            line: format!("[stderr] {line}"),
                        });
                    }
                }
            });

            let exit_code = child
                .wait()
                .ok()
                .and_then(|s| s.code())
                .unwrap_or(1);
            let _ = h_out.join();
            let _ = h_err.join();

            let _ = tx.send(RunnerEvent::Finished {
                task: task_name,
                exit_code,
                duration_ms: start.elapsed().as_millis(),
            });
        });
    }

    pub fn drain_events(&self) -> Vec<RunnerEvent> {
        let mut events = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            events.push(ev);
        }
        events
    }
}

/// Suspend TUI, execute a foreground command with full TTY access, and restore TUI.
pub fn run_foreground(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    workdir: &Path,
    program: &str,
    args: &[String],
    wait_for_enter: bool,
) -> Result<(i32, u128)> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    println!("$ {} {}", program, args.join(" "));
    let _ = io::stdout().flush();

    let start = Instant::now();
    let status = Command::new(program)
        .args(args)
        .current_dir(workdir)
        .status()
        .with_context(|| format!("failed to run `{program}`"))?;
    let duration_ms = start.elapsed().as_millis();
    let code = status.code().unwrap_or(1);

    if wait_for_enter {
        println!(
            "\n[exit {} in {:.2}s] Press Enter to return to lazymise...",
            code,
            duration_ms as f64 / 1000.0
        );
        let _ = io::stdout().flush();
        let mut buf = String::new();
        let _ = io::stdin().read_line(&mut buf);
    }

    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    terminal.hide_cursor()?;
    terminal.clear()?;

    Ok((code, duration_ms))
}

pub fn copy_to_clipboard(text: &str) -> Result<()> {
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else {
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ]
    };

    for (prog, args) in candidates {
        if let Ok(mut child) = Command::new(prog)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(text.as_bytes())?;
            }
            let status = child.wait()?;
            if status.success() {
                return Ok(());
            }
        }
    }

    anyhow::bail!("no clipboard utility found (pbcopy / wl-copy / xclip)")
}
