use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};

use crate::action::{FocusArea, Panel};
use crate::app::state::AppState;
use crate::events::{CommandLogEntry, CommandSource, JobStatus};
use crate::ui::layout::LayoutMap;
use crate::ui::theme::{self, glyphs};
use crate::ui::widgets::{
    empty_state_lines, format_duration_ms, format_hms, kv_line, panel_block,
    render_job_output_line, render_scrollbar, section_divider, truncate_display,
};

/// Collect command log entries visible under current `show_internal_commands` & `filters.jobs`.
pub fn visible_entries(state: &AppState) -> Vec<&CommandLogEntry> {
    state.filtered_command_log()
}

/// Render `[5] COMMAND LOG & JOBS` as a full sidebar (used on narrow screens or Full zoom).
pub fn render_sidebar(
    frame: &mut Frame,
    state: &mut AppState,
    area: Rect,
    layout: &mut LayoutMap,
) {
    let focused = state.active_panel == Panel::Jobs
        && (state.focus == FocusArea::Sidebar || state.focus == FocusArea::Log);
    let inner = render_jobs_list(frame, state, area, focused, false);
    layout.register_sidebar_panel(Panel::Jobs, area, inner);
}

/// Render the bottom `[5] COMMAND LOG & JOBS` drawer (`@` cycles hidden / 4-row / 10-row).
pub fn render_log_drawer(
    frame: &mut Frame,
    state: &mut AppState,
    area: Rect,
    focused: bool,
    layout: &mut LayoutMap,
) {
    let inner = render_jobs_list(frame, state, area, focused, true);
    layout.log_inner = inner;
}

/// Render the right main pane when `[5] JOBS & LOG` is active: full ANSI output or command metadata.
pub fn render_main(
    frame: &mut Frame,
    state: &mut AppState,
    area: Rect,
    layout: &mut LayoutMap,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let focused = state.active_panel == Panel::Jobs && state.focus == FocusArea::Main;
    let right_hint = Some(Line::from(vec![
        Span::styled(
            if state.auto_follow {
                " FOLLOW:ON "
            } else {
                " FOLLOW:OFF "
            },
            theme::muted(),
        ),
    ]));
    let block = panel_block(
        Line::from(vec![Span::styled(" JOB OUTPUT & COMMAND DETAIL ", theme::panel_title(focused))]),
        right_hint,
        focused,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    layout.main_inner = inner;

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    state.detail_scroll.viewport_height = inner.height;

    let lines = build_job_detail_lines(state, inner.width as usize);
    let total_lines = lines.len().min(u16::MAX as usize) as u16;
    state.detail_scroll.content_len = total_lines;
    if state.auto_follow {
        state.detail_scroll.scroll_bottom();
    } else {
        state.detail_scroll.clamp();
    }

    let scroll_y = state.detail_scroll.offset;
    let para = Paragraph::new(lines)
        .scroll((scroll_y, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(para, inner);

    render_scrollbar(
        frame,
        area,
        total_lines as usize,
        inner.height as usize,
        scroll_y as usize,
        focused,
    );
}

fn render_jobs_list(
    frame: &mut Frame,
    state: &mut AppState,
    area: Rect,
    focused: bool,
    is_drawer: bool,
) -> Rect {
    if area.height == 0 || area.width == 0 {
        return Rect::default();
    }

    let running = state.running_jobs_count();
    let entries = visible_entries(state);
    let total_entries = entries.len();

    let mut title_spans = vec![Span::styled(
        if is_drawer {
            " [5] COMMAND LOG & JOBS "
        } else {
            " [5] JOBS & LOG "
        },
        theme::panel_title(focused),
    )];
    if running > 0 {
        title_spans.push(Span::styled(
            format!("({} {running} running) ", state.spinner_char()),
            theme::accent(),
        ));
    } else {
        title_spans.push(Span::styled(format!("({total_entries}) "), theme::muted()));
    }

    let mut right_spans = Vec::new();
    if state.show_internal_commands {
        right_spans.push(Span::styled(" +INT", theme::purple()));
    }
    if !state.filters.jobs.is_empty() {
        right_spans.push(Span::styled(
            format!(" /{}", truncate_display(&state.filters.jobs, 10)),
            theme::highlight_match(),
        ));
    }
    if area.width >= 60 {
        right_spans.push(Span::styled(
            if is_drawer {
                " @:toggle  K:kill  Enter:inspect "
            } else {
                " K:kill  r:rerun  y:copy "
            },
            theme::muted(),
        ));
    } else {
        right_spans.push(Span::raw(" "));
    }

    let block = panel_block(Line::from(title_spans), Some(Line::from(right_spans)), focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return inner;
    }

    if entries.is_empty() {
        let msg = if !state.filters.jobs.is_empty() {
            format!("No commands match '/{}'.", state.filters.jobs)
        } else if !state.show_internal_commands && !state.command_log.is_empty() {
            format!(
                "No user commands yet ({} internal queries hidden — press `i` to toggle).",
                state.command_log.len()
            )
        } else {
            "Command log ready — executed mise commands and background jobs appear here.".to_string()
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(msg, theme::muted())))
                .wrap(Wrap { trim: true }),
            inner,
        );
        return inner;
    }

    let inner_w = inner.width as usize;
    let spinner = state.spinner_char();
    let items: Vec<ListItem<'static>> = entries
        .iter()
        .map(|entry| {
            let ts = format_hms(entry.timestamp);
            let (st_text, st_style, dur_str) = format_entry_status(entry, state, spinner);
            let (src_badge, src_style) = match entry.source {
                CommandSource::User => ("USR", theme::accent()),
                CommandSource::Internal => ("INT", theme::subtle()),
            };

            let fixed_w = ts.len() + 2 + 7 + 1 + 5 + 2 + dur_str.len() + 2;
            let cmd_budget = inner_w.saturating_sub(fixed_w).max(8);
            let cmd = truncate_display(&entry.command, cmd_budget);

            let mut spans = vec![
                Span::styled(format!("{ts} "), theme::subtle()),
                Span::styled(format!("{st_text:<6} "), st_style),
                Span::styled(format!("[{src_badge}] "), src_style),
                Span::styled(cmd, theme::primary()),
            ];
            if !dur_str.is_empty() && inner_w > 34 {
                spans.push(Span::styled(format!("  [{dur_str}]"), theme::muted()));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items).highlight_style(theme::selected(focused));
    frame.render_stateful_widget(list, inner, &mut state.list_states.jobs);

    render_scrollbar(
        frame,
        area,
        total_entries,
        inner.height as usize,
        state.list_states.jobs.offset(),
        focused,
    );

    inner
}

fn build_job_detail_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let entries = visible_entries(state);
    let selected_entry = state
        .list_states
        .jobs
        .selected()
        .and_then(|i| entries.get(i).copied());

    let resolved_job = selected_entry
        .and_then(|e| e.job_id)
        .or(state.selected_job_id)
        .and_then(|id| state.jobs.get(&id));

    if let Some(job) = resolved_job {
        let mut lines = Vec::new();
        let (st_label, st_style, dur) = match job.status {
            JobStatus::Running => (
                format!("{} RUNNING", state.spinner_char()),
                theme::accent().add_modifier(Modifier::BOLD),
                format_duration_ms(job.started_at.elapsed().as_millis()),
            ),
            JobStatus::Success { duration_ms } => (
                format!("{} EXIT 0", glyphs::OK),
                theme::success().add_modifier(Modifier::BOLD),
                format_duration_ms(duration_ms),
            ),
            JobStatus::Failed { code, duration_ms } => (
                format!("{} EXIT {code}", glyphs::FAIL),
                theme::danger().add_modifier(Modifier::BOLD),
                format_duration_ms(duration_ms),
            ),
            JobStatus::Killed { duration_ms } => (
                format!("{} KILLED", glyphs::PRUNE),
                theme::warning().add_modifier(Modifier::BOLD),
                format_duration_ms(duration_ms),
            ),
        };

        lines.push(kv_line("JOB", format!("#{} — {}", job.id.0, job.label), theme::label()));
        lines.push(kv_line("COMMAND", job.command_display.clone(), theme::primary()));
        let mut status_spans = vec![
            Span::styled(format!("{:<10} ", "STATUS"), theme::muted().add_modifier(Modifier::BOLD)),
            Span::styled(st_label, st_style),
            Span::styled(format!("   DURATION: {dur}"), theme::muted()),
        ];
        if let Some(pid) = job.pid {
            status_spans.push(Span::styled(format!("   PID: {pid}"), theme::muted()));
        }
        lines.push(Line::from(status_spans));
        lines.push(Line::from(""));
        lines.push(section_divider(
            &format!("LIVE ANSI OUTPUT ({} lines)", job.lines.len()),
            width,
        ));

        if job.lines.is_empty() {
            lines.push(Line::from(Span::styled(
                if job.status.is_running() {
                    "  Waiting for process output…"
                } else {
                    "  (no output captured)"
                },
                theme::muted(),
            )));
        } else {
            for out in &job.lines {
                let base = if out.is_stderr {
                    theme::warning()
                } else {
                    theme::primary()
                };
                lines.push(render_job_output_line(&out.text, base));
            }
        }
        return lines;
    }

    if let Some(entry) = selected_entry {
        let (st_text, st_style, dur) = format_entry_status(entry, state, state.spinner_char());
        let mut lines = vec![
            kv_line("TIMESTAMP", format_hms(entry.timestamp), theme::muted()),
            kv_line(
                "SOURCE",
                match entry.source {
                    CommandSource::User => "User-triggered action",
                    CommandSource::Internal => "Internal mise query",
                },
                theme::accent(),
            ),
            kv_line("STATUS", format!("{st_text} ({dur})"), st_style),
            kv_line("COMMAND", entry.command.clone(), theme::primary()),
        ];
        if let Some(summary) = &entry.summary {
            lines.push(kv_line("SUMMARY", summary.clone(), theme::muted()));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  Actions: ", theme::label()),
            Span::styled("y: copy command   r: rerun   c: clear finished   i: toggle internal", theme::muted()),
        ]));
        return lines;
    }

    empty_state_lines(
        "No Job Selected",
        "Select a command or background job in `[5] COMMAND LOG & JOBS` to inspect its output.",
        Some("Run any task with `b` to stream live ANSI output here."),
    )
}

fn format_entry_status(
    entry: &CommandLogEntry,
    state: &AppState,
    spinner: char,
) -> (String, ratatui::style::Style, String) {
    match entry.status {
        JobStatus::Running => {
            let dur = entry
                .job_id
                .and_then(|id| state.jobs.get(&id))
                .map(|j| format_duration_ms(j.started_at.elapsed().as_millis()))
                .unwrap_or_else(|| "…".to_string());
            (format!("{spinner} RUN"), theme::accent(), dur)
        }
        JobStatus::Success { duration_ms } => (
            format!("{} 0", glyphs::OK),
            theme::success(),
            format_duration_ms(duration_ms),
        ),
        JobStatus::Failed { code, duration_ms } => (
            format!("{} {code}", glyphs::FAIL),
            theme::danger(),
            format_duration_ms(duration_ms),
        ),
        JobStatus::Killed { duration_ms } => (
            format!("{} KILL", glyphs::PRUNE),
            theme::warning(),
            format_duration_ms(duration_ms),
        ),
    }
}
