//! Copy text to the desktop clipboard, including through a remote terminal.

use base64::Engine as _;
use std::io::{self, IsTerminal as _, Write as _};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn terminal_clipboard(env: impl Fn(&str) -> Option<String>) -> bool {
    let set = |name: &str| env(name).is_some_and(|value| !value.is_empty());
    set("SSH_CONNECTION")
        || set("SSH_TTY")
        || (cfg!(target_os = "linux") && !set("DISPLAY") && !set("WAYLAND_DISPLAY"))
}

fn write_osc52(mut out: impl io::Write, text: &str) -> io::Result<()> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    write!(out, "\x1b]52;c;{encoded}\x1b\\")?;
    out.flush()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalClipboard {
    Tmux,
    Kitty,
    Osc52,
}

fn terminal_backend(env: impl Fn(&str) -> Option<String>) -> TerminalClipboard {
    let set = |name: &str| env(name).is_some_and(|value| !value.is_empty());
    if set("TMUX") {
        TerminalClipboard::Tmux
    } else if set("KITTY_WINDOW_ID") || env("TERM").is_some_and(|value| value.contains("kitty")) {
        TerminalClipboard::Kitty
    } else {
        TerminalClipboard::Osc52
    }
}

fn copy_with_command(program: &str, args: &[&str], text: &str) -> io::Result<()> {
    copy_with_command_timeout(program, args, text, Duration::from_secs(10))
}

fn copy_with_command_timeout(
    program: &str,
    args: &[&str],
    text: &str,
    timeout: Duration,
) -> io::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let payload = text.as_bytes().to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&payload));
    let started = Instant::now();
    while child.try_wait()?.is_none() {
        if started.elapsed() >= timeout {
            child.kill()?;
            let _ = child.wait();
            let _ = writer.join();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "{program} clipboard did not respond within {} ms",
                    timeout.as_millis()
                ),
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let result = child.wait_with_output()?;
    if !result.status.success() {
        let message = String::from_utf8_lossy(&result.stderr);
        return Err(io::Error::other(format!(
            "{program} clipboard failed: {}",
            message.trim()
        )));
    }
    writer
        .join()
        .map_err(|_| io::Error::other("clipboard writer panicked"))??;
    Ok(())
}

/// Returns the status to show the user. OSC 52 has no success reply: the
/// terminal may reject clipboard writes according to its own settings.
pub fn copy_text(text: &str) -> Result<&'static str, String> {
    if terminal_clipboard(|name| std::env::var(name).ok()) {
        if !io::stdout().is_terminal() {
            return Err("no terminal is available for clipboard copy".into());
        }
        let backend = terminal_backend(|name| std::env::var(name).ok());
        tracing::info!(
            ?backend,
            bytes = text.len(),
            "copying text through terminal"
        );
        match backend {
            TerminalClipboard::Tmux => {
                copy_with_command("tmux", &["load-buffer", "-w", "-"], text)
                    .map_err(|e| e.to_string())?;
                Ok("copied to tmux buffer; terminal clipboard requested")
            }
            TerminalClipboard::Kitty => {
                match copy_with_command("kitten", &["clipboard", "--wait-for-completion"], text) {
                    Ok(()) => Ok("copied to terminal clipboard"),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        write_osc52(io::stdout().lock(), text).map_err(|e| e.to_string())?;
                        Ok("sent to terminal clipboard via OSC 52")
                    }
                    Err(error) => Err(error.to_string()),
                }
            }
            TerminalClipboard::Osc52 => {
                write_osc52(io::stdout().lock(), text).map_err(|e| e.to_string())?;
                Ok("sent to terminal clipboard via OSC 52")
            }
        }
    } else {
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(text))
            .map_err(|e| e.to_string())?;
        Ok("copied to clipboard")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_uses_the_terminal_even_with_a_forwarded_display() {
        let env = |name: &str| match name {
            "SSH_CONNECTION" => Some("client 1234 server 22".into()),
            "DISPLAY" => Some("localhost:10".into()),
            _ => None,
        };
        assert!(terminal_clipboard(env));
    }

    #[test]
    fn osc52_encodes_control_characters_as_data() {
        let mut output = Vec::new();
        write_osc52(&mut output, "/tmp/a\nfile").unwrap();
        assert_eq!(output, b"\x1b]52;c;L3RtcC9hCmZpbGU=\x1b\\");
    }

    #[test]
    fn kitty_and_tmux_use_their_clipboard_helpers() {
        let kitty = |name: &str| (name == "TERM").then(|| "xterm-kitty".into());
        assert_eq!(terminal_backend(kitty), TerminalClipboard::Kitty);
        let tmux = |name: &str| match name {
            "TERM" => Some("xterm-kitty".into()),
            "TMUX" => Some("/tmp/tmux/socket".into()),
            _ => None,
        };
        assert_eq!(terminal_backend(tmux), TerminalClipboard::Tmux);
        assert_eq!(terminal_backend(|_| None), TerminalClipboard::Osc52);
    }

    #[test]
    fn helper_receives_multiline_text_and_reports_rejection() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("clipboard.txt");
        // Execute the installed shell; fresh executable fixtures can race
        // with concurrent process creation and fail with ETXTBSY.
        copy_with_command(
            "sh",
            &[
                "-c",
                "cat > \"$1\"",
                "clipboard-helper",
                target.to_str().unwrap(),
            ],
            "one\ntwo\n",
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(target).unwrap(), "one\ntwo\n");

        let error = copy_with_command(
            "sh",
            &["-c", "cat >/dev/null; echo rejected >&2; exit 3"],
            "report",
        )
        .unwrap_err();
        assert!(error.to_string().contains("rejected"), "{error}");

        let error = copy_with_command_timeout(
            "sh",
            &["-c", "sleep 2"],
            "report",
            Duration::from_millis(50),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }
}
