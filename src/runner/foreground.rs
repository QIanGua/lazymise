use std::io::{self, Write};
use std::path::Path;
use std::process::Command;
use std::time::Instant;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

use anyhow::{Context, Result};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

/// RAII guard that suspends raw mode, alternate screen, and mouse capture while active,
/// and ALWAYS restores them on drop (even if spawning the child process fails or panics).
pub struct TerminalGuard<'a> {
    terminal: &'a mut Terminal<CrosstermBackend<io::Stdout>>,
    mouse_enabled: bool,
}

impl<'a> TerminalGuard<'a> {
    pub fn suspend(
        terminal: &'a mut Terminal<CrosstermBackend<io::Stdout>>,
        mouse_enabled: bool,
    ) -> Result<Self> {
        disable_raw_mode()?;
        let guard = Self {
            terminal,
            mouse_enabled,
        };
        if guard.mouse_enabled {
            let _ = execute!(io::stdout(), DisableMouseCapture);
        }
        // Perform terminal leave steps AFTER guard exists so any error triggers Drop restoration.
        execute!(io::stdout(), LeaveAlternateScreen)?;
        let _ = execute!(io::stdout(), crossterm::cursor::Show);
        Ok(guard)
    }
}

impl Drop for TerminalGuard<'_> {
    fn drop(&mut self) {
        let _ = enable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), EnterAlternateScreen);
        if self.mouse_enabled {
            let _ = execute!(self.terminal.backend_mut(), EnableMouseCapture);
        }
        let _ = self.terminal.hide_cursor();
        let _ = self.terminal.clear();
    }
}

/// Parse a POSIX-like command string (such as `$EDITOR="code --wait"` or task arguments)
/// into program + argument tokens, handling single quotes, double quotes, and backslash escapes.
pub fn split_shell_words(input: &str) -> Result<Vec<String>> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut has_token = false;

    for ch in input.chars() {
        if escaped {
            if in_double {
                match ch {
                    '"' | '\\' | '$' | '`' => current.push(ch),
                    other => {
                        current.push('\\');
                        current.push(other);
                    }
                }
            } else {
                current.push(ch);
            }
            escaped = false;
            has_token = true;
            continue;
        }
        if in_single {
            if ch == '\'' {
                in_single = false;
            } else {
                current.push(ch);
            }
            continue;
        }
        if in_double {
            match ch {
                '"' => in_double = false,
                '\\' => escaped = true,
                _ => current.push(ch),
            }
            continue;
        }
        match ch {
            '\\' => {
                escaped = true;
                has_token = true;
            }
            '\'' => {
                in_single = true;
                has_token = true;
            }
            '"' => {
                in_double = true;
                has_token = true;
            }
            c if c.is_whitespace() => {
                if has_token {
                    words.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            c => {
                current.push(c);
                has_token = true;
            }
        }
    }

    if in_single || in_double {
        anyhow::bail!("unterminated quote in command: `{input}`");
    }
    if escaped {
        current.push('\\');
    }
    if has_token {
        words.push(current);
    }
    Ok(words)
}

/// Check whether an executable exists on `PATH` (or as an explicit executable file path).
/// Useful for pre-checking optional tools like `watchexec` before `mise watch`.
pub fn command_exists(cmd: &str) -> bool {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return false;
    }

    let candidate = Path::new(trimmed);
    if trimmed.contains('/') || trimmed.contains(std::path::MAIN_SEPARATOR) {
        return is_executable_file(candidate);
    }

    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };

    for dir in std::env::split_paths(&path_var) {
        let full = dir.join(trimmed);
        if is_executable_file(&full) {
            return true;
        }
        #[cfg(windows)]
        if let Some(exts) = std::env::var_os("PATHEXT") {
            for ext in exts.to_string_lossy().split(';') {
                let ext = ext.trim();
                if !ext.is_empty() && is_executable_file(&dir.join(format!("{trimmed}{ext}"))) {
                    return true;
                }
            }
        }
    }

    false
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Format the exit status banner shown after a foreground command completes.
pub fn format_exit_banner(code: i32, duration_ms: u128) -> String {
    let secs = duration_ms as f64 / 1000.0;
    if code == 0 {
        format!("[ok: exit 0 in {secs:.2}s] Press Enter to return to lazymise...")
    } else {
        format!("[failed: exit {code} in {secs:.2}s] Press Enter to return to lazymise...")
    }
}

/// Suspend TUI safely via `TerminalGuard`, execute foreground command with full TTY access,
/// pause on request or non-zero exit, and restore TUI unconditionally.
pub fn run_foreground(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    workdir: &Path,
    program: &str,
    args: &[String],
    wait_for_enter: bool,
    mouse_enabled: bool,
) -> Result<(i32, u128)> {
    let _guard = TerminalGuard::suspend(terminal, mouse_enabled)?;

    let display_args: Vec<String> = args
        .iter()
        .map(|a| {
            if a.is_empty() || a.chars().any(char::is_whitespace) {
                format!("{a:?}")
            } else {
                a.clone()
            }
        })
        .collect();

    if display_args.is_empty() {
        println!("$ {program}");
    } else {
        println!("$ {} {}", program, display_args.join(" "));
    }
    let _ = io::stdout().flush();

    let start = Instant::now();
    let status = Command::new(program)
        .args(args)
        .current_dir(workdir)
        .status()
        .with_context(|| format!("failed to run `{program}`"))?;
    let duration_ms = start.elapsed().as_millis();
    let code = status.code().unwrap_or_else(|| {
        #[cfg(unix)]
        {
            status.signal().map(|s| 128 + s).unwrap_or(1)
        }
        #[cfg(not(unix))]
        {
            1
        }
    });

    if wait_for_enter || code != 0 {
        println!("\n{}", format_exit_banner(code, duration_ms));
        let _ = io::stdout().flush();
        let mut buf = String::new();
        let _ = io::stdin().read_line(&mut buf);
    }

    Ok((code, duration_ms))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn splits_editor_with_flags_and_quotes() {
        assert_eq!(
            split_shell_words("code --wait").unwrap(),
            vec!["code", "--wait"]
        );
        assert_eq!(
            split_shell_words(r#"nvim -u "NONE" --cmd 'set number'"#).unwrap(),
            vec!["nvim", "-u", "NONE", "--cmd", "set number"]
        );
        assert_eq!(
            split_shell_words(r#""/Applications/VS Code/bin/code" --wait"#).unwrap(),
            vec!["/Applications/VS Code/bin/code", "--wait"]
        );
    }

    #[test]
    fn handles_escapes_empty_quotes_and_unicode() {
        assert_eq!(
            split_shell_words(r#"echo hello\ world "a\"b\\c" '' "中文 参数""#).unwrap(),
            vec!["echo", "hello world", r#"a"b\c"#, "", "中文 参数"]
        );
        // Non-special backslash inside double quotes stays intact (POSIX)
        assert_eq!(
            split_shell_words(r#""foo\nbar""#).unwrap(),
            vec![r"foo\nbar"]
        );
    }

    #[test]
    fn rejects_unterminated_quotes() {
        assert!(split_shell_words("code \"--wait").is_err());
        assert!(split_shell_words("nvim 'unclosed").is_err());
    }

    #[test]
    fn checks_command_existence() {
        assert!(command_exists("sh"));
        assert!(!command_exists(""));
        assert!(!command_exists("definitely_nonexistent_binary_lazymise_xyz"));
    }

    #[test]
    fn formats_exit_banners() {
        assert_eq!(
            format_exit_banner(0, 1250),
            "[ok: exit 0 in 1.25s] Press Enter to return to lazymise..."
        );
        assert_eq!(
            format_exit_banner(2, 500),
            "[failed: exit 2 in 0.50s] Press Enter to return to lazymise..."
        );
    }
}
