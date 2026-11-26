use ratatui::style::{Color, Modifier, Style};

pub const SURFACE: Color = Color::Rgb(0x1f, 0x24, 0x30);
pub const PRIMARY: Color = Color::Rgb(0xd8, 0xde, 0xe9);
pub const MUTED: Color = Color::Rgb(0x73, 0x7c, 0x8c);
pub const CYAN: Color = Color::Rgb(0x5c, 0xcf, 0xe6);
pub const AMBER: Color = Color::Rgb(0xff, 0xcc, 0x66);
pub const GREEN: Color = Color::Rgb(0xba, 0xe6, 0x7e);
pub const RED: Color = Color::Rgb(0xff, 0x61, 0x71);

pub fn primary() -> Style {
    Style::default().fg(PRIMARY)
}

pub fn muted() -> Style {
    Style::default().fg(MUTED)
}

pub fn selected(focused: bool) -> Style {
    if focused {
        Style::default()
            .bg(SURFACE)
            .fg(PRIMARY)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(SURFACE).fg(MUTED)
    }
}

pub fn border(focused: bool) -> Style {
    if focused {
        Style::default().fg(CYAN)
    } else {
        Style::default().fg(MUTED)
    }
}

pub fn label() -> Style {
    Style::default().fg(CYAN).add_modifier(Modifier::BOLD)
}

pub fn success() -> Style {
    Style::default().fg(GREEN)
}

pub fn warning() -> Style {
    Style::default().fg(AMBER)
}

pub fn danger() -> Style {
    Style::default().fg(RED)
}
