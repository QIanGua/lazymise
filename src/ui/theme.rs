use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};

use crate::action::ToastLevel;

// Canonical TrueColor palette from `docs/design.md` (retained as public constants).
pub const SURFACE: Color = Color::Rgb(0x1f, 0x24, 0x30);
pub const SURFACE_ELEVATED: Color = Color::Rgb(0x26, 0x2c, 0x3a);
pub const PRIMARY: Color = Color::Rgb(0xd8, 0xde, 0xe9);
pub const MUTED: Color = Color::Rgb(0x73, 0x7c, 0x8c);
pub const SUBTLE: Color = Color::Rgb(0x4c, 0x56, 0x6a);
pub const CYAN: Color = Color::Rgb(0x5c, 0xcf, 0xe6);
pub const AMBER: Color = Color::Rgb(0xff, 0xcc, 0x66);
pub const GREEN: Color = Color::Rgb(0xba, 0xe6, 0x7e);
pub const RED: Color = Color::Rgb(0xff, 0x61, 0x71);
pub const PURPLE: Color = Color::Rgb(0xc3, 0xa6, 0xff);

/// Non-color redundant status glyphs so color is never the sole indicator.
pub mod glyphs {
    pub const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    pub const OK: &str = "✓";
    pub const FAIL: &str = "✗";
    pub const RUNNING: &str = "●";
    pub const IDLE: &str = "○";
    pub const UPGRADE: &str = "↑";
    pub const WARN: &str = "⚠";
    pub const PRUNE: &str = "✂";
    pub const LOCK: &str = "🔒";
    pub const UNLOCK: &str = "🔓";
    pub const TOOL: &str = "⚙";
    pub const VAR: &str = "◈";
    pub const CACHE: &str = "⚡";
    pub const FOLD_OPEN: &str = "▾";
    pub const FOLD_CLOSED: &str = "▸";
    pub const BULLET: &str = "•";
    pub const INACTIVE: &str = "·";
}

/// Terminal color capability detected from standard environment variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCapability {
    TrueColor,
    Ansi256,
    Ansi16,
    NoColor,
}

/// Resolved color palette adapted to the terminal's color depth (including `NO_COLOR`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub capability: ColorCapability,
    pub surface: Option<Color>,
    pub surface_elevated: Option<Color>,
    pub primary: Option<Color>,
    pub muted: Option<Color>,
    pub subtle: Option<Color>,
    pub cyan: Option<Color>,
    pub amber: Option<Color>,
    pub green: Option<Color>,
    pub red: Option<Color>,
    pub purple: Option<Color>,
}

impl Palette {
    pub const fn for_capability(capability: ColorCapability) -> Self {
        match capability {
            ColorCapability::TrueColor => Self {
                capability,
                surface: Some(SURFACE),
                surface_elevated: Some(SURFACE_ELEVATED),
                primary: Some(PRIMARY),
                muted: Some(MUTED),
                subtle: Some(SUBTLE),
                cyan: Some(CYAN),
                amber: Some(AMBER),
                green: Some(GREEN),
                red: Some(RED),
                purple: Some(PURPLE),
            },
            ColorCapability::Ansi256 => Self {
                capability,
                surface: Some(Color::Indexed(235)),
                surface_elevated: Some(Color::Indexed(236)),
                primary: Some(Color::Indexed(253)),
                muted: Some(Color::Indexed(243)),
                subtle: Some(Color::Indexed(239)),
                cyan: Some(Color::Indexed(80)),
                amber: Some(Color::Indexed(221)),
                green: Some(Color::Indexed(150)),
                red: Some(Color::Indexed(203)),
                purple: Some(Color::Indexed(147)),
            },
            ColorCapability::Ansi16 => Self {
                capability,
                surface: Some(Color::DarkGray),
                surface_elevated: Some(Color::DarkGray),
                primary: Some(Color::White),
                muted: Some(Color::Gray),
                subtle: Some(Color::DarkGray),
                cyan: Some(Color::Cyan),
                amber: Some(Color::Yellow),
                green: Some(Color::Green),
                red: Some(Color::Red),
                purple: Some(Color::Magenta),
            },
            ColorCapability::NoColor => Self {
                capability,
                surface: None,
                surface_elevated: None,
                primary: None,
                muted: None,
                subtle: None,
                cyan: None,
                amber: None,
                green: None,
                red: None,
                purple: None,
            },
        }
    }

    pub fn is_no_color(self) -> bool {
        self.capability == ColorCapability::NoColor
    }
}

/// Pure capability detector (unit-testable without mutating process env).
pub fn detect_color_capability_from(
    no_color: Option<&str>,
    clicolor: Option<&str>,
    colorterm: Option<&str>,
    term: Option<&str>,
) -> ColorCapability {
    // Per https://no-color.org/, NO_COLOR being present and non-empty disables color.
    if no_color.is_some_and(|v| !v.is_empty()) {
        return ColorCapability::NoColor;
    }
    if clicolor == Some("0") {
        return ColorCapability::NoColor;
    }
    if let Some(t) = term {
        let lower = t.to_ascii_lowercase();
        if lower == "dumb" {
            return ColorCapability::NoColor;
        }
    }
    if let Some(ct) = colorterm {
        let lower = ct.to_ascii_lowercase();
        if lower.contains("truecolor") || lower.contains("24bit") {
            return ColorCapability::TrueColor;
        }
    }
    if let Some(t) = term {
        let lower = t.to_ascii_lowercase();
        if lower.contains("direct")
            || lower.contains("kitty")
            || lower.contains("wezterm")
            || lower.contains("alacritty")
            || lower.contains("ghostty")
            || lower.contains("iterm")
        {
            return ColorCapability::TrueColor;
        }
        if lower.contains("256color") || lower.contains("tmux") || lower.contains("screen") {
            return ColorCapability::Ansi256;
        }
        if !lower.is_empty() {
            return ColorCapability::Ansi16;
        }
    }
    // Modern default (including TestBackend where TERM may be unset in CI sandboxes).
    ColorCapability::TrueColor
}

pub fn active_palette() -> Palette {
    static PALETTE: OnceLock<Palette> = OnceLock::new();
    *PALETTE.get_or_init(|| {
        let no_color = std::env::var("NO_COLOR").ok();
        let clicolor = std::env::var("CLICOLOR").ok();
        let colorterm = std::env::var("COLORTERM").ok();
        let term = std::env::var("TERM").ok();
        let cap = detect_color_capability_from(
            no_color.as_deref(),
            clicolor.as_deref(),
            colorterm.as_deref(),
            term.as_deref(),
        );
        Palette::for_capability(cap)
    })
}

fn fg_opt(color: Option<Color>) -> Style {
    match color {
        Some(c) => Style::default().fg(c),
        None => Style::default(),
    }
}

pub fn primary() -> Style {
    fg_opt(active_palette().primary)
}

pub fn muted() -> Style {
    let p = active_palette();
    if p.is_no_color() {
        Style::default().add_modifier(Modifier::DIM)
    } else {
        fg_opt(p.muted)
    }
}

pub fn subtle() -> Style {
    let p = active_palette();
    if p.is_no_color() {
        Style::default().add_modifier(Modifier::DIM)
    } else {
        fg_opt(p.subtle)
    }
}

pub fn selected(focused: bool) -> Style {
    let p = active_palette();
    if p.is_no_color() {
        return if focused {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::UNDERLINED)
        };
    }
    if focused {
        let mut s = Style::default().add_modifier(Modifier::BOLD);
        if let Some(bg) = p.surface {
            s = s.bg(bg);
        }
        if let Some(fg) = p.primary {
            s = s.fg(fg);
        }
        s
    } else {
        let mut s = Style::default();
        if let Some(bg) = p.surface {
            s = s.bg(bg);
        }
        if let Some(fg) = p.muted {
            s = s.fg(fg);
        }
        s
    }
}

pub fn border(focused: bool) -> Style {
    let p = active_palette();
    if p.is_no_color() {
        return if focused {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::DIM)
        };
    }
    if focused {
        fg_opt(p.cyan)
    } else {
        fg_opt(p.subtle)
    }
}

pub fn panel_title(focused: bool) -> Style {
    let p = active_palette();
    if focused {
        fg_opt(p.cyan).add_modifier(Modifier::BOLD)
    } else if p.is_no_color() {
        Style::default().add_modifier(Modifier::DIM)
    } else {
        fg_opt(p.muted)
    }
}

pub fn label() -> Style {
    fg_opt(active_palette().cyan).add_modifier(Modifier::BOLD)
}

pub fn accent() -> Style {
    fg_opt(active_palette().cyan)
}

pub fn highlight_match() -> Style {
    let p = active_palette();
    if p.is_no_color() {
        Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        fg_opt(p.amber).add_modifier(Modifier::BOLD)
    }
}

pub fn success() -> Style {
    fg_opt(active_palette().green)
}

pub fn warning() -> Style {
    fg_opt(active_palette().amber)
}

pub fn danger() -> Style {
    fg_opt(active_palette().red)
}

pub fn purple() -> Style {
    fg_opt(active_palette().purple)
}

pub fn brand_pill() -> Style {
    let p = active_palette();
    if p.is_no_color() {
        return Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD);
    }
    let mut s = Style::default().add_modifier(Modifier::BOLD);
    if let Some(bg) = p.surface_elevated {
        s = s.bg(bg);
    }
    if let Some(fg) = p.cyan {
        s = s.fg(fg);
    }
    s
}

pub fn subtab(active: bool, pane_focused: bool) -> Style {
    let p = active_palette();
    if active {
        if p.is_no_color() {
            Style::default().add_modifier(if pane_focused {
                Modifier::REVERSED | Modifier::BOLD
            } else {
                Modifier::BOLD | Modifier::UNDERLINED
            })
        } else if pane_focused {
            let mut s = Style::default().add_modifier(Modifier::BOLD);
            if let Some(fg) = p.cyan {
                s = s.fg(fg);
            }
            if let Some(bg) = p.surface {
                s = s.bg(bg);
            }
            s
        } else {
            fg_opt(p.primary).add_modifier(Modifier::BOLD)
        }
    } else {
        muted()
    }
}

pub fn filter_pill(active: bool) -> Style {
    let p = active_palette();
    if active {
        if p.is_no_color() {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            let mut s = Style::default().add_modifier(Modifier::BOLD);
            if let Some(bg) = p.surface_elevated {
                s = s.bg(bg);
            }
            if let Some(fg) = p.cyan {
                s = s.fg(fg);
            }
            s
        }
    } else {
        muted()
    }
}

pub fn key_cap() -> Style {
    let p = active_palette();
    if p.is_no_color() {
        return Style::default().add_modifier(Modifier::BOLD);
    }
    let mut s = Style::default().add_modifier(Modifier::BOLD);
    if let Some(fg) = p.cyan {
        s = s.fg(fg);
    }
    if let Some(bg) = p.surface {
        s = s.bg(bg);
    }
    s
}

pub fn section_header() -> Style {
    fg_opt(active_palette().primary).add_modifier(Modifier::BOLD)
}

pub fn surface_bg() -> Style {
    let p = active_palette();
    match p.surface {
        Some(bg) => Style::default().bg(bg),
        None => Style::default(),
    }
}

pub fn toast(level: ToastLevel) -> Style {
    match level {
        ToastLevel::Info => label(),
        ToastLevel::Success => success().add_modifier(Modifier::BOLD),
        ToastLevel::Warning => warning().add_modifier(Modifier::BOLD),
        ToastLevel::Error => danger().add_modifier(Modifier::BOLD),
    }
}

pub fn toast_border(level: ToastLevel) -> Style {
    match level {
        ToastLevel::Info => accent(),
        ToastLevel::Success => success(),
        ToastLevel::Warning => warning(),
        ToastLevel::Error => danger(),
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn detects_no_color_and_terminal_color_depths() {
        assert_eq!(
            detect_color_capability_from(Some("1"), None, Some("truecolor"), Some("xterm-256color")),
            ColorCapability::NoColor
        );
        assert_eq!(
            detect_color_capability_from(Some(""), Some("0"), Some("truecolor"), Some("xterm-256color")),
            ColorCapability::NoColor
        );
        assert_eq!(
            detect_color_capability_from(None, None, None, Some("dumb")),
            ColorCapability::NoColor
        );
        assert_eq!(
            detect_color_capability_from(None, None, Some("truecolor"), Some("xterm-256color")),
            ColorCapability::TrueColor
        );
        assert_eq!(
            detect_color_capability_from(None, None, None, Some("xterm-256color")),
            ColorCapability::Ansi256
        );
        assert_eq!(
            detect_color_capability_from(None, None, None, Some("vt100")),
            ColorCapability::Ansi16
        );

        let no_color_palette = Palette::for_capability(ColorCapability::NoColor);
        assert!(no_color_palette.is_no_color());
        assert_eq!(no_color_palette.cyan, None);
        assert_eq!(no_color_palette.surface, None);

        let p256 = Palette::for_capability(ColorCapability::Ansi256);
        assert_eq!(p256.cyan, Some(Color::Indexed(80)));
    }
}
