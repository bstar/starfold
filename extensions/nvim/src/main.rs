//! Embedded Neovim editor: RPC/UI events in, bounded native surfaces out.
mod grid;
use anyhow::{ensure, Context, Result};
use crossbeam_channel::{bounded, Receiver};
use rmpv::Value;
use starfold_preview_protocol::{Input, Limits, Presentation, Request, TextPage, Viewport};
use std::{
    io::{BufReader, Read, Write},
    os::unix::{ffi::OsStringExt, fs::PermissionsExt},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    thread::JoinHandle,
    time::{Duration, Instant},
};
struct Session {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Result<Value>>,
    reader: Option<JoinHandle<()>>,
    sequence: u64,
    grid: grid::Grid,
    viewport: Viewport,
    closed: bool,
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
impl Session {
    fn open(executable: &str, path: Vec<u8>, limits: Limits, viewport: Viewport) -> Result<Self> {
        let path = Self::validate_path(path, limits)?;
        let filename = path.to_str().unwrap();
        let recovery = recovery_directory()?;
        let mut child = Command::new(executable)
            .args(["--embed", "--cmd", "set nomodeline noexrc"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("Could not start Neovim; install nvim or supply its executable path")?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, messages) = bounded(128);
        let reader = std::thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            loop {
                let result = rmpv::decode::read_value(&mut stdout).map_err(anyhow::Error::from);
                let failed = result.is_err();
                if tx.send_timeout(result, Duration::from_secs(1)).is_err() || failed {
                    break;
                }
            }
        });
        let mut s = Self {
            child,
            stdin,
            messages,
            reader: Some(reader),
            sequence: 0,
            grid: Default::default(),
            viewport,
            closed: false,
        };
        let (columns, rows) = s.dimensions();
        s.call(
            "nvim_ui_attach",
            vec![
                columns.into(),
                rows.into(),
                Value::Map(vec![
                    ("rgb".into(), true.into()),
                    ("ext_linegrid".into(), true.into()),
                ]),
            ],
        )?;
        let api_info = s.call("nvim_get_api_info", vec![])?;
        let channel = api_info
            .as_array()
            .and_then(|a| a.first())
            .and_then(Value::as_u64)
            .context("No Neovim RPC channel")?;
        s.call(
            "nvim_exec_lua",
            vec![
                include_str!("session.lua").into(),
                Value::Array(vec![
                    filename.into(),
                    recovery.to_string_lossy().to_string().into(),
                    channel.into(),
                ]),
            ],
        )?;
        s.drain()?;
        Ok(s)
    }
    fn validate_path(path: Vec<u8>, limits: Limits) -> Result<PathBuf> {
        let path = PathBuf::from(std::ffi::OsString::from_vec(path));
        let meta = std::fs::metadata(&path)?;
        ensure!(meta.is_file(), "Neovim requires a regular text file");
        let cap = limits.text_bytes.clamp(1, 512 * 1024);
        ensure!(
            meta.len() <= cap as u64,
            "File exceeds editable preview budget; open it externally"
        );
        let mut bytes = vec![];
        std::fs::File::open(&path)?
            .take((cap + 1) as u64)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= cap,
            "File grew beyond editable preview budget"
        );
        ensure!(
            !bytes.contains(&0) && std::str::from_utf8(&bytes).is_ok(),
            "Neovim embedding requires UTF-8 text"
        );
        ensure!(
            path.to_str().is_some(),
            "Neovim embedding requires a UTF-8 pathname"
        );
        Ok(path)
    }
    fn reopen(&mut self, path: Vec<u8>, limits: Limits, viewport: Viewport) -> Result<()> {
        let path = Self::validate_path(path, limits)?;
        self.call(
            "nvim_exec_lua",
            vec![
                include_str!("switch.lua").into(),
                Value::Array(vec![path.to_str().unwrap().into()]),
            ],
        )?;
        self.input(Input::Viewport { viewport })?;
        Ok(())
    }
    fn dimensions(&self) -> (u64, u64) {
        if let Some([cols, rows]) = self.viewport.cells {
            let cols = u32::from(cols).clamp(1, 256);
            return (
                u64::from(cols),
                u64::from(u32::from(rows).clamp(1, 113).min(8192 / cols)),
            );
        }
        let cols = (self.viewport.width.clamp(1, 2048) / 8).clamp(1, 256);
        (
            u64::from(cols),
            u64::from(
                (self.viewport.height.clamp(1, 2048) / 18)
                    .clamp(1, 113)
                    .min(8192 / cols),
            ),
        )
    }

    fn notify(&mut self, method: &str, args: Vec<Value>) -> Result<()> {
        rmpv::encode::write_value(
            &mut self.stdin,
            &Value::Array(vec![2.into(), method.into(), Value::Array(args)]),
        )?;
        self.stdin.flush()?;
        Ok(())
    }
    fn call(&mut self, method: &str, args: Vec<Value>) -> Result<Value> {
        self.sequence += 1;
        let id = self.sequence;
        rmpv::encode::write_value(
            &mut self.stdin,
            &Value::Array(vec![0.into(), id.into(), method.into(), Value::Array(args)]),
        )?;
        self.stdin.flush()?;
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let message = self
                .messages
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .with_context(|| format!("Neovim RPC {method} timed out"))??;
            if let Some(a) = message.as_array() {
                if a.first().and_then(Value::as_u64) == Some(1)
                    && a.get(1).and_then(Value::as_u64) == Some(id)
                {
                    ensure!(a.len() == 4, "Invalid Neovim RPC response");
                    ensure!(a[2].is_nil(), "Neovim: {}", a[2]);
                    return Ok(a[3].clone());
                }
            }
            self.grid.notification(&message)?;
        }
    }
    fn drain(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_millis(2);
        while Instant::now() < deadline {
            match self
                .messages
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(Ok(v)) => self.grid.notification(&v)?,
                Ok(Err(error)) => {
                    if self.child.try_wait()?.is_some_and(|s| s.success()) {
                        self.closed = true;
                        return Ok(());
                    }
                    return Err(error);
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => break,
                Err(_) => {
                    self.closed = true;
                    break;
                }
            }
        }
        Ok(())
    }
    fn send_keys(&mut self, notation: &str) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut remaining = notation;
        while !remaining.is_empty() {
            let end = remaining
                .char_indices()
                .nth(256)
                .map(|(i, _)| i)
                .unwrap_or(remaining.len());
            let chunk = &remaining[..end];
            let accepted = self
                .call("nvim_input", vec![chunk.into()])?
                .as_u64()
                .context("Invalid input acknowledgement")? as usize;
            ensure!(
                accepted <= chunk.len() && chunk.is_char_boundary(accepted),
                "Invalid Neovim input acknowledgement"
            );
            remaining = &remaining[accepted..];
            if accepted == 0 {
                self.drain()?;
                ensure!(Instant::now() < deadline, "Neovim input buffer is full");
            }
        }
        Ok(())
    }
    fn input(&mut self, input: Input) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        match input {
            Input::Action { action } if action == "editor-save" || action == "editor-discard" => {
                self.send_keys("<C-\\><C-N><Esc>")?;
                self.drain()?;
                let script = if action == "editor-save" {
                    "local ok,err=pcall(vim.cmd,'wall'); local modified=false; for _,b in ipairs(vim.api.nvim_list_bufs()) do if vim.api.nvim_buf_is_valid(b) and vim.bo[b].modified then modified=true end end; vim.rpcnotify(...,'starfold_editor',modified); if not ok then vim.api.nvim_echo({{tostring(err),'ErrorMsg'}},true,{}) end"
                } else {
                    "for _,b in ipairs(vim.api.nvim_list_bufs()) do if vim.api.nvim_buf_is_valid(b) and vim.bo[b].modified then vim.api.nvim_buf_delete(b,{force=true}) end end; vim.rpcnotify(...,'starfold_editor',false)"
                };
                let api = self.call("nvim_get_api_info", vec![])?;
                let channel = api
                    .as_array()
                    .and_then(|a| a.first())
                    .context("No RPC channel")?
                    .clone();
                // Failed writes leave the live editor and its modified buffers intact.
                let _ = self.call(
                    "nvim_exec_lua",
                    vec![script.into(), Value::Array(vec![channel])],
                );
            }
            Input::Key { key } => {
                let notation = key_notation(&key);
                if let Err(e) = self.send_keys(&notation) {
                    if self.child.try_wait()?.is_some_and(|s| s.success()) {
                        self.closed = true;
                        return Ok(());
                    }
                    return Err(e);
                }
            }
            Input::Paste { text } => {
                ensure!(text.len() <= 65536, "Paste too large");
                if self.grid.blocking {
                    self.send_keys(&text.replace('<', "<lt>").replace('\n', "<CR>"))?;
                } else {
                    // Run paste on the main loop, after queued mode-changing keys.
                    self.call(
                        "nvim_exec_lua",
                        vec![
                            "return vim.api.nvim_paste(..., false, -1)".into(),
                            Value::Array(vec![text.into()]),
                        ],
                    )?;
                }
            }
            Input::Viewport { viewport } => {
                self.viewport = viewport;
                let (cols, rows) = self.dimensions();
                self.notify("nvim_ui_try_resize", vec![cols.into(), rows.into()])?;
            }
            Input::Pointer {
                action,
                x,
                y,
                button,
            } => {
                let col = u64::from(if self.viewport.cells.is_some() {
                    x
                } else {
                    x / 8
                })
                .min(self.grid.columns.saturating_sub(1) as u64);
                let row = u64::from(if self.viewport.cells.is_some() {
                    y
                } else {
                    y / 18
                })
                .min(self.grid.rows.saturating_sub(1) as u64);
                let (button, action) = match action.as_str() {
                    "scroll_up" => ("wheel", "up"),
                    "scroll_down" => ("wheel", "down"),
                    "down" => (if button == 0 { "left" } else { "right" }, "press"),
                    "drag" => (if button == 0 { "left" } else { "right" }, "drag"),
                    "up" => (if button == 0 { "left" } else { "right" }, "release"),
                    _ => return Ok(()),
                };
                self.call(
                    "nvim_input_mouse",
                    vec![
                        button.into(),
                        action.into(),
                        "".into(),
                        1.into(),
                        row.into(),
                        col.into(),
                    ],
                )?;
            }
            _ => {}
        }
        if !self.closed {
            // This RPC also drains redraw notifications preceding its response.
            // One short tail drain handles main-loop redraws without imposing
            // two idle waits on every mouse sample.
            let mode = self.call("nvim_get_mode", vec![])?;
            self.grid.blocking = grid::map_get(&mode, "blocking")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            self.drain()?;
        }
        Ok(())
    }
    fn closed_cleanly(&mut self) -> Result<bool> {
        let deadline = Instant::now() + Duration::from_millis(80);
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status.success());
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn presentation(&self) -> Result<Presentation> {
        if self.closed {
            return Ok(Presentation {
                kind: "Neovim closed".into(),
                notice: Some("Editor closed; Preview follows the browser again".into()),
                ..Default::default()
            });
        }
        let surface = self.grid.surface(&self.viewport)?;
        Ok(Presentation {
            kind: "Neovim · read/write".into(),
            interactive: true,
            modified: self.grid.modified,
            fields: vec![(
                "Editor".into(),
                format!(
                    "{}{} · F6 browser · :w save · :q close",
                    self.grid.mode,
                    if self.grid.modified {
                        " · modified"
                    } else {
                        ""
                    }
                ),
            )],
            pages: vec![TextPage {
                number: 1,
                text: self.grid.text(),
                truncated: false,
            }],
            surface: Some(surface),
            cells: Some(self.grid.cell_grid(&self.viewport)?),
            keys: vec!["*".into()],
            ..Default::default()
        })
    }
}
fn key_notation(key: &str) -> String {
    let mut rest = key;
    let mut modifiers = String::new();
    loop {
        let modifier = [
            ("ctrl+", "C-"),
            ("alt+", "M-"),
            ("shift+", "S-"),
            ("super+", "D-"),
        ]
        .iter()
        .find(|(prefix, _)| rest.starts_with(prefix));
        let Some((prefix, notation)) = modifier else {
            break;
        };
        modifiers.push_str(notation);
        rest = &rest[prefix.len()..];
    }
    if !modifiers.is_empty() {
        let base = match rest {
            "enter" => "CR",
            "escape" => "Esc",
            "backspace" => "BS",
            "space" => "Space",
            other => other.trim_start_matches('<').trim_end_matches('>'),
        };
        return format!("<{modifiers}{base}>");
    }
    match key {
        "space" => " ".into(),
        "<" => "<lt>".into(),
        "enter" => "<CR>".into(),
        "escape" => "<Esc>".into(),
        "backspace" => "<BS>".into(),
        "tab" => "<Tab>".into(),
        "backtab" => "<S-Tab>".into(),
        "left" | "right" | "up" | "down" | "home" | "end" | "delete" | "pageup" | "pagedown" => {
            format!("<{key}>")
        }
        key if key.starts_with("ctrl+") => format!("<C-{}>", &key[5..]),
        key if key.starts_with("alt+") => format!("<M-{}>", &key[4..]),
        other => other.into(),
    }
}
fn recovery_directory() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .context("No state directory for Neovim recovery")?;
    let path = base.join("starfold/nvim-recovery");
    std::fs::create_dir_all(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    Ok(path)
}
fn main() -> Result<()> {
    let executable = std::env::args().nth(1).unwrap_or_else(|| "nvim".into());
    let mut session: Option<Session> = None;
    starfold_preview_protocol::serve(
        "nvim",
        env!("CARGO_PKG_VERSION"),
        &["documents", "surfaces", "input", "editor", "reopen"],
        |request| {
            match request {
                Request::Open {
                    path,
                    limits,
                    viewport,
                } => {
                    if let Some(editor) = session.as_mut().filter(|s| !s.closed) {
                        editor.reopen(path, limits, viewport)?;
                    } else {
                        session = Some(Session::open(&executable, path, limits, viewport)?);
                    }
                }
                Request::Input { input } => {
                    let editor = session.as_mut().context("Open first")?;
                    if let Err(error) = editor.input(input) {
                        if editor.closed_cleanly()? {
                            editor.closed = true;
                        } else {
                            return Err(error);
                        }
                    }
                }
                _ => anyhow::bail!("Unsupported request"),
            }
            Ok((session.as_ref().unwrap().presentation()?, vec![]))
        },
    )
}
