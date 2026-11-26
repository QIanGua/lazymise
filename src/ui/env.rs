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
            .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
            .split(area);
        render_list(frame, app, chunks[0]);
        render_preview(frame, app, chunks[1]);
    }
}

fn render_list(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == FocusPane::List;
    let rows = app.filtered_env();

    let items: Vec<ListItem> = rows
        .iter()
        .map(|row| {
            let mut spans = vec![Span::styled(row.key.clone(), theme::primary())];
            if row.sensitive {
                spans.push(Span::styled(" [SECRET]", theme::warning()));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let title = format!(" ENV ({}) ", rows.len());
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
        state.select(Some(app.env_idx));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_preview(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == FocusPane::Preview;
    let rows = app.filtered_env();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border(focused))
        .title(Span::styled(" VALUE [S:reveal  y:copy] ", theme::label()));

    let lines = match rows.get(app.env_idx) {
        None => vec![Line::styled("No environment variables match filter.", theme::muted())],
        Some(row) => {
            let revealed = app.revealed_env_keys.contains(&row.key);
            let mut l = vec![
                Line::from(vec![
                    Span::styled("KEY   ", theme::muted()),
                    Span::styled(row.key.clone(), theme::label()),
                ]),
                Line::raw(""),
            ];

            if row.key == "PATH" {
                l.push(Line::styled("PATH ENTRIES", theme::muted()));
                for (i, entry) in row.value.split(':').enumerate() {
                    l.push(Line::from(vec![
                        Span::styled(format!("{:>2}. ", i + 1), theme::muted()),
                        Span::styled(entry.to_string(), theme::primary()),
                    ]));
                }
            } else {
                l.push(Line::styled(
                    row.display_value(revealed),
                    if row.sensitive && !revealed {
                        theme::warning()
                    } else {
                        theme::primary()
                    },
                ));
            }
            l
        }
    };

    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.preview_scroll, 0));
    frame.render_widget(para, area);
}
