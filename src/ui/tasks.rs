use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use crate::app::{App, FocusPane, TaskPreviewMode};
use crate::model::task_tree::TaskRow;
use crate::runner::RunStatus;
use crate::ui::theme;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let narrow = area.width < 80;
    if narrow {
        match app.focus {
            FocusPane::List => render_list(frame, app, area),
            FocusPane::Preview => render_preview(frame, app, area),
        }
    } else {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(area);
        render_list(frame, app, chunks[0]);
        render_preview(frame, app, chunks[1]);
    }
}

fn render_list(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == FocusPane::List;
    let items: Vec<ListItem> = app
        .task_rows
        .iter()
        .map(|row| match row {
            TaskRow::Group {
                key,
                count,
                collapsed,
            } => {
                let arrow = if *collapsed { "▸" } else { "▾" };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{arrow} {key} "), theme::label()),
                    Span::styled(format!("({count})"), theme::muted()),
                ]))
            }
            TaskRow::Task(task) => {
                let badge = match app.run_records.get(&task.name).map(|r| &r.status) {
                    Some(RunStatus::Running) => Span::styled(" [RUN] ", theme::warning()),
                    Some(RunStatus::Success { .. }) => Span::styled(" [OK]  ", theme::success()),
                    Some(RunStatus::Failed { .. }) => Span::styled(" [ERR] ", theme::danger()),
                    None => Span::styled("       ", theme::muted()),
                };
                let mut spans = vec![
                    Span::raw("  "),
                    Span::styled(task.short_name().to_string(), theme::primary()),
                    badge,
                ];
                if !task.description.is_empty() {
                    spans.push(Span::styled(task.description.clone(), theme::muted()));
                }
                ListItem::new(Line::from(spans))
            }
        })
        .collect();

    let title = format!(" TASKS ({}) ", app.tasks.len());
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme::border(focused))
                .title(Span::styled(title, theme::label())),
        )
        .highlight_style(theme::selected(focused));

    let mut state = ListState::default();
    if !app.task_rows.is_empty() {
        state.select(Some(app.task_idx));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_preview(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == FocusPane::Preview;
    let mode_tag = match app.task_preview_mode {
        TaskPreviewMode::Spec => "SPEC [p:output]",
        TaskPreviewMode::Output => "OUTPUT [p:spec]",
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border(focused))
        .title(Span::styled(format!(" {mode_tag} "), theme::label()));

    let lines = match app.selected_task() {
        None => vec![Line::styled(
            "Select a task (press Space/za on a group to expand).",
            theme::muted(),
        )],
        Some(task) => match app.task_preview_mode {
            TaskPreviewMode::Spec => build_spec_lines(app, task),
            TaskPreviewMode::Output => build_output_lines(app, &task.name),
        },
    };

    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.preview_scroll, 0));
    frame.render_widget(para, area);
}

fn build_spec_lines(app: &App, task: &crate::mise::schema::MiseTask) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(vec![
            Span::styled("NAME     ", theme::muted()),
            Span::styled(
                task.name.clone(),
                theme::primary().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("SCOPE    ", theme::muted()),
            Span::styled(
                if task.global { "global" } else { "workspace" },
                if task.global {
                    theme::warning()
                } else {
                    theme::success()
                },
            ),
        ]),
    ];

    if !task.description.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("DESC     ", theme::muted()),
            Span::styled(task.description.clone(), theme::primary()),
        ]));
    }
    if let Some(dir) = &task.dir {
        lines.push(Line::from(vec![
            Span::styled("DIR      ", theme::muted()),
            Span::styled(dir.clone(), theme::primary()),
        ]));
    }
    if !task.source.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("SOURCE   ", theme::muted()),
            Span::styled(task.source.clone(), theme::muted()),
        ]));
    }
    let deps = task.depends_display();
    if !deps.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("DEPENDS  ", theme::muted()),
            Span::styled(deps.join(", "), theme::warning()),
        ]));
    }
    if !task.sources.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("SOURCES  ", theme::muted()),
            Span::styled(task.sources.join(", "), theme::primary()),
        ]));
    }
    let outs = task.outputs_display();
    if !outs.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("OUTPUTS  ", theme::muted()),
            Span::styled(outs.join(", "), theme::success()),
        ]));
    }

    if let Some(rec) = app.run_records.get(&task.name) {
        let status_line = match rec.status {
            RunStatus::Running => Line::from(vec![
                Span::styled("LAST RUN ", theme::muted()),
                Span::styled("running...", theme::warning()),
            ]),
            RunStatus::Success { duration_ms } => Line::from(vec![
                Span::styled("LAST RUN ", theme::muted()),
                Span::styled(
                    format!("OK ({:.2}s)", duration_ms as f64 / 1000.0),
                    theme::success(),
                ),
            ]),
            RunStatus::Failed { code, duration_ms } => Line::from(vec![
                Span::styled("LAST RUN ", theme::muted()),
                Span::styled(
                    format!("FAILED exit {code} ({:.2}s)", duration_ms as f64 / 1000.0),
                    theme::danger(),
                ),
            ]),
        };
        lines.push(status_line);
    }

    lines.push(Line::raw(""));
    lines.push(Line::styled("SCRIPT / COMMAND", theme::label()));
    lines.push(Line::styled("────────────────", theme::muted()));
    for script_line in app.selected_task_script().lines() {
        lines.push(Line::styled(script_line.to_string(), theme::primary()));
    }
    lines
}

fn build_output_lines(app: &App, task_name: &str) -> Vec<Line<'static>> {
    match app.run_records.get(task_name) {
        None => vec![Line::styled(
            "No captured output yet. Press `x` to run in background or `Enter` for foreground TTY.",
            theme::muted(),
        )],
        Some(rec) => {
            let mut lines = Vec::new();
            let header = match rec.status {
                RunStatus::Running => Line::styled("STATUS: RUNNING", theme::warning()),
                RunStatus::Success { duration_ms } => Line::styled(
                    format!("STATUS: OK ({:.2}s)", duration_ms as f64 / 1000.0),
                    theme::success(),
                ),
                RunStatus::Failed { code, duration_ms } => Line::styled(
                    format!(
                        "STATUS: FAILED exit {} ({:.2}s)",
                        code,
                        duration_ms as f64 / 1000.0
                    ),
                    theme::danger(),
                ),
            };
            lines.push(header);
            lines.push(Line::styled("────────────────", theme::muted()));
            for l in &rec.lines {
                let style = if l.starts_with("[stderr]") {
                    theme::danger()
                } else {
                    theme::primary()
                };
                lines.push(Line::styled(l.clone(), style));
            }
            lines
        }
    }
}
