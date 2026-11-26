use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    Wrap,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::action::ToastLevel;
use crate::app::state::{AppState, TextInput};
use crate::runner::ansi::{parse_ansi_line, strip_ansi};
use crate::ui::layout::LayoutMap;
use crate::ui::theme::{self, glyphs};

/// Truncate a string cleanly to fit within `max_width` terminal display columns (CJK/emoji safe).
pub fn truncate_display(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 1 {
        return "…".to_string();
    }
    let mut out = String::new();
    let mut width = 0usize;
    let limit = max_width - 1;
    for ch in text.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + cw > limit {
            break;
        }
        out.push(ch);
        width += cw;
    }
    out.push('…');
    out
}

/// Alias matching design specification naming (`truncate_display_width`).
pub fn truncate_display_width(text: &str, max_width: usize) -> String {
    truncate_display(text, max_width)
}

/// Pad or truncate `text` so its terminal display width equals `target_width` (CJK/emoji safe).
pub fn pad_display_width(text: &str, target_width: usize) -> String {
    let fitted = truncate_display(text, target_width);
    let w = UnicodeWidthStr::width(fitted.as_str());
    if w >= target_width {
        fitted
    } else {
        format!("{fitted}{}", " ".repeat(target_width - w))
    }
}

/// Shorten a filesystem path by replacing `$HOME` with `~` and collapsing middle segments
/// so it fits cleanly within `max_width` terminal columns (CJK/emoji safe).
pub fn shorten_path(path: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let normalized = replace_home_prefix(path);
    if UnicodeWidthStr::width(normalized.as_str()) <= max_width {
        return normalized;
    }

    let parts: Vec<&str> = normalized.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() <= 2 {
        return truncate_display(&normalized, max_width);
    }

    let leading_tilde = normalized.starts_with("~/");
    let leading_slash = normalized.starts_with('/');
    let prefix = if leading_tilde {
        "~/"
    } else if leading_slash {
        "/"
    } else {
        ""
    };

    let tail = parts[parts.len() - 1];
    let parent = parts[parts.len() - 2];

    // Try `~/…/<parent>/<tail>` first, then `…/<tail>`, then hard truncate.
    let candidate_two = format!("{prefix}…/{parent}/{tail}");
    if UnicodeWidthStr::width(candidate_two.as_str()) <= max_width {
        return candidate_two;
    }
    let candidate_one = format!("{prefix}…/{tail}");
    if UnicodeWidthStr::width(candidate_one.as_str()) <= max_width {
        return candidate_one;
    }
    let candidate_leaf = format!("…/{tail}");
    if UnicodeWidthStr::width(candidate_leaf.as_str()) <= max_width {
        return candidate_leaf;
    }
    truncate_display(tail, max_width)
}

fn replace_home_prefix(path: &str) -> String {
    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
    {
        if path == home {
            return "~".to_string();
        }
        if let Some(rest) = path.strip_prefix(&home)
            && rest.starts_with('/')
        {
            return format!("~{rest}");
        }
    }
    path.to_string()
}

/// Render text with fuzzy-matched character indices highlighted.
pub fn highlighted_spans(
    text: &str,
    matched_indices: &[usize],
    base_style: Style,
) -> Vec<Span<'static>> {
    if matched_indices.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }
    let mut spans = Vec::new();
    let mut buf = String::new();
    let mut prev_hi = false;
    for (i, ch) in text.chars().enumerate() {
        let is_hi = matched_indices.binary_search(&i).is_ok();
        if i > 0 && is_hi != prev_hi && !buf.is_empty() {
            spans.push(Span::styled(
                std::mem::take(&mut buf),
                if prev_hi {
                    theme::highlight_match()
                } else {
                    base_style
                },
            ));
        }
        buf.push(ch);
        prev_hi = is_hi;
    }
    if !buf.is_empty() {
        spans.push(Span::styled(
            buf,
            if prev_hi {
                theme::highlight_match()
            } else {
                base_style
            },
        ));
    }
    spans
}

/// Alias matching design specification naming (`highlight_fuzzy_spans`).
pub fn highlight_fuzzy_spans(
    text: &str,
    matched_indices: &[usize],
    base_style: Style,
) -> Vec<Span<'static>> {
    highlighted_spans(text, matched_indices, base_style)
}

/// Render fuzzy-highlighted text truncated cleanly to `max_width` display columns.
pub fn highlighted_truncated_spans(
    text: &str,
    matched_indices: &[usize],
    base_style: Style,
    max_width: usize,
) -> Vec<Span<'static>> {
    if max_width == 0 {
        return Vec::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return highlighted_spans(text, matched_indices, base_style);
    }
    if max_width == 1 {
        return vec![Span::styled("…".to_string(), base_style)];
    }

    let limit = max_width - 1;
    let mut width = 0usize;
    let mut spans = Vec::new();
    let mut buf = String::new();
    let mut prev_hi = false;
    let mut has_any = false;

    for (i, ch) in text.chars().enumerate() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + cw > limit {
            break;
        }
        let is_hi = !matched_indices.is_empty() && matched_indices.binary_search(&i).is_ok();
        if has_any && is_hi != prev_hi && !buf.is_empty() {
            spans.push(Span::styled(
                std::mem::take(&mut buf),
                if prev_hi {
                    theme::highlight_match()
                } else {
                    base_style
                },
            ));
        }
        buf.push(ch);
        width += cw;
        prev_hi = is_hi;
        has_any = true;
    }

    if !buf.is_empty() {
        spans.push(Span::styled(
            buf,
            if prev_hi {
                theme::highlight_match()
            } else {
                base_style
            },
        ));
    }
    spans.push(Span::styled("…".to_string(), base_style));
    spans
}

/// Rounded panel block with consistent focus border & title styling.
pub fn panel_block(
    title: Line<'static>,
    right_title: Option<Line<'static>>,
    focused: bool,
) -> Block<'static> {
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border(focused))
        .title_top(title);
    if let Some(rt) = right_title {
        block = block.title_top(rt.right_aligned());
    }
    block
}

/// Build a main-pane rounded block with clickable sub-view tabs on the top border
/// and register every sub-tab's exact `Rect` into `layout_map.main_subtabs`.
pub fn main_subtab_block(
    area: Rect,
    labels: &[&str],
    active_idx: usize,
    focused: bool,
    extra_right_hint: Option<&str>,
    layout_map: &mut LayoutMap,
) -> Block<'static> {
    let mut title_spans: Vec<Span<'static>> = Vec::new();
    // Top border renders `╭` at `area.x`, so title content starts at `area.x + 1`.
    let mut cursor_x = area.x.saturating_add(1);
    let max_x = area.x.saturating_add(area.width.saturating_sub(1));

    title_spans.push(Span::styled(" ", theme::muted()));
    cursor_x = cursor_x.saturating_add(1);

    for (idx, label) in labels.iter().enumerate() {
        if idx > 0 {
            let sep = " │ ";
            let sep_w = UnicodeWidthStr::width(sep) as u16;
            if cursor_x.saturating_add(sep_w) < max_x {
                title_spans.push(Span::styled(sep, theme::subtle()));
                cursor_x = cursor_x.saturating_add(sep_w);
            }
        }
        let text = if idx == active_idx {
            format!("[{label}]")
        } else {
            format!(" {label} ")
        };
        let w = UnicodeWidthStr::width(text.as_str()) as u16;
        if cursor_x < max_x {
            let clamped_w = w.min(max_x.saturating_sub(cursor_x));
            if clamped_w > 0 {
                layout_map
                    .main_subtabs
                    .push((idx, Rect::new(cursor_x, area.y, clamped_w, 1)));
            }
            title_spans.push(Span::styled(text, theme::subtab(idx == active_idx, focused)));
            cursor_x = cursor_x.saturating_add(w);
        }
    }
    title_spans.push(Span::styled(" ", theme::muted()));

    let hint = extra_right_hint.unwrap_or("[ / ] switch");
    let right_needed = UnicodeWidthStr::width(hint) as u16 + 4;
    let right_title = if cursor_x.saturating_add(right_needed) < max_x {
        Some(Line::from(vec![
            Span::styled(" ", theme::muted()),
            Span::styled(hint.to_string(), theme::muted()),
            Span::styled(" ", theme::muted()),
        ]))
    } else {
        None
    };

    panel_block(Line::from(title_spans), right_title, focused)
}

/// Render a refined vertical scrollbar along the right border when content exceeds viewport.
pub fn render_scrollbar(
    frame: &mut Frame,
    area: Rect,
    total_items: usize,
    viewport_height: usize,
    position: usize,
    focused: bool,
) {
    if area.height < 3 || area.width < 4 || total_items <= viewport_height || viewport_height == 0 {
        return;
    }
    let max_pos = total_items.saturating_sub(viewport_height);
    let mut sb_state = ScrollbarState::new(max_pos.saturating_add(1))
        .viewport_content_length(viewport_height)
        .position(position.min(max_pos));

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None)
        .track_symbol(Some("│"))
        .thumb_symbol("┃")
        .track_style(theme::subtle())
        .thumb_style(if focused {
            theme::accent()
        } else {
            theme::muted()
        });

    let sb_area = Rect {
        x: area.x,
        y: area.y.saturating_add(1),
        width: area.width,
        height: area.height.saturating_sub(2),
    };
    frame.render_stateful_widget(scrollbar, sb_area, &mut sb_state);
}

/// Render a floating top-right Toast notification card that never obstructs the left sidebar.
pub fn render_toast_overlay(frame: &mut Frame, full_area: Rect, state: &AppState) {
    let Some(toast) = state.active_toast() else {
        return;
    };
    if full_area.width < 32 || full_area.height < 8 {
        return;
    }

    let (icon, level_label) = match toast.level {
        ToastLevel::Info => ("ℹ", "INFO"),
        ToastLevel::Success => (glyphs::OK, "OK"),
        ToastLevel::Warning => (glyphs::WARN, "WARN"),
        ToastLevel::Error => (glyphs::FAIL, "ERR"),
    };

    let max_card_w = (full_area.width / 2).clamp(24, 56).min(full_area.width.saturating_sub(2));
    let content_w = max_card_w.saturating_sub(4) as usize;
    let msg_text = format!("{icon} {}", toast.message);
    let msg_width = UnicodeWidthStr::width(msg_text.as_str());
    let wrapped_lines = if content_w == 0 {
        1
    } else {
        msg_width.div_ceil(content_w).clamp(1, 3)
    };
    let card_w = (msg_width.saturating_add(4) as u16)
        .clamp(18, max_card_w)
        .min(full_area.width.saturating_sub(2));
    let card_h = (wrapped_lines as u16 + 2).min(full_area.height.saturating_sub(3));
    let x = full_area.x.saturating_add(full_area.width.saturating_sub(card_w + 1));
    // Place at `y + 2` so neither the header (row 0) nor clickable main sub-tabs (row 1) are covered.
    let y = full_area.y.saturating_add(2);
    let rect = Rect::new(x, y, card_w, card_h);

    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::toast_border(toast.level))
        .title_top(Line::from(vec![
            Span::styled(" ", theme::muted()),
            Span::styled(level_label, theme::toast(toast.level)),
            Span::styled(" ", theme::muted()),
        ]));

    let para = Paragraph::new(Line::from(vec![
        Span::styled(format!("{icon} "), theme::toast(toast.level)),
        Span::styled(toast.message.clone(), theme::primary()),
    ]))
    .block(block)
    .wrap(Wrap { trim: false });
    frame.render_widget(para, rect);
}

/// Render an aligned `KEY    VALUE` metadata row.
pub fn kv_line(key: &str, value: impl Into<String>, value_style: Style) -> Line<'static> {
    let key_padded = format!("{:<10}", key);
    Line::from(vec![
        Span::styled(key_padded, theme::muted().add_modifier(Modifier::BOLD)),
        Span::styled(" ", theme::muted()),
        Span::styled(value.into(), value_style),
    ])
}

/// Render a crisp horizontal section divider with title.
pub fn section_divider(title: &str, width: usize) -> Line<'static> {
    let prefix = format!("── {title} ");
    let used = UnicodeWidthStr::width(prefix.as_str());
    let fill = width.saturating_sub(used);
    Line::from(vec![
        Span::styled("── ", theme::subtle()),
        Span::styled(title.to_string(), theme::label()),
        Span::styled(" ", theme::subtle()),
        Span::styled("─".repeat(fill), theme::subtle()),
    ])
}

/// Build a friendly, actionable empty-state block of lines.
pub fn empty_state_lines(title: &str, detail: &str, hint: Option<&str>) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(format!("  {title}"), theme::section_header())),
        Line::from(Span::styled(format!("  {detail}"), theme::muted())),
    ];
    if let Some(h) = hint {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  Tip: ", theme::label()),
            Span::styled(h.to_string(), theme::primary()),
        ]));
    }
    lines
}

/// Format elapsed / duration milliseconds compactly (`12ms`, `0.37s`, `1.2s`, `2m 14s`).
pub fn format_duration_ms(ms: u128) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        let secs = ms as f64 / 1000.0;
        if secs < 10.0 {
            format!("{secs:.2}s")
        } else {
            format!("{secs:.1}s")
        }
    } else {
        let total_secs = ms / 1000;
        let mins = total_secs / 60;
        let rem = total_secs % 60;
        format!("{mins}m {rem:02}s")
    }
}

/// Format a `SystemTime` as `HH:MM:SS` (UTC) without external dependencies or unsafe FFI.
pub fn format_hms(ts: SystemTime) -> String {
    let secs = ts
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let day_secs = secs % 86_400;
    let h = day_secs / 3_600;
    let m = (day_secs % 3_600) / 60;
    let s = day_secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

/// Render a `TextInput` with a visible Unicode-safe cursor and horizontal windowing.
pub fn text_input_spans(
    input: &TextInput,
    placeholder: &str,
    focused: bool,
    max_width: usize,
) -> Vec<Span<'static>> {
    if input.value.is_empty() {
        return if focused {
            vec![
                Span::styled(
                    " ",
                    theme::primary().add_modifier(Modifier::REVERSED),
                ),
                Span::styled(
                    truncate_display(placeholder, max_width.saturating_sub(1)),
                    theme::muted(),
                ),
            ]
        } else {
            vec![Span::styled(
                truncate_display(placeholder, max_width),
                theme::muted(),
            )]
        };
    }

    let chars: Vec<char> = input.value.chars().collect();
    let cursor = input.cursor.min(chars.len());

    // Compute visible character slice keeping cursor in view within `max_width`.
    let budget = max_width.max(2);
    let mut start = 0usize;
    let cursor_prefix_width = |from: usize| -> usize {
        chars[from..cursor]
            .iter()
            .map(|c| UnicodeWidthChar::width(*c).unwrap_or(0))
            .sum()
    };
    while start < cursor && cursor_prefix_width(start) >= budget.saturating_sub(1) {
        start += 1;
    }

    let mut before = String::new();
    let mut at_cursor = String::new();
    let mut after = String::new();
    let mut used = 0usize;

    for (idx, &ch) in chars.iter().enumerate().skip(start) {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cw > budget {
            break;
        }
        if idx < cursor {
            before.push(ch);
        } else if idx == cursor {
            at_cursor.push(ch);
        } else {
            after.push(ch);
        }
        used += cw;
    }

    let mut spans = Vec::new();
    if !before.is_empty() {
        spans.push(Span::styled(before, theme::primary()));
    }
    if focused {
        let cursor_glyph = if at_cursor.is_empty() {
            " ".to_string()
        } else {
            at_cursor
        };
        spans.push(Span::styled(
            cursor_glyph,
            theme::primary().add_modifier(Modifier::REVERSED | Modifier::BOLD),
        ));
    } else if !at_cursor.is_empty() {
        spans.push(Span::styled(at_cursor, theme::primary()));
    }
    if !after.is_empty() {
        spans.push(Span::styled(after, theme::primary()));
    }
    spans
}

/// Lightweight zero-dependency TOML syntax highlighter for cached config files.
pub fn highlight_toml_line(line: &str) -> Line<'static> {
    let trimmed = line.trim_start();
    let indent_len = line.len() - trimmed.len();
    let indent = &line[..indent_len];

    if trimmed.is_empty() {
        return Line::from("");
    }
    if trimmed.starts_with('#') {
        return Line::from(Span::styled(line.to_string(), theme::muted()));
    }
    if trimmed.starts_with('[') {
        return Line::from(vec![
            Span::raw(indent.to_string()),
            Span::styled(trimmed.to_string(), theme::label()),
        ]);
    }
    if let Some((lhs, rhs)) = trimmed.split_once('=') {
        let mut spans = vec![
            Span::raw(indent.to_string()),
            Span::styled(lhs.to_string(), theme::accent()),
            Span::styled("=", theme::subtle()),
        ];
        let rhs_trim = rhs.trim();
        let val_style = if rhs_trim.starts_with('"') || rhs_trim.starts_with('\'') {
            theme::success()
        } else if matches!(rhs_trim, "true" | "false")
            || rhs_trim
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
        {
            theme::warning()
        } else {
            theme::primary()
        };
        spans.push(Span::styled(rhs.to_string(), val_style));
        return Line::from(spans);
    }
    Line::from(Span::styled(line.to_string(), theme::primary()))
}

/// Lightweight zero-dependency shell/script syntax highlighter for cached task scripts.
pub fn highlight_shell_line(line: &str) -> Line<'static> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return Line::from("");
    }
    if trimmed.starts_with("#!") {
        return Line::from(Span::styled(
            line.to_string(),
            theme::purple().add_modifier(Modifier::ITALIC),
        ));
    }
    if trimmed.starts_with('#') {
        return Line::from(Span::styled(line.to_string(), theme::muted()));
    }

    let indent_len = line.len() - trimmed.len();
    let mut spans = Vec::new();
    if indent_len > 0 {
        spans.push(Span::raw(line[..indent_len].to_string()));
    }

    // Highlight first command/keyword token, flags, strings, and inline comments.
    let mut in_single = false;
    let mut in_double = false;
    let mut token = String::new();
    let mut is_first_token = true;

    let flush_token = |tok: &mut String, spans: &mut Vec<Span<'static>>, first: &mut bool| {
        if tok.is_empty() {
            return;
        }
        let s = std::mem::take(tok);
        let style = if s.starts_with('"') || s.starts_with('\'') {
            theme::success()
        } else if s.starts_with('$') {
            theme::warning()
        } else if s.starts_with('-') && s.len() > 1 {
            theme::purple()
        } else if *first
            || matches!(
                s.as_str(),
                "if" | "then"
                    | "else"
                    | "elif"
                    | "fi"
                    | "for"
                    | "in"
                    | "do"
                    | "done"
                    | "while"
                    | "case"
                    | "esac"
                    | "export"
                    | "local"
                    | "set"
                    | "return"
                    | "exit"
                    | "mise"
                    | "cargo"
            )
        {
            theme::accent()
        } else {
            theme::primary()
        };
        if !s.trim().is_empty() {
            *first = matches!(s.as_str(), "&&" | "||" | "|" | ";" | "then" | "do");
        }
        spans.push(Span::styled(s, style));
    };

    let chars: Vec<char> = trimmed.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        if !in_single && !in_double && ch == '#' && (i == 0 || chars[i - 1].is_whitespace()) {
            flush_token(&mut token, &mut spans, &mut is_first_token);
            let rest: String = chars[i..].iter().collect();
            spans.push(Span::styled(rest, theme::muted()));
            break;
        }
        if ch == '\'' && !in_double {
            in_single = !in_single;
            token.push(ch);
        } else if ch == '"' && !in_single {
            in_double = !in_double;
            token.push(ch);
        } else if !in_single && !in_double && ch.is_whitespace() {
            flush_token(&mut token, &mut spans, &mut is_first_token);
            spans.push(Span::raw(ch.to_string()));
        } else {
            token.push(ch);
        }
        i += 1;
    }
    flush_token(&mut token, &mut spans, &mut is_first_token);
    Line::from(spans)
}

/// Render a streamed output line, respecting `NO_COLOR` by stripping ANSI SGR color codes when disabled.
pub fn render_ansi_line_respecting_no_color(
    text: &str,
    base: Style,
    no_color: bool,
) -> Line<'static> {
    if no_color {
        Line::from(Span::styled(strip_ansi(text), base))
    } else {
        parse_ansi_line(text, base)
    }
}

/// Render a streamed job output line using the active terminal's `NO_COLOR` capability.
pub fn render_job_output_line(text: &str, base: Style) -> Line<'static> {
    render_ansi_line_respecting_no_color(text, base, theme::active_palette().is_no_color())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use ratatui::style::Color;

    use super::*;

    #[test]
    fn respects_no_color_when_rendering_ansi_output() {
        let input = "\x1b[1;32mOK\x1b[0m plain";
        let colored = render_ansi_line_respecting_no_color(input, Style::default(), false);
        assert_eq!(colored.spans[0].style.fg, Some(Color::Green));

        let plain = render_ansi_line_respecting_no_color(input, Style::default(), true);
        assert_eq!(plain.spans.len(), 1);
        assert_eq!(plain.spans[0].content, "OK plain");
        assert_eq!(plain.spans[0].style.fg, None);
        assert_eq!(plain.spans[0].style.bg, None);
    }

    #[test]
    fn truncates_cjk_and_emoji_by_display_width() {
        assert_eq!(truncate_display_width("hello", 10), "hello");
        assert_eq!(truncate_display_width("你好世界测试", 7), "你好世…");
        assert_eq!(UnicodeWidthStr::width(truncate_display_width("你好世界测试", 7).as_str()), 7);
        assert_eq!(truncate_display_width("🚀部署完成", 5), "🚀部…");
        assert_eq!(truncate_display_width("abcdef", 1), "…");
        assert_eq!(truncate_display_width("abcdef", 0), "");
    }

    #[test]
    fn shortens_paths_preserving_filename_and_width() {
        let long = "/Users/dev/workspace/projects/deep/nested/mise.toml";
        let short = shorten_path(long, 26);
        assert!(UnicodeWidthStr::width(short.as_str()) <= 26);
        assert!(short.ends_with("mise.toml"));
    }

    #[test]
    fn highlights_fuzzy_spans_and_truncates_safely() {
        let spans = highlight_fuzzy_spans("build-test", &[0, 6], theme::primary());
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "build-test");
        assert_eq!(spans.len(), 4);

        let trunc = highlighted_truncated_spans("项目:构建测试任务", &[3, 4], theme::primary(), 11);
        let trunc_str: String = trunc.iter().map(|s| s.content.as_ref()).collect();
        assert!(UnicodeWidthStr::width(trunc_str.as_str()) <= 11);
        assert!(trunc_str.ends_with('…'));
    }

    #[test]
    fn formats_durations_cleanly() {
        assert_eq!(format_duration_ms(42), "42ms");
        assert_eq!(format_duration_ms(370), "370ms");
        assert_eq!(format_duration_ms(1_250), "1.25s");
        assert_eq!(format_duration_ms(14_500), "14.5s");
        assert_eq!(format_duration_ms(134_000), "2m 14s");
    }
}
