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
            .constraints([Constraint::Percentage(36), Constraint::Percentage(64)])
            .split(area);
        render_list(frame, app, chunks[0]);
        render_preview(frame, app, chunks[1]);
    }
}

fn render_list(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == FocusPane::List;
    let rows = app.filtered_templates();

    let items: Vec<ListItem> = rows
        .iter()
        .map(|tpl| {
            ListItem::new(Line::from(vec![
                Span::styled(tpl.name.clone(), theme::primary()),
                Span::styled(format!("  ({} files)", tpl.files.len()), theme::muted()),
            ]))
        })
        .collect();

    let title = format!(" TEMPLATES ({}) ", rows.len());
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
        state.select(Some(app.template_idx));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_preview(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == FocusPane::Preview;
    let rows = app.filtered_templates();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border(focused))
        .title(Span::styled(
            " TEMPLATE PREVIEW [Enter/n: scaffold] ",
            theme::label(),
        ));

    let lines = match rows.get(app.template_idx) {
        None => vec![Line::styled(
            "No templates found in ~/.config/mise/templates.",
            theme::muted(),
        )],
        Some(tpl) => {
            let mut l = vec![
                Line::from(vec![
                    Span::styled("TEMPLATE ", theme::muted()),
                    Span::styled(tpl.name.clone(), theme::label()),
                ]),
                Line::from(vec![
                    Span::styled("PATH     ", theme::muted()),
                    Span::styled(tpl.path.display().to_string(), theme::muted()),
                ]),
                Line::raw(""),
                Line::styled("FILES", theme::label()),
            ];
            for f in &tpl.files {
                l.push(Line::styled(format!("  {f}"), theme::primary()));
            }
            l.push(Line::raw(""));
            l.push(Line::styled("mise.toml", theme::label()));
            l.push(Line::styled("─────────", theme::muted()));
            for line in tpl.mise_toml_preview.lines() {
                l.push(Line::styled(line.to_string(), theme::primary()));
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
