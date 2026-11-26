use std::io::{self, Write};
use std::process::{Command, Stdio};

use anyhow::Result;

/// Backend that fulfilled a clipboard copy operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardMethod {
    Pbcopy,
    WlCopy,
    Xclip,
    Xsel,
    Osc52,
}

impl ClipboardMethod {
    pub fn label(self) -> &'static str {
        match self {
            ClipboardMethod::Pbcopy => "pbcopy",
            ClipboardMethod::WlCopy => "wl-copy",
            ClipboardMethod::Xclip => "xclip",
            ClipboardMethod::Xsel => "xsel",
            ClipboardMethod::Osc52 => "OSC 52",
        }
    }
}

/// Copy text to system clipboard using native backends with fault-tolerant fallback
/// (`pbcopy` -> `wl-copy` -> `xclip` -> `xsel`) and terminal OSC 52 escape sequence fallback.
pub fn copy_to_clipboard(text: &str) -> Result<()> {
    copy_to_clipboard_detailed(text).map(|_| ())
}

/// Same as `copy_to_clipboard`, returning which clipboard mechanism succeeded (for Toast display).
pub fn copy_to_clipboard_detailed(text: &str) -> Result<ClipboardMethod> {
    let candidates: &[(&str, &[&str], ClipboardMethod)] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[], ClipboardMethod::Pbcopy)]
    } else {
        &[
            ("wl-copy", &[], ClipboardMethod::WlCopy),
            ("xclip", &["-selection", "clipboard"], ClipboardMethod::Xclip),
            ("xsel", &["--clipboard", "--input"], ClipboardMethod::Xsel),
        ]
    };

    for &(prog, args, method) in candidates {
        if try_pipe_clipboard(prog, args, text) {
            return Ok(method);
        }
    }

    write_osc52(text)?;
    Ok(ClipboardMethod::Osc52)
}

fn try_pipe_clipboard(prog: &str, args: &[&str], text: &str) -> bool {
    let Ok(mut child) = Command::new(prog)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };

    if let Some(mut stdin) = child.stdin.take()
        && stdin.write_all(text.as_bytes()).is_err()
    {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    }

    child.wait().is_ok_and(|s| s.success())
}

/// Encode an OSC 52 clipboard escape sequence (with optional tmux DCS passthrough wrapper).
pub fn encode_osc52(text: &str, in_tmux: bool) -> String {
    let encoded = base64_encode(text.as_bytes());
    if in_tmux {
        format!("\x1bPtmux;\x1b\x1b]52;c;{encoded}\x07\x1b\\")
    } else {
        format!("\x1b]52;c;{encoded}\x07")
    }
}

fn write_osc52(text: &str) -> Result<()> {
    let in_tmux = std::env::var_os("TMUX").is_some();
    let seq = encode_osc52(text, in_tmux);

    #[cfg(unix)]
    if let Ok(mut tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty")
        && tty.write_all(seq.as_bytes()).is_ok()
        && tty.flush().is_ok()
    {
        return Ok(());
    }

    let mut stdout = io::stdout();
    if stdout.write_all(seq.as_bytes()).is_ok() && stdout.flush().is_ok() {
        return Ok(());
    }

    let mut stderr = io::stderr();
    stderr.write_all(seq.as_bytes())?;
    stderr.flush()?;
    Ok(())
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);

    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);

        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn encodes_rfc4648_base64() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode("mise 中文".as_bytes()), "bWlzZSDkuK3mloc=");
    }

    #[test]
    fn encodes_osc52_plain_and_tmux() {
        assert_eq!(
            encode_osc52("mise run check", false),
            "\x1b]52;c;bWlzZSBydW4gY2hlY2s=\x07"
        );
        assert_eq!(
            encode_osc52("mise run check", true),
            "\x1bPtmux;\x1b\x1b]52;c;bWlzZSBydW4gY2hlY2s=\x07\x1b\\"
        );
    }

    #[test]
    fn failed_backend_returns_false_instead_of_bailing() {
        assert!(!try_pipe_clipboard(
            "nonexistent_clipboard_tool_lazymise_xyz",
            &[],
            "hello"
        ));
    }
}
