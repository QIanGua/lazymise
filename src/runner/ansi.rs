use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Zero-dependency ANSI SGR + `\r` parser targeting `ratatui::text::Line`.
///
/// Supports:
/// - SGR reset (`0` / empty), bold (`1`), dim (`2`), italic (`3`), underline (`4`),
///   blink (`5`/`6`), reversed (`7`), hidden (`8`), crossed-out (`9`), and off codes (`21..=29`)
/// - Standard 16 colors (`30..=37`, `40..=47`, `90..=97`, `100..=107`, `39`, `49`)
/// - 256-color indexed (`38;5;n`, `48;5;n`, and colon form `38:5:n`)
/// - 24-bit TrueColor (`38;2;r;g;b`, `48;2;r;g;b`, and colon form `38:2::r:g:b`)
/// - Silent stripping of non-SGR CSI, OSC (e.g. hyperlinks/titles), DCS/APC/PM, and truncated sequences
/// - `\r` carriage-return line overwrites (progress bars) without erasing on trailing `\r`
pub fn parse_ansi_line(input: &str, base_style: Style) -> Line<'static> {
    let mut style = base_style;
    parse_line_with_state(input, base_style, &mut style)
}

/// Parse multi-line ANSI text into `Vec<Line<'static>>`, preserving active SGR styles across newlines.
pub fn parse_ansi_text(input: &str, base_style: Style) -> Vec<Line<'static>> {
    let mut style = base_style;
    let mut lines = Vec::new();
    for raw_line in input.split('\n') {
        let trimmed = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        lines.push(parse_line_with_state(trimmed, base_style, &mut style));
    }
    lines
}

/// Strip ANSI CSI/OSC/ESC sequences and resolve `\r` overwrites for plain-text search/copy.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut cr_pending = false;
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            0x1b => {
                i = skip_escape_sequence(bytes, i);
            }
            b'\r' => {
                cr_pending = true;
                i += 1;
            }
            b'\n' => {
                cr_pending = false;
                out.push('\n');
                i += 1;
            }
            b if b < 0x20 && b != b'\t' => {
                i += 1;
            }
            0x7f => {
                i += 1;
            }
            _ => {
                if cr_pending {
                    if let Some(last_nl) = out.rfind('\n') {
                        out.truncate(last_nl + 1);
                    } else {
                        out.clear();
                    }
                    cr_pending = false;
                }
                let ch = input[i..].chars().next().unwrap_or('\u{FFFD}');
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }

    out
}

fn parse_line_with_state(
    input: &str,
    base_style: Style,
    current_style: &mut Style,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut text_buf = String::new();
    let mut cr_pending = false;

    let bytes = input.as_bytes();
    let mut i = 0;

    let flush = |spans: &mut Vec<Span<'static>>, text_buf: &mut String, style: Style| {
        if !text_buf.is_empty() {
            spans.push(Span::styled(std::mem::take(text_buf), style));
        }
    };

    while i < bytes.len() {
        match bytes[i] {
            0x1b => {
                flush(&mut spans, &mut text_buf, *current_style);
                let (next_i, sgr_slice) = parse_escape_sequence(bytes, i);
                if let Some(sgr) = sgr_slice {
                    apply_sgr(sgr, base_style, current_style);
                }
                i = next_i;
            }
            b'\r' => {
                flush(&mut spans, &mut text_buf, *current_style);
                cr_pending = true;
                i += 1;
            }
            b'\t' => {
                if cr_pending {
                    spans.clear();
                    text_buf.clear();
                    cr_pending = false;
                }
                text_buf.push_str("    ");
                i += 1;
            }
            b if b < 0x20 => {
                // Skip stray control bytes
                i += 1;
            }
            0x7f => {
                i += 1;
            }
            _ => {
                if cr_pending {
                    spans.clear();
                    text_buf.clear();
                    cr_pending = false;
                }
                let ch = input[i..].chars().next().unwrap_or('\u{FFFD}');
                text_buf.push(ch);
                i += ch.len_utf8();
            }
        }
    }

    flush(&mut spans, &mut text_buf, *current_style);

    if spans.is_empty() {
        Line::from(Span::styled(String::new(), base_style))
    } else {
        Line::from(spans)
    }
}

/// Skip an escape sequence starting at `bytes[start] == 0x1b`, returning the index after it.
fn skip_escape_sequence(bytes: &[u8], start: usize) -> usize {
    parse_escape_sequence(bytes, start).0
}

/// Parse an escape sequence starting at `bytes[start] == 0x1b`.
/// Returns `(next_index, Some(sgr_params))` if it was a valid SGR (`CSI ... m`) sequence,
/// or `(next_index, None)` for any other CSI/OSC/DCS/ESC or truncated sequence.
fn parse_escape_sequence(bytes: &[u8], start: usize) -> (usize, Option<&[u8]>) {
    let len = bytes.len();
    let i = start + 1;
    if i >= len {
        return (len, None);
    }

    match bytes[i] {
        // CSI: ESC [ <params 0x30..=0x3F>* <intermediates 0x20..=0x2F>* <final 0x40..=0x7E>
        b'[' => {
            let param_start = i + 1;
            let mut j = param_start;
            while j < len && (0x30..=0x3f).contains(&bytes[j]) {
                j += 1;
            }
            let param_end = j;
            while j < len && (0x20..=0x2f).contains(&bytes[j]) {
                j += 1;
            }
            if j >= len {
                // Truncated CSI sequence: swallow cleanly
                return (len, None);
            }
            let final_byte = bytes[j];
            let next = j + 1;
            if final_byte == b'm' && param_end == j {
                let params = &bytes[param_start..param_end];
                // Ignore private-mode CSI sequences (e.g. `?`, `>`, `<`, `=`)
                let is_private = params
                    .first()
                    .is_some_and(|b| matches!(b, b'?' | b'>' | b'<' | b'='));
                if !is_private {
                    return (next, Some(params));
                }
            }
            (next, None)
        }
        // OSC: ESC ] ... (BEL | ESC \)
        b']' => {
            let mut j = i + 1;
            while j < len {
                if bytes[j] == 0x07 {
                    return (j + 1, None);
                }
                if bytes[j] == 0x1b && j + 1 < len && bytes[j + 1] == b'\\' {
                    return (j + 2, None);
                }
                j += 1;
            }
            (len, None)
        }
        // DCS / SOS / PM / APC: ESC (P|X|^|_) ... ESC \
        b'P' | b'X' | b'^' | b'_' => {
            let mut j = i + 1;
            while j < len {
                if bytes[j] == 0x1b && j + 1 < len && bytes[j + 1] == b'\\' {
                    return (j + 2, None);
                }
                j += 1;
            }
            (len, None)
        }
        // Charset designation: ESC ( B, ESC ) 0, etc.
        b'(' | b')' | b'*' | b'+' | b'-' | b'.' | b'/' => ((i + 2).min(len), None),
        // Other 2-byte ESC sequence
        _ => (i + 1, None),
    }
}

fn apply_sgr(params_bytes: &[u8], base_style: Style, style: &mut Style) {
    let Ok(params_str) = std::str::from_utf8(params_bytes) else {
        return;
    };
    if params_str.is_empty() {
        *style = base_style;
        return;
    }

    let tokens: Vec<&str> = params_str.split(';').collect();
    let mut idx = 0;

    while idx < tokens.len() {
        let token = tokens[idx];

        // Support ITU colon-separated extended color subparameters like `38:5:208` or `38:2::r:g:b`
        if token.contains(':') {
            let sub: Vec<u16> = token
                .split(':')
                .map(|s| s.parse::<u16>().unwrap_or(0))
                .collect();
            if let Some(&first) = sub.first()
                && (first == 38 || first == 48)
                && let Some(color) = parse_colon_color(&sub[1..])
            {
                if first == 38 {
                    *style = style.fg(color);
                } else {
                    *style = style.bg(color);
                }
            }
            idx += 1;
            continue;
        }

        let code = if token.is_empty() {
            0
        } else if let Ok(v) = token.parse::<u16>() {
            v
        } else {
            idx += 1;
            continue;
        };

        match code {
            0 => *style = base_style,
            1 => *style = style.add_modifier(Modifier::BOLD),
            2 => *style = style.add_modifier(Modifier::DIM),
            3 => *style = style.add_modifier(Modifier::ITALIC),
            4 => *style = style.add_modifier(Modifier::UNDERLINED),
            5 | 6 => *style = style.add_modifier(Modifier::SLOW_BLINK),
            7 => *style = style.add_modifier(Modifier::REVERSED),
            8 => *style = style.add_modifier(Modifier::HIDDEN),
            9 => *style = style.add_modifier(Modifier::CROSSED_OUT),
            21 | 22 => clear_modifier(style, base_style, Modifier::BOLD | Modifier::DIM),
            23 => clear_modifier(style, base_style, Modifier::ITALIC),
            24 => clear_modifier(style, base_style, Modifier::UNDERLINED),
            25 => clear_modifier(
                style,
                base_style,
                Modifier::SLOW_BLINK | Modifier::RAPID_BLINK,
            ),
            27 => clear_modifier(style, base_style, Modifier::REVERSED),
            28 => clear_modifier(style, base_style, Modifier::HIDDEN),
            29 => clear_modifier(style, base_style, Modifier::CROSSED_OUT),
            30..=37 => *style = style.fg(ansi_standard_color((code - 30) as u8)),
            38 => {
                if let Some((color, consumed)) = parse_semicolon_color(&tokens[idx + 1..]) {
                    *style = style.fg(color);
                    idx += consumed;
                }
            }
            39 => style.fg = base_style.fg,
            40..=47 => *style = style.bg(ansi_standard_color((code - 40) as u8)),
            48 => {
                if let Some((color, consumed)) = parse_semicolon_color(&tokens[idx + 1..]) {
                    *style = style.bg(color);
                    idx += consumed;
                }
            }
            49 => style.bg = base_style.bg,
            90..=97 => *style = style.fg(ansi_bright_color((code - 90) as u8)),
            100..=107 => *style = style.bg(ansi_bright_color((code - 100) as u8)),
            _ => {}
        }

        idx += 1;
    }
}

fn clear_modifier(style: &mut Style, base_style: Style, m: Modifier) {
    style.add_modifier.remove(m);
    let from_base = base_style.add_modifier & m;
    if !from_base.is_empty() {
        style.sub_modifier.insert(from_base);
    }
}

fn parse_semicolon_color(rest: &[&str]) -> Option<(Color, usize)> {
    let mode = rest.first()?.parse::<u16>().ok()?;
    match mode {
        5 => {
            let n = rest.get(1)?.parse::<u8>().ok()?;
            Some((Color::Indexed(n), 2))
        }
        2 => {
            let r = rest.get(1)?.parse::<u8>().ok()?;
            let g = rest.get(2)?.parse::<u8>().ok()?;
            let b = rest.get(3)?.parse::<u8>().ok()?;
            Some((Color::Rgb(r, g, b), 4))
        }
        _ => None,
    }
}

fn parse_colon_color(rest: &[u16]) -> Option<Color> {
    let &mode = rest.first()?;
    match mode {
        5 => {
            let n = u8::try_from(*rest.get(1)?).ok()?;
            Some(Color::Indexed(n))
        }
        2 => {
            // Supports both `38:2:r:g:b` (len 4) and ITU `38:2:cs:r:g:b` (len >= 5)
            let (r, g, b) = if rest.len() >= 5 {
                (rest[2], rest[3], rest[4])
            } else if rest.len() == 4 {
                (rest[1], rest[2], rest[3])
            } else {
                return None;
            };
            Some(Color::Rgb(
                u8::try_from(r).ok()?,
                u8::try_from(g).ok()?,
                u8::try_from(b).ok()?,
            ))
        }
        _ => None,
    }
}

fn ansi_standard_color(idx: u8) -> Color {
    match idx {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        _ => Color::Gray,
    }
}

fn ansi_bright_color(idx: u8) -> Color {
    match idx {
        0 => Color::DarkGray,
        1 => Color::LightRed,
        2 => Color::LightGreen,
        3 => Color::LightYellow,
        4 => Color::LightBlue,
        5 => Color::LightMagenta,
        6 => Color::LightCyan,
        _ => Color::White,
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use ratatui::style::{Color, Modifier, Style};

    use super::*;

    #[test]
    fn parses_16_color_and_modifiers_and_reset() {
        let base = Style::default().fg(Color::White);
        let line = parse_ansi_line(
            "\x1b[1;32mOK\x1b[0m plain \x1b[3;4;91merr\x1b[23;39m end",
            base,
        );
        assert_eq!(line.spans.len(), 4);
        assert_eq!(line.spans[0].content, "OK");
        assert_eq!(
            line.spans[0].style,
            base.fg(Color::Green).add_modifier(Modifier::BOLD)
        );
        assert_eq!(line.spans[1].content, " plain ");
        assert_eq!(line.spans[1].style, base);
        assert_eq!(line.spans[2].content, "err");
        assert_eq!(
            line.spans[2].style,
            base.fg(Color::LightRed)
                .add_modifier(Modifier::ITALIC | Modifier::UNDERLINED)
        );
        assert_eq!(line.spans[3].content, " end");
        assert_eq!(
            line.spans[3].style,
            base.add_modifier(Modifier::UNDERLINED)
        );
    }

    #[test]
    fn parses_256_and_truecolor_fg_and_bg() {
        let base = Style::default();
        let line = parse_ansi_line(
            "\x1b[38;5;208;48;2;31;36;48morange\x1b[m \x1b[38:2::92:207:230mcyan\x1b[0m",
            base,
        );
        assert_eq!(line.spans.len(), 3);
        assert_eq!(line.spans[0].content, "orange");
        assert_eq!(
            line.spans[0].style,
            Style::default()
                .fg(Color::Indexed(208))
                .bg(Color::Rgb(31, 36, 48))
        );
        assert_eq!(line.spans[1].content, " ");
        assert_eq!(line.spans[1].style, base);
        assert_eq!(line.spans[2].content, "cyan");
        assert_eq!(
            line.spans[2].style,
            Style::default().fg(Color::Rgb(92, 207, 230))
        );
    }

    #[test]
    fn strips_non_sgr_csi_osc_and_truncated_sequences() {
        let input = "\x1b[2K\x1b[?25l\x1b]8;;https://mise.jdx.dev\x07mise\x1b]8;;\x07 ok\x1b[38;2;10";
        assert_eq!(strip_ansi(input), "mise ok");
        let line = parse_ansi_line(input, Style::default());
        let joined: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "mise ok");
    }

    #[test]
    fn handles_carriage_return_overwrite_and_trailing_cr() {
        let line = parse_ansi_line(
            "\x1b[33m10%\r\x1b[32m100% done\x1b[0m\r",
            Style::default(),
        );
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content, "100% done");
        assert_eq!(line.spans[0].style.fg, Some(Color::Green));
        assert_eq!(strip_ansi("a\rb\rc"), "c");
        assert_eq!(strip_ansi("keep\r"), "keep");
    }

    #[test]
    fn multiline_parse_preserves_style_across_newlines() {
        let lines = parse_ansi_text("\x1b[36mline1\nline2\x1b[0m\nline3", Style::default());
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].spans[0].content, "line1");
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Cyan));
        assert_eq!(lines[1].spans[0].content, "line2");
        assert_eq!(lines[1].spans[0].style.fg, Some(Color::Cyan));
        assert_eq!(lines[2].spans[0].content, "line3");
        assert_eq!(lines[2].spans[0].style.fg, None);
    }
}
