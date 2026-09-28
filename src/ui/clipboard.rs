//! Copy text to the desktop clipboard, including through a remote terminal.

use base64::Engine as _;
use std::io::{self, IsTerminal as _, Write as _};

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

/// Returns the status to show the user. OSC 52 has no success reply: the
/// terminal may reject clipboard writes according to its own settings.
pub fn copy_text(text: &str) -> Result<&'static str, String> {
    if terminal_clipboard(|name| std::env::var(name).ok()) {
        if !io::stdout().is_terminal() {
            return Err("no terminal is available for clipboard copy".into());
        }
        write_osc52(io::stdout().lock(), text).map_err(|e| e.to_string())?;
        Ok("current path sent to terminal clipboard")
    } else {
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(text))
            .map_err(|e| e.to_string())?;
        Ok("current path copied")
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
}
