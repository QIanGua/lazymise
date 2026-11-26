use std::io::{self, IsTerminal};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use lazymise::action::ToastLevel;
use lazymise::app::{App, ForegroundRequest};
use lazymise::cli::Cli;
use lazymise::events::AppEvent;
use lazymise::runner::{BackgroundRunner, run_foreground, split_shell_words};
use lazymise::ui;

const TICK_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Default)]
struct InputControl {
    paused: bool,
    ack_paused: bool,
    shutdown: bool,
}

struct InputPauseHandle {
    sync: Arc<(Mutex<InputControl>, Condvar)>,
}

impl InputPauseHandle {
    fn pause(&self) {
        let (lock, cvar) = &*self.sync;
        let mut ctrl = lock.lock().expect("input control mutex poisoned");
        ctrl.paused = true;
        while !ctrl.ack_paused && !ctrl.shutdown {
            ctrl = cvar.wait(ctrl).expect("input control condvar poisoned");
        }
    }

    fn resume(&self) {
        let (lock, cvar) = &*self.sync;
        let mut ctrl = lock.lock().expect("input control mutex poisoned");
        ctrl.paused = false;
        cvar.notify_all();
    }

    fn shutdown(&self) {
        let (lock, cvar) = &*self.sync;
        let mut ctrl = lock.lock().expect("input control mutex poisoned");
        ctrl.shutdown = true;
        ctrl.paused = false;
        cvar.notify_all();
    }
}

fn spawn_input_thread(tx: mpsc::Sender<AppEvent>) -> InputPauseHandle {
    let sync = Arc::new((Mutex::new(InputControl::default()), Condvar::new()));
    let worker_sync = Arc::clone(&sync);

    thread::spawn(move || {
        let (lock, cvar) = &*worker_sync;
        loop {
            {
                let mut ctrl = lock.lock().expect("input control mutex poisoned");
                if ctrl.shutdown {
                    break;
                }
                if ctrl.paused {
                    ctrl.ack_paused = true;
                    cvar.notify_all();
                    while ctrl.paused && !ctrl.shutdown {
                        ctrl = cvar.wait(ctrl).expect("input control condvar poisoned");
                    }
                    ctrl.ack_paused = false;
                    if ctrl.shutdown {
                        break;
                    }
                }
            }

            if let Ok(true) = event::poll(Duration::from_millis(25))
                && let Ok(ev) = event::read()
            {
                let mapped = match ev {
                    Event::Key(k) if k.kind == KeyEventKind::Press => Some(AppEvent::Key(k)),
                    Event::Mouse(m) => Some(AppEvent::Mouse(m)),
                    Event::Resize(w, h) => Some(AppEvent::Resize(w, h)),
                    _ => None,
                };
                if let Some(app_ev) = mapped
                    && tx.send(app_ev).is_err()
                {
                    break;
                }
            }
        }
    });

    InputPauseHandle { sync }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if std::env::var_os("LAZYMISE_NESTED")
        .is_some_and(|v| !v.is_empty() && v != "0")
    {
        anyhow::bail!(
            "lazymise is already running in this session; nested lazymise sessions are disabled."
        );
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        anyhow::bail!(
            "lazymise requires an interactive terminal (TTY). Run it directly in a terminal (or press `T` in lazymise for foreground TTY)."
        );
    }
    let workdir = match cli.dir {
        Some(d) => d,
        None => std::env::current_dir()?,
    };

    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
        orig_hook(info);
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;

    let (tx, rx) = mpsc::channel::<AppEvent>();
    let input_handle = spawn_input_thread(tx.clone());

    let app = App::new(workdir, cli.env, cli.tab.into(), tx);
    let res = run_app(&mut terminal, app, rx, &input_handle);

    input_handle.shutdown();

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    res
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mut app: App,
    rx: mpsc::Receiver<AppEvent>,
    input_handle: &InputPauseHandle,
) -> Result<()> {
    let mut last_tick = Instant::now();

    while !app.state.should_quit {
        if app.state.dirty {
            terminal.draw(|frame| ui::render(frame, &mut app.state, &mut app.layout_map))?;
            app.state.dirty = false;
        }

        if let Some(req) = app.state.pending_fg.take() {
            handle_foreground(terminal, &mut app, req, input_handle);
            // Drain any stale queued input events that arrived right before pause
            while let Ok(ev) = rx.try_recv() {
                if !matches!(ev, AppEvent::Key(_) | AppEvent::Mouse(_)) {
                    app.handle_event(ev);
                }
            }
            app.state.dirty = true;
            last_tick = Instant::now();
            continue;
        }

        if app.state.has_active_animations() {
            let wait = TICK_INTERVAL.saturating_sub(last_tick.elapsed());
            match rx.recv_timeout(wait) {
                Ok(ev) => {
                    app.handle_event(ev);
                    while let Ok(more) = rx.try_recv() {
                        app.handle_event(more);
                    }
                    if last_tick.elapsed() >= TICK_INTERVAL {
                        app.handle_event(AppEvent::Tick);
                        last_tick = Instant::now();
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    app.handle_event(AppEvent::Tick);
                    last_tick = Instant::now();
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            // Truly idle: block with 0% CPU until an input, resize, or async loader event arrives
            match rx.recv() {
                Ok(ev) => {
                    app.handle_event(ev);
                    while let Ok(more) = rx.try_recv() {
                        app.handle_event(more);
                    }
                    last_tick = Instant::now();
                }
                Err(_) => break,
            }
        }
    }

    app.runner.kill_all();
    Ok(())
}

fn handle_foreground(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    req: ForegroundRequest,
    input_handle: &InputPauseHandle,
) {
    let workdir = app.state.workdir.clone();
    let profile = app.state.profile.clone();

    input_handle.pause();

    let result = match &req {
        ForegroundRequest::RunTask { name, args, force } => {
            let mut mise_args = vec!["run".to_string()];
            if *force {
                mise_args.push("--force".to_string());
            }
            mise_args.push(name.clone());
            if !args.is_empty() {
                mise_args.push("--".to_string());
                mise_args.extend(args.iter().cloned());
            }
            let full = BackgroundRunner::build_mise_args(&workdir, &profile, &mise_args);
            run_foreground(terminal, &workdir, "mise", &full, true, true)
                .map(|(code, ms)| (code, ms, false))
        }
        ForegroundRequest::WatchTask { name } => {
            let mise_args = vec!["watch".to_string(), "-t".to_string(), name.clone()];
            let full = BackgroundRunner::build_mise_args(&workdir, &profile, &mise_args);
            run_foreground(terminal, &workdir, "mise", &full, true, true)
                .map(|(code, ms)| (code, ms, false))
        }
        ForegroundRequest::EditFile { path } => {
            let editor_raw = std::env::var("EDITOR")
                .or_else(|_| std::env::var("VISUAL"))
                .unwrap_or_else(|_| "nvim".to_string());
            match split_shell_words(&editor_raw) {
                Ok(mut words) if !words.is_empty() => {
                    let program = words.remove(0);
                    words.push(path.display().to_string());
                    run_foreground(terminal, &workdir, &program, &words, false, true)
                        .map(|(code, ms)| (code, ms, true))
                }
                Ok(_) => Err(anyhow::anyhow!("$EDITOR is empty")),
                Err(err) => Err(err),
            }
        }
        ForegroundRequest::RunMiseCommand {
            args,
            wait_for_enter,
            reload_after,
            ..
        } => {
            let full = BackgroundRunner::build_mise_args(&workdir, &profile, args);
            run_foreground(terminal, &workdir, "mise", &full, *wait_for_enter, true)
                .map(|(code, ms)| (code, ms, *reload_after))
        }
    };

    input_handle.resume();

    match result {
        Ok((code, ms, reload_after)) => {
            app.record_foreground_result(&req, code, ms, reload_after);
        }
        Err(err) => {
            app.state
                .push_toast(ToastLevel::Error, format!("Foreground error: {err}"));
        }
    }
}
