use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::{App, Overlay};
use crate::ui::theme;

pub fn render(frame: &mut Frame, app: &App) {
    match &app.overlay {
        Overlay::None => {}
        Overlay::Help => render_help(frame),
        Overlay::Filter => render_filter(frame, app),
        Overlay::Confirm {
            title,
            command_preview,
            ..
        } => render_confirm(frame, title, command_preview),
        Overlay::TemplateNameInput { template, input } => {
            render_template_input(frame, template, input);
        }
    }
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(height.min(area.height.saturating_sub(2))),
            Constraint::Fill(1),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}

fn render_help(frame: &mut Frame) {
    let area = centered_rect(72, 22, frame.area());
    frame.render_widget(Clear, area);

    let lines = vec![
        Line::styled("GLOBAL NAVIGATION", theme::label()),
        Line::raw("  1 / 2 / 3 / 4     Switch tab (Tasks / Tools / Env / Templates)"),
        Line::raw("  j / k, g g, G     Move down / up / top / bottom"),
        Line::raw("  Ctrl-d / Ctrl-u   Half-page down / up"),
        Line::raw("  h / l / Tab       Focus list / preview pane"),
        Line::raw("  /  (Esc)          Filter current list (Esc clears filter)"),
        Line::raw("  R / ? / q         Reload mise state / toggle help / quit"),
        Line::raw(""),
        Line::styled("TASKS", theme::label()),
        Line::raw("  Enter             Run task in foreground TTY (suspends TUI)"),
        Line::raw("  x                 Run task in background with live output"),
        Line::raw("  r / w             Re-run last task / run `mise watch`"),
        Line::raw("  e / y             Edit source in $EDITOR / copy command"),
        Line::raw("  za / Space / p    Fold group / toggle Spec vs Output preview"),
        Line::raw(""),
        Line::styled("TOOLS / ENV / TEMPLATES", theme::label()),
        Line::raw("  Tools: i install  u upgrade  p prune"),
        Line::raw("  Env:   S reveal secret  y copy value"),
        Line::raw("  Tpl:   Enter / n scaffold new workspace (`mise run new`)"),
    ];

    let widget = Paragraph::new(lines)
        .style(Style::default().bg(theme::SURFACE).fg(theme::PRIMARY))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme::border(true))
                .style(Style::default().bg(theme::SURFACE))
                .title(Span::styled(" KEYBINDINGS [Esc/? to close] ", theme::label())),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(widget, area);
}

fn render_filter(frame: &mut Frame, app: &App) {
    let area = centered_rect(60, 5, frame.area());
    frame.render_widget(Clear, area);

    let lines = vec![
        Line::from(vec![
            Span::styled("/ ", theme::label()),
            Span::styled(app.filter.clone(), theme::primary()),
            Span::styled("█", theme::warning()),
        ]),
        Line::styled("Enter: apply   Esc: clear", theme::muted()),
    ];

    let widget = Paragraph::new(lines)
        .style(Style::default().bg(theme::SURFACE).fg(theme::PRIMARY))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme::border(true))
                .style(Style::default().bg(theme::SURFACE))
                .title(Span::styled(" FILTER ", theme::label())),
        );
    frame.render_widget(widget, area);
}

fn render_confirm(frame: &mut Frame, title: &str, command_preview: &str) {
    let area = centered_rect(68, 7, frame.area());
    frame.render_widget(Clear, area);

    let lines = vec![
        Line::styled(title.to_string(), theme::primary()),
        Line::raw(""),
        Line::from(vec![
            Span::styled("COMMAND  ", theme::muted()),
            Span::styled(command_preview.to_string(), theme::warning()),
        ]),
        Line::raw(""),
        Line::styled("[y/Enter] Confirm   [n/Esc] Cancel", theme::label()),
    ];

    let widget = Paragraph::new(lines)
        .style(Style::default().bg(theme::SURFACE).fg(theme::PRIMARY))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme::warning())
                .style(Style::default().bg(theme::SURFACE))
                .title(Span::styled(" CONFIRM ACTION ", theme::warning())),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(widget, area);
}

fn render_template_input(frame: &mut Frame, template: &str, input: &str) {
    let area = centered_rect(62, 6, frame.area());
    frame.render_widget(Clear, area);

    let lines = vec![
        Line::from(vec![
            Span::styled("Template: ", theme::muted()),
            Span::styled(template.to_string(), theme::label()),
        ]),
        Line::from(vec![
            Span::styled("Project name: ", theme::muted()),
            Span::styled(input.to_string(), theme::primary()),
            Span::styled("█", theme::warning()),
        ]),
        Line::styled("Enter: continue   Esc: cancel", theme::muted()),
    ];

    let widget = Paragraph::new(lines)
        .style(Style::default().bg(theme::SURFACE).fg(theme::PRIMARY))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme::border(true))
                .style(Style::default().bg(theme::SURFACE))
                .title(Span::styled(" NEW WORKSPACE ", theme::label())),
        );
    frame.render_widget(widget, area);
}
