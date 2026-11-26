pub mod env;
pub mod overlay;
pub mod tasks;
pub mod templates;
pub mod theme;
pub mod tools;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, Tab};

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 40 || area.height < 10 {
        let msg = Paragraph::new("Terminal too small for lazymise (min 40x10). Please resize.")
            .style(theme::warning());
        frame.render_widget(msg, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(frame, app, chunks[0]);

    match app.tab {
        Tab::Tasks => tasks::render(frame, app, chunks[1]),
        Tab::Tools => tools::render(frame, app, chunks[1]),
        Tab::Env => env::render(frame, app, chunks[1]),
        Tab::Templates => templates::render(frame, app, chunks[1]),
    }

    render_footer(frame, app, chunks[2]);
    overlay::render(frame, app);
}

fn render_header(frame: &mut Frame, app: &App, area: Rect) {
    let tabs = [
        (Tab::Tasks, "1:TASKS"),
        (Tab::Tools, "2:TOOLS"),
        (Tab::Env, "3:ENV"),
        (Tab::Templates, "4:TEMPLATES"),
    ];

    let mut spans = vec![
        Span::styled(" lazymise ", theme::label()),
        Span::styled("│ ", theme::muted()),
    ];

    for (tab, label) in tabs {
        if app.tab == tab {
            spans.push(Span::styled(
                format!(" {label} "),
                theme::selected(true).add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(format!(" {label} "), theme::muted()));
        }
        spans.push(Span::raw(" "));
    }

    if !app.filter.is_empty() {
        spans.push(Span::styled(
            format!("  [/{}]", app.filter),
            theme::warning(),
        ));
    }

    spans.push(Span::styled(
        format!("  {}", app.client.workdir().display()),
        theme::muted(),
    ));

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    let hints = match app.tab {
        Tab::Tasks => "Enter:run  x:bg  r:rerun  w:watch  e:edit  p:spec/out  za:fold  /:filter  ?:help  q:quit",
        Tab::Tools => "i:install  u:upgrade  p:prune  /:filter  R:reload  ?:help  q:quit",
        Tab::Env => "S:reveal  y:copy  /:filter  R:reload  ?:help  q:quit",
        Tab::Templates => "Enter/n:scaffold  /:filter  R:reload  ?:help  q:quit",
    };

    let text = if app.status_msg.is_empty() {
        Line::styled(format!(" {hints}"), theme::muted())
    } else {
        Line::from(vec![
            Span::styled(format!(" {} ", app.status_msg), theme::primary()),
            Span::styled(" │ ", theme::muted()),
            Span::styled(hints, theme::muted()),
        ])
    };

    frame.render_widget(Paragraph::new(text), area);
}
