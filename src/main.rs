mod app;
mod cli;
mod mise;
mod model;
mod runner;
mod ui;

use std::io;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::app::{App, ForegroundRequest};
use crate::cli::Cli;
use crate::runner::run_foreground;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let workdir = match cli.dir {
        Some(d) => d,
        None => std::env::current_dir()?,
    };

    // Restore terminal on panic
    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        orig_hook(info);
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;

    let res = run_app(&mut terminal, App::new(workdir, cli.tab));

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    res
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mut app: App,
) -> Result<()> {
    while !app.should_quit {
        app.poll_runner();
        terminal.draw(|frame| ui::render(frame, &app))?;

        if let Some(req) = app.pending_fg.take() {
            handle_foreground(terminal, &mut app, req);
            continue;
        }

        if event::poll(Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.handle_key(key);
        }
    }
    Ok(())
}

fn handle_foreground(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    req: ForegroundRequest,
) {
    let workdir = app.client.workdir().to_path_buf();
    match req {
        ForegroundRequest::RunTask(name) => {
            let args = vec!["run".to_string(), name.clone()];
            match run_foreground(terminal, &workdir, "mise", &args, true) {
                Ok((code, ms)) => app.record_foreground_result(&name, code, ms),
                Err(e) => app.status_msg = format!("Run error: {e}"),
            }
        }
        ForegroundRequest::WatchTask(name) => {
            let args = vec!["watch".to_string(), "-t".to_string(), name.clone()];
            match run_foreground(terminal, &workdir, "mise", &args, true) {
                Ok((code, ms)) => app.record_foreground_result(&name, code, ms),
                Err(e) => app.status_msg = format!("Watch error: {e}"),
            }
        }
        ForegroundRequest::EditFile(path) => {
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nvim".to_string());
            let args = vec![path.display().to_string()];
            match run_foreground(terminal, &workdir, &editor, &args, false) {
                Ok(_) => app.reload_all(false),
                Err(e) => app.status_msg = format!("Editor `{editor}` failed: {e}"),
            }
        }
        ForegroundRequest::RunCommand {
            program,
            args,
            wait_for_enter,
            reload_after,
        } => {
            match run_foreground(terminal, &workdir, &program, &args, wait_for_enter) {
                Ok((code, ms)) => {
                    if reload_after {
                        app.reload_all(false);
                    }
                    app.status_msg = format!(
                        "`{} {}` exited {} ({:.2}s)",
                        program,
                        args.join(" "),
                        code,
                        ms as f64 / 1000.0
                    );
                }
                Err(e) => app.status_msg = format!("Command error: {e}"),
            }
        }
    }
}
