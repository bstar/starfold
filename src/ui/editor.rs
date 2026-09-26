//! A terminal editor contained in the Preview panel.
//!
//! The editor has its own PTY and terminal screen. Its reader runs on a
//! thread, so neither a quiet editor nor a noisy one can block the UI loop.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use crossbeam_channel::{bounded, Receiver};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use starkit::chrome::{frame, header};
use starkit::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use starkit::ratatui::buffer::Buffer;
use starkit::ratatui::layout::Rect;
use starkit::ratatui::style::{Color, Modifier, Style};

use crate::fold::entry::{Entry, EntryKind};
use crate::ui::panels::rgb;
use crate::ui::theme::Theme;

enum Output {
    Bytes(Vec<u8>),
    Error(String),
}

pub struct Editor {
    pub path: PathBuf,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    output: Receiver<Output>,
    parser: vt100::Parser,
    size: (u16, u16),
}

/// The menu uses cached entry facts and filename MIME hints. It never reads
/// an arbitrary file on the drawing thread. Extensionless files remain
/// available because README, Makefile, dotfiles, and new files are common.
pub fn is_editable(entry: &Entry) -> bool {
    if !matches!(entry.kind, EntryKind::File)
        && !(entry.kind == EntryKind::Symlink && entry.link_kind == Some(EntryKind::File))
    {
        return false;
    }
    if entry.path.extension().is_none() {
        return true;
    }
    let mime = mime_guess::from_path(&entry.path)
        .first_raw()
        .unwrap_or_default();
    mime.starts_with("text/")
        || matches!(
            mime,
            "application/json"
                | "application/ld+json"
                | "application/xml"
                | "application/javascript"
                | "application/x-yaml"
                | "application/x-toml"
                | "application/toml"
                | "application/x-sh"
                | "application/x-shellscript"
        )
}

/// `$VISUAL` takes precedence over `$EDITOR`; a missing setting uses `vi`.
/// Shell quoting is parsed into arguments, never executed through a shell.
pub fn editor_argv(
    visual: Option<&str>,
    editor: Option<&str>,
    path: &Path,
) -> Result<Vec<OsString>> {
    let setting = visual
        .filter(|value| !value.trim().is_empty())
        .or_else(|| editor.filter(|value| !value.trim().is_empty()))
        .unwrap_or("vi");
    let words = shell_words::split(setting).context("parsing $VISUAL or $EDITOR")?;
    if words.is_empty() {
        return Err(anyhow!("editor command is empty"));
    }
    let mut argv: Vec<OsString> = words.into_iter().map(OsString::from).collect();
    argv.push(path.as_os_str().to_os_string());
    Ok(argv)
}

impl Editor {
    pub fn start(path: PathBuf, size: (u16, u16)) -> Result<Self> {
        let visual = std::env::var("VISUAL").ok();
        let editor = std::env::var("EDITOR").ok();
        let argv = editor_argv(visual.as_deref(), editor.as_deref(), &path)?;
        Self::spawn(path, size, argv)
    }

    fn spawn(path: PathBuf, size: (u16, u16), argv: Vec<OsString>) -> Result<Self> {
        let size = (size.0.max(1), size.1.max(1));
        let pty_size = PtySize {
            cols: size.0,
            rows: size.1,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = native_pty_system()
            .openpty(pty_size)
            .context("opening editor terminal")?;
        let reader = pair
            .master
            .try_clone_reader()
            .context("reading editor terminal")?;
        let writer = pair
            .master
            .take_writer()
            .context("writing editor terminal")?;
        let mut command = CommandBuilder::from_argv(argv);
        command.env("TERM", "xterm-256color");
        if let Some(dir) = path.parent() {
            command.cwd(dir);
        }
        let child = pair
            .slave
            .spawn_command(command)
            .context("starting terminal editor")?;
        drop(pair.slave);

        let (sender, output) = bounded(128);
        std::thread::Builder::new()
            .name("starfold-editor-reader".into())
            .spawn(move || read_output(reader, sender))
            .context("starting editor reader")?;

        Ok(Self {
            path,
            master: pair.master,
            child,
            writer,
            output,
            parser: vt100::Parser::new(size.1, size.0, 0),
            size,
        })
    }

    pub fn poll(&mut self) -> Result<Option<portable_pty::ExitStatus>> {
        for _ in 0..128 {
            match self.output.try_recv() {
                Ok(Output::Bytes(bytes)) => self.parser.process(&bytes),
                Ok(Output::Error(error)) => return Err(anyhow!(error)),
                Err(_) => break,
            }
        }
        self.child.try_wait().context("checking terminal editor")
    }

    pub fn key(&mut self, key: KeyEvent) -> Result<()> {
        let bytes = key_bytes(key, self.parser.screen().application_cursor());
        if !bytes.is_empty() {
            self.writer.write_all(&bytes).context("typing in editor")?;
            self.writer.flush().context("flushing editor input")?;
        }
        Ok(())
    }

    pub fn paste(&mut self, text: &str) -> Result<()> {
        if self.parser.screen().bracketed_paste() {
            self.writer.write_all(b"\x1b[200~")?;
        }
        self.writer.write_all(text.as_bytes())?;
        if self.parser.screen().bracketed_paste() {
            self.writer.write_all(b"\x1b[201~")?;
        }
        self.writer.flush().context("pasting in editor")
    }

    fn resize(&mut self, size: (u16, u16)) -> Result<()> {
        let size = (size.0.max(1), size.1.max(1));
        if self.size == size {
            return Ok(());
        }
        self.master.resize(PtySize {
            cols: size.0,
            rows: size.1,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        self.parser.screen_mut().set_size(size.1, size.0);
        self.size = size;
        Ok(())
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer, theme: &Theme) -> Result<()> {
        let detail = self.path.file_name().unwrap_or_default().to_string_lossy();
        let body = frame::frame(
            area,
            buf,
            &frame::Frame {
                theme,
                focused: true,
                title: "edit",
                detail: Some(&detail),
                heading: false,
                badge: None,
                footer: None,
                words: &[] as &[crate::ui::panels::Word],
            },
        );
        let body = content_rect(body);
        if body.width == 0 || body.height == 0 {
            return Ok(());
        }
        self.resize((body.width, body.height))?;
        let screen = self.parser.screen();
        for row in 0..body.height {
            for col in 0..body.width {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let fg = terminal_color(cell.fgcolor(), rgb(theme.row_fg));
                let bg = terminal_color(cell.bgcolor(), rgb(theme.panel_bg));
                let mut style = Style::default().fg(fg).bg(bg);
                if cell.bold() {
                    style = style.add_modifier(Modifier::BOLD);
                }
                if cell.dim() {
                    style = style.add_modifier(Modifier::DIM);
                }
                if cell.italic() {
                    style = style.add_modifier(Modifier::ITALIC);
                }
                if cell.underline() {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
                if cell.inverse() {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                let drawn = &mut buf[(body.x + col, body.y + row)];
                drawn.set_symbol(if cell.contents().is_empty() {
                    " "
                } else {
                    cell.contents()
                });
                drawn.set_style(style);
            }
        }
        if !screen.hide_cursor() {
            let (row, col) = screen.cursor_position();
            if row < body.height && col < body.width {
                let cell = &mut buf[(body.x + col, body.y + row)];
                cell.set_style(cell.style().add_modifier(Modifier::REVERSED));
            }
        }
        Ok(())
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub fn preview_content_rect(area: Rect) -> Rect {
    content_rect(header::body(area))
}

fn content_rect(body: Rect) -> Rect {
    Rect {
        x: body.x.saturating_add(u16::from(body.width > 0)),
        width: body.width.saturating_sub(2),
        ..body
    }
}

fn terminal_color(color: vt100::Color, default: Color) -> Color {
    match color {
        vt100::Color::Default => default,
        vt100::Color::Idx(index) => Color::Indexed(index),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

fn read_output(mut reader: Box<dyn Read + Send>, sender: crossbeam_channel::Sender<Output>) {
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if sender.send(Output::Bytes(buf[..n].to_vec())).is_err() {
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            #[cfg(unix)]
            Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
            Err(error) => {
                let _ = sender.send(Output::Error(error.to_string()));
                break;
            }
        }
    }
}

fn key_bytes(key: KeyEvent, application_cursor: bool) -> Vec<u8> {
    let mut out = match key.code {
        KeyCode::Char(ch) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let lower = ch.to_ascii_lowercase();
            match lower {
                'a'..='z' => vec![lower as u8 - b'a' + 1],
                ' ' | '@' => vec![0],
                '[' => vec![27],
                '\\' => vec![28],
                ']' => vec![29],
                '^' => vec![30],
                '_' => vec![31],
                _ => Vec::new(),
            }
        }
        KeyCode::Char(ch) => ch.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Esc => vec![27],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![127],
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Up => cursor_key(b'A', application_cursor),
        KeyCode::Down => cursor_key(b'B', application_cursor),
        KeyCode::Right => cursor_key(b'C', application_cursor),
        KeyCode::Left => cursor_key(b'D', application_cursor),
        KeyCode::F(1) => b"\x1bOP".to_vec(),
        KeyCode::F(2) => b"\x1bOQ".to_vec(),
        KeyCode::F(3) => b"\x1bOR".to_vec(),
        KeyCode::F(4) => b"\x1bOS".to_vec(),
        KeyCode::F(5) => b"\x1b[15~".to_vec(),
        KeyCode::F(6) => b"\x1b[17~".to_vec(),
        KeyCode::F(7) => b"\x1b[18~".to_vec(),
        KeyCode::F(8) => b"\x1b[19~".to_vec(),
        KeyCode::F(9) => b"\x1b[20~".to_vec(),
        KeyCode::F(10) => b"\x1b[21~".to_vec(),
        KeyCode::F(11) => b"\x1b[23~".to_vec(),
        KeyCode::F(12) => b"\x1b[24~".to_vec(),
        _ => Vec::new(),
    };
    if key.modifiers.contains(KeyModifiers::ALT) && !out.is_empty() {
        out.insert(0, 27);
    }
    out
}

fn cursor_key(letter: u8, application_cursor: bool) -> Vec<u8> {
    vec![27, if application_cursor { b'O' } else { b'[' }, letter]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_command_precedence_and_quoting_keep_path_as_one_arg() {
        let path = Path::new("/tmp/-draft with spaces.txt");
        assert_eq!(
            editor_argv(Some("nvim --cmd 'set number'"), Some("nano"), path).unwrap(),
            vec!["nvim", "--cmd", "set number", "/tmp/-draft with spaces.txt"]
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>()
        );
        assert_eq!(editor_argv(None, None, path).unwrap()[0], "vi");
    }

    #[test]
    fn editable_regular_text_and_symlinks_but_not_directories_or_known_binary() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), b"hello").unwrap();
        std::fs::write(dir.path().join("blob.bin"), b"\0").unwrap();
        std::fs::write(dir.path().join("Makefile"), b"all:\n").unwrap();
        assert!(is_editable(&crate::fold::entry::stat(
            &dir.path().join("note.txt")
        )));
        assert!(is_editable(&crate::fold::entry::stat(
            &dir.path().join("Makefile")
        )));
        assert!(!is_editable(&crate::fold::entry::stat(
            &dir.path().join("blob.bin")
        )));
        assert!(!is_editable(&crate::fold::entry::stat(dir.path())));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("note.txt", dir.path().join("link.txt")).unwrap();
            assert!(is_editable(&crate::fold::entry::stat(
                &dir.path().join("link.txt")
            )));
        }
    }

    #[test]
    fn common_editor_keys_are_encoded_for_a_pty() {
        let key = |code, mods| KeyEvent::new(code, mods);
        assert_eq!(
            key_bytes(key(KeyCode::Char('x'), KeyModifiers::NONE), false),
            b"x"
        );
        assert_eq!(
            key_bytes(key(KeyCode::Char('c'), KeyModifiers::CONTROL), false),
            b"\x03"
        );
        assert_eq!(
            key_bytes(key(KeyCode::Up, KeyModifiers::NONE), true),
            b"\x1bOA"
        );
        assert_eq!(
            key_bytes(key(KeyCode::Up, KeyModifiers::NONE), false),
            b"\x1b[A"
        );
        assert_eq!(
            key_bytes(key(KeyCode::Enter, KeyModifiers::NONE), false),
            b"\r"
        );
    }

    #[cfg(unix)]
    #[test]
    fn embedded_pty_renders_output_accepts_input_and_edits_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file with spaces.txt");
        std::fs::write(&path, b"").unwrap();
        let script = "printf '\\033[31mREADY\\033[0m\\r\\n'; IFS= read -r line; printf '%s' \"$line\" > \"$1\"";
        let argv = vec![
            OsString::from("sh"),
            OsString::from("-c"),
            OsString::from(script),
            OsString::from("sh"),
            path.as_os_str().to_os_string(),
        ];
        let mut editor = Editor::spawn(path.clone(), (40, 8), argv).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !editor.parser.screen().contents().contains("READY") {
            assert!(
                std::time::Instant::now() < deadline,
                "editor did not render output"
            );
            editor.poll().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let area = Rect::new(0, 0, 40, 12);
        let mut frame = Buffer::empty(area);
        editor
            .render(
                area,
                &mut frame,
                &crate::ui::theme::tests_support::theme("terminal"),
            )
            .unwrap();
        assert!(
            (0..area.height).any(|y| { (0..area.width).any(|x| frame[(x, y)].symbol() == "R") }),
            "editor output should be visible inside Preview"
        );
        editor
            .key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
            .unwrap();
        editor
            .key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        loop {
            assert!(std::time::Instant::now() < deadline, "editor did not exit");
            if editor.poll().unwrap().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"x");
    }
}
