use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use crate::app::{App, FocusPane};
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
            .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
            .split(area);
        render_list(frame, app, chunks[0]);
        render_preview(frame, app, chunks[1]);
    }
}

fn render_list(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == FocusPane::List;
    let rows = app.filtered_tools();

    let items: Vec<ListItem> = rows
        .iter()
        .map(|row| {
            let state_span = if !row.entry.installed {
                Span::styled("[MISS] ", theme::danger())
            } else if row.entry.scheduled_for_pruning {
                Span::styled("[PRUN] ", theme::warning())
            } else if row.entry.active {
                Span::styled("[ACTV] ", theme::success())
            } else {
                Span::styled("[IDLE] ", theme::muted())
            };
            ListItem::new(Line::from(vec![
                state_span,
                Span::styled(row.name.clone(), theme::primary()),
                Span::styled(format!("  {}", row.entry.version), theme::muted()),
            ]))
        })
        .collect();

    let title = format!(" TOOLS ({}) ", rows.len());
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme::border(focused))
                .title(Span::styled(title, theme::label())),
        )
        .highlight_style(theme::selected(focused));

    let mut state = ListState::default();
    if !rows.is_empty() {
        state.select(Some(app.tool_idx));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_preview(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == FocusPane::Preview;
    let rows = app.filtered_tools();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border(focused))
        .title(Span::styled(" TOOL DETAILS ", theme::label()));

    let lines = match rows.get(app.tool_idx) {
        None => vec![Line::styled("No tools match current filter.", theme::muted())],
        Some(row) => {
            let mut l = vec![
                Line::from(vec![
                    Span::styled("TOOL      ", theme::muted()),
                    Span::styled(row.name.clone(), theme::primary()),
                ]),
                Line::from(vec![
                    Span::styled("VERSION   ", theme::muted()),
                    Span::styled(row.entry.version.clone(), theme::success()),
                ]),
                Line::from(vec![
                    Span::styled("REQUESTED ", theme::muted()),
                    Span::styled(
                        row.entry
                            .requested_version
                            .clone()
                            .unwrap_or_else(|| "—".to_string()),
                        theme::primary(),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("STATE     ", theme::muted()),
                    Span::styled(
                        format!(
                            "installed={} active={} prune={}",
                            row.entry.installed, row.entry.active, row.entry.scheduled_for_pruning
                        ),
                        theme::primary(),
                    ),
                ]),
            ];
            if let Some(src) = &row.entry.source
                && let Some(path) = &src.path
            {
                l.push(Line::from(vec![
                    Span::styled("SOURCE    ", theme::muted()),
                    Span::styled(path.clone(), theme::muted()),
                ]));
            }
            if let Some(install_path) = &row.entry.install_path {
                l.push(Line::from(vec![
                    Span::styled("PATH      ", theme::muted()),
                    Span::styled(install_path.clone(), theme::muted()),
                ]));
            }
            l.push(Line::raw(""));
            l.push(Line::styled(
                "Actions: [i] mise install  [u] mise upgrade  [p] mise prune",
                theme::muted(),
            ));
            l
        }
    };

    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.preview_scroll, 0));
    frame.render_widget(para, area);
}
