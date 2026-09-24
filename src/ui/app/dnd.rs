//! Native OSC 72 gestures and their bridge to the file-operation queue.

use std::path::PathBuf;

use starkit::ratatui::layout::Rect;

use super::super::dnd::{self as wire, Active, Choice, Message, Offer};
use super::*;

fn final_drop_operation(status: Option<(OpStatus, usize)>, intended: OpKind) -> Option<i32> {
    match status {
        Some((OpStatus::Done, skipped)) => Some(if intended == OpKind::Move && skipped == 0 {
            2
        } else {
            1
        }),
        Some((OpStatus::Failed | OpStatus::Cancelled, _)) | None => Some(0),
        _ => None,
    }
}

impl App {
    pub(super) fn dnd_query(&self) {
        // DA is the ordering marker prescribed by OSC 72. Crossterm swallows
        // its reply; if no OSC 72 reply arrives, the feature stays disabled.
        tracing::info!("querying terminal for OSC 72 drag and drop");
        let _ = wire::send("t=q:i=1", None);
        let mut out = std::io::stdout();
        let _ = std::io::Write::write_all(&mut out, b"\x1b[c");
        let _ = std::io::Write::flush(&mut out);
    }

    pub(super) fn dnd_stop(&mut self) {
        if !self.dnd.enabled {
            return;
        }
        if self.dnd.choice.is_some() {
            self.dnd_cancel_choice();
        }
        if let Some(active) = &self.dnd.active {
            self.core.send(Command::Cancel(active.op));
            let _ = wire::send("t=r:o=0", None);
        }
        if let Some(id) = self.dnd.export_op.take() {
            self.core.send(Command::Cancel(id));
            self.dnd.export_tx = None;
            self.dnd.export_output = None;
            if let Some(rx) = self.dnd.export_done.take() {
                let _ = rx.recv_timeout(std::time::Duration::from_secs(1));
            }
            self.core.send(Command::FinishExport {
                op: id,
                success: false,
            });
        }
        let _ = wire::send("t=A", None);
        let _ = wire::send("t=o:x=2", None);
    }

    pub(super) fn dnd_message(&mut self, raw: &str) {
        let Some(m) = Message::parse(raw) else { return };
        // Kitty sends the initial source gesture before it knows the drag's
        // client ID; it learns that ID from our subsequent MIME offer. All
        // other replies belong to one of the subscriptions and must match.
        let initial_offer = m.get("t") == Some("o") && m.get("i").is_none();
        if m.number("i") != Some(1) && !initial_offer {
            return;
        }
        match m.get("t") {
            Some("q") => {
                if self.dnd.enabled {
                    return;
                }
                self.dnd.enabled = true;
                let id = wire::machine_id();
                tracing::info!(
                    remote_identity = id.is_some(),
                    "OSC 72 drag and drop enabled"
                );
                // Enabling drops and advertising the machine ID are distinct
                // OSC 72 requests. x=1 only updates the machine ID; without
                // the plain t=a request Kitty still pastes dropped paths.
                let _ = wire::send("t=a:i=1", Some("text/uri-list"));
                let _ = wire::send("t=a:x=1:i=1", id.as_deref());
                let _ = wire::send("t=o:x=1:i=1", id.as_deref());
            }
            Some("o") if self.dnd.enabled => self.dnd_offer(&m),
            Some("m") if self.dnd.enabled => self.dnd_hover(&m),
            Some("M") if self.dnd.enabled => self.dnd_drop(&m),
            Some("r") | None
                if self.dnd.enabled && (self.dnd.receiving_uri || self.dnd.remote.is_some()) =>
            {
                self.dnd_received(&m)
            }
            Some("e") if self.dnd.enabled => self.dnd_offered_request(&m),
            Some("k") if self.dnd.enabled => self.dnd_export_request(&m),
            Some("E") if m.payload == "OK" => {}
            Some("E") if self.dnd.export_op.is_some() => {
                self.dnd.export_cancelled = true;
                self.dnd.export_tx = None;
                self.dnd.export_output = None;
                if let Some(id) = self.dnd.export_op {
                    self.core.send(Command::Cancel(id));
                }
            }
            Some("E") | Some("R") => self.dnd_error(),
            _ => {}
        }
    }

    fn dnd_offer(&mut self, m: &Message<'_>) {
        let (Some(x), Some(y)) = (m.number("x"), m.number("y")) else {
            return;
        };
        let Some(sources) = self.dnd_sources(x, y) else {
            return;
        };
        let uri_text: String = sources
            .iter()
            .filter_map(|p| wire::file_uri(p).map(|uri| (p, uri)))
            .map(|(path, mut uri)| {
                if std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir()) {
                    uri.push('/');
                }
                format!("{uri}\r\n")
            })
            .collect();
        if uri_text.is_empty() {
            return;
        }
        self.dnd.offer = Some(Offer {
            sources,
            uri_text: uri_text.clone(),
        });
        // SSH exports are copy-only. In-window Move remains possible through
        // the post-drop choice because STAR/FOLD itself owns both paths.
        let _ = wire::send("t=o:o=1:i=1", Some("text/uri-list"));
        let _ = wire::send_data("t=p:x=0:i=1", uri_text.as_bytes());
        let _ = wire::send("t=P:x=-1:i=1", None);
    }

    fn dnd_hover(&mut self, m: &Message<'_>) {
        let (Some(x), Some(y)) = (m.number("x"), m.number("y")) else {
            return;
        };
        let allowed = m.number("o").unwrap_or(0);
        if x >= 0 && y >= 0 && !m.payload.is_empty() {
            self.dnd.offered_uri = m.payload.split_whitespace().any(|s| s == "text/uri-list");
        }
        let mime_ok = self.dnd.offered_uri;
        let target = (mime_ok && allowed != 0 && x >= 0 && y >= 0)
            .then(|| self.dnd_target(x, y))
            .flatten();
        tracing::debug!(
            x,
            y,
            allowed,
            mime_ok,
            accepts = target.is_some(),
            "OSC 72 drag hover received"
        );
        let coords = target.as_ref().map(|_| (x as u16, y as u16));
        let changed = self.dnd.hover != target || self.dnd.hover_coords != coords;
        self.dnd.hover = target;
        self.dnd.hover_coords = coords;
        if self.dnd.hover.is_some() {
            let action = if allowed == 2 { 2 } else { 1 };
            let _ = wire::send(&format!("t=m:o={action}:i=1"), Some("text/uri-list"));
        } else {
            let _ = wire::send("t=m:o=0:i=1", None);
        }
        // Kitty reports pointer motion at pixel resolution. Repainting a full
        // Commander view for every event can lag behind the gesture and leave
        // a source drag canceled before the drop reaches us.
        if changed {
            self.repaint = true;
        }
    }

    fn dnd_drop(&mut self, m: &Message<'_>) {
        self.dnd.hover = None;
        self.dnd.hover_coords = None;
        self.dnd.offered_uri = false;
        let (Some(x), Some(y)) = (m.number("x"), m.number("y")) else {
            let _ = wire::send("t=r:o=0:i=1", None);
            return;
        };
        tracing::info!(x, y, allowed = m.number("o"), "OSC 72 drop received");
        let Some(dest) = self.dnd_target(x, y) else {
            let _ = wire::send("t=r:o=0:i=1", None);
            return;
        };
        let mime_index = m
            .payload
            .split_whitespace()
            .position(|mime| mime == "text/uri-list")
            .map(|i| i as i32 + 1);
        if mime_index.is_none() {
            let _ = wire::send("t=r:o=0:i=1", None);
            return;
        }
        let own_sources = self.dnd.offer.as_ref().map(|offer| offer.sources.clone());
        let allowed = m.number("o").unwrap_or(1);
        if allowed == 0 {
            let _ = wire::send("t=r:o=0:i=1", None);
            return;
        }
        self.dnd.choice = Some(Choice {
            dest,
            own_sources,
            allowed,
            mime_index,
            remote: false,
        });
        if allowed == 3
            || self
                .dnd
                .choice
                .as_ref()
                .is_some_and(|c| c.own_sources.is_some())
        {
            self.overlays.open_drop((x.max(0) as u16, y.max(0) as u16));
            self.repaint = true;
        } else {
            self.dnd_choose(if allowed == 2 {
                OpKind::Move
            } else {
                OpKind::Copy
            });
        }
    }

    pub(super) fn dnd_choose(&mut self, kind: OpKind) {
        let Some(choice) = self.dnd.choice.as_ref() else {
            return;
        };
        if choice.own_sources.is_none()
            && ((kind == OpKind::Move && choice.allowed & 2 == 0)
                || (kind == OpKind::Copy && choice.allowed & 1 == 0))
        {
            self.dnd_cancel_choice();
            return;
        }
        if let Some(sources) = choice.own_sources.clone() {
            self.dnd_queue(sources, kind);
            // Our offer is copy-only to prevent an external target from
            // deleting a remote source. An in-window Move is performed by
            // our own queue, so the terminal still completes a copy offer.
            if let Some(active) = &mut self.dnd.active {
                active.result_operation = OpKind::Copy;
            }
        } else if let Some(index) = choice.mime_index {
            self.dnd.receiving_uri = true;
            self.dnd.received.clear();
            let _ = wire::send(&format!("t=r:x={index}:i=1"), None);
            // Remember the choice until the URI list arrives.
            self.dnd_requested_kind(kind);
        } else {
            self.dnd_cancel_choice();
        }
    }

    fn dnd_requested_kind(&mut self, kind: OpKind) {
        self.dnd.result_kind = Some(kind);
    }

    fn dnd_received(&mut self, m: &Message<'_>) {
        if let Some(remote) = self.dnd.remote.as_mut() {
            match remote.receive(m) {
                Ok(true) => {
                    let remote = self.dnd.remote.take().unwrap();
                    let roots = remote.roots.clone();
                    self.dnd.staged = Some(remote.stage);
                    if let Some(op) = self.dnd.import_op.take() {
                        self.core.send(Command::FinishImport { op, success: true });
                    }
                    let result = self.dnd.result_kind.unwrap_or(OpKind::Copy);
                    self.dnd_queue(roots, OpKind::Move);
                    if let Some(active) = &mut self.dnd.active {
                        active.result_operation = result;
                    }
                }
                Ok(false) => {}
                Err(err) => self.dnd_error_with(&err),
            }
            return;
        }
        if !self.dnd.receiving_uri {
            return;
        }
        if m.number("X") == Some(1) {
            if let Some(choice) = &mut self.dnd.choice {
                choice.remote = true;
            }
        }
        let Some(data) = m.data() else {
            self.dnd_error();
            return;
        };
        if self.dnd.received.len().saturating_add(data.len()) > 8 * 1024 * 1024 {
            self.dnd_error();
            return;
        }
        self.dnd.received.extend(data);
        if m.end_of_data() {
            self.dnd.receiving_uri = false;
            let Some(paths) = wire::uri_list(&self.dnd.received) else {
                self.dnd_error();
                return;
            };
            let kind = self.dnd.result_kind.unwrap_or(OpKind::Copy);
            if self.dnd.choice.as_ref().is_some_and(|choice| choice.remote) {
                tracing::info!(files = paths.len(), "receiving remote drop files");
                let choice = self.dnd.choice.as_ref().unwrap();
                let Some(index) = choice.mime_index else {
                    self.dnd_error();
                    return;
                };
                self.core.send(Command::BeginImport {
                    sources: paths.clone(),
                    dest: choice.dest.clone(),
                });
                let latest = {
                    let state = self.core.state();
                    state
                        .queue
                        .iter()
                        .last()
                        .map(|op| (op.id, Arc::clone(&op.progress)))
                };
                let Some((op_id, progress)) = latest else {
                    self.dnd_error();
                    return;
                };
                self.dnd.import_op = Some(op_id);
                match wire::Remote::new(&choice.dest, &paths, index, progress) {
                    Ok(mut remote) => match remote.request_next() {
                        Ok(false) => self.dnd.remote = Some(remote),
                        Ok(true) => self.dnd_error_with("remote drop had no files"),
                        Err(err) => self.dnd_error_with(&err),
                    },
                    Err(err) => self.dnd_error_with(&err),
                }
            } else {
                // An external source owns removal on successful Move completion.
                self.dnd_queue(paths, OpKind::Copy);
                if let Some(active) = &mut self.dnd.active {
                    active.result_operation = kind;
                }
            }
        }
    }

    fn dnd_offered_request(&mut self, m: &Message<'_>) {
        if m.number("x") == Some(5) && m.number("y") == Some(0) {
            if let Some(offer) = &self.dnd.offer {
                let _ = wire::send_data("t=e:y=0:i=1", offer.uri_text.as_bytes());
            }
        }
        if m.number("x") == Some(4) {
            self.dnd.export_drag_finished = true;
            self.dnd.export_cancelled = m.number("y") == Some(1);
            self.dnd.export_tx = None;
            if self.dnd.export_cancelled {
                self.dnd.export_output = None;
                if let Some(id) = self.dnd.export_op {
                    self.core.send(Command::Cancel(id));
                }
            }
            if self.dnd.export_op.is_none() {
                self.dnd.offer = None;
            }
        }
    }

    fn dnd_export_request(&mut self, m: &Message<'_>) {
        let Some(index) = m.number("x").and_then(|x| usize::try_from(x).ok()) else {
            return;
        };
        let Some(offer) = &self.dnd.offer else { return };
        if self.dnd.export_op.is_none() {
            self.core.send(Command::BeginExport(offer.sources.clone()));
            let state = self.core.state();
            let Some(op) = state.queue.iter().last() else {
                return;
            };
            let id = op.id;
            let progress = Arc::clone(&op.progress);
            drop(state);
            let (requests_tx, requests_rx) = crossbeam_channel::bounded(256);
            let (done_tx, done_rx) = crossbeam_channel::bounded(1);
            let (output_tx, output_rx) = crossbeam_channel::bounded(1024);
            let sources = offer.sources.clone();
            std::thread::spawn(move || {
                let result = wire::stream_offer(sources, requests_rx, progress, output_tx);
                let _ = done_tx.send(result);
            });
            self.dnd.export_op = Some(id);
            self.dnd.export_tx = Some(requests_tx);
            self.dnd.export_done = Some(done_rx);
            self.dnd.export_output = Some(output_rx);
            self.dnd.export_drag_finished = false;
            self.dnd.export_cancelled = false;
        }
        if self
            .dnd
            .export_tx
            .as_ref()
            .is_some_and(|tx| tx.try_send(index).is_err())
        {
            self.dnd.export_cancelled = true;
            self.dnd.export_tx = None;
            self.dnd.export_output = None;
            if let Some(id) = self.dnd.export_op {
                self.core.send(Command::Cancel(id));
            }
            let _ = wire::send("t=E:i=1", Some("EMFILE:too many drag requests"));
        }
    }

    fn dnd_queue(&mut self, sources: Vec<PathBuf>, kind: OpKind) {
        let Some(choice) = self.dnd.choice.take() else {
            return;
        };
        if sources.is_empty() {
            self.dnd_error();
            return;
        }
        self.core.send(Command::QueueDrop {
            kind,
            sources,
            dest: choice.dest,
        });
        let op = self.core.state().queue.iter().last().map(|op| op.id);
        if let Some(op) = op {
            self.dnd.active = Some(Active {
                op,
                result_operation: kind,
            });
        } else {
            self.dnd_error();
        }
    }

    pub(super) fn dnd_poll_completion(&mut self) {
        self.dnd_flush_output();
        self.dnd_poll_export();
        let Some(active) = &self.dnd.active else {
            return;
        };
        let status = self
            .core
            .state()
            .queue
            .iter()
            .find(|op| op.id == active.op)
            .map(|op| (op.status, op.skipped));
        let Some(operation) = final_drop_operation(status, active.result_operation) else {
            return;
        };
        tracing::info!(operation, "OSC 72 drop finished");
        if matches!(status, Some((OpStatus::Done, skipped)) if skipped > 0)
            && active.result_operation == OpKind::Move
        {
            self.note = Some((
                "Some dropped files were skipped; the source was kept".into(),
                NoteLevel::Warning,
                Instant::now(),
            ));
        }
        let _ = wire::send(&format!("t=r:o={operation}:i=1"), None);
        self.dnd.active = None;
        self.dnd.staged = None;
        self.dnd.result_kind = None;
    }

    fn dnd_flush_output(&mut self) {
        let Some(rx) = &self.dnd.export_output else {
            return;
        };
        for _ in 0..2048 {
            let Ok(out) = rx.try_recv() else { break };
            if wire::send(&out.meta, out.payload.as_deref()).is_err() {
                if let Some(id) = self.dnd.export_op {
                    self.core.send(Command::Cancel(id));
                }
                break;
            }
        }
    }

    fn dnd_poll_export(&mut self) {
        let Some(rx) = &self.dnd.export_done else {
            return;
        };
        // The worker can finish after queuing many OSC chunks. Keep the
        // output receiver alive until every queued chunk reaches the TTY.
        if self
            .dnd
            .export_output
            .as_ref()
            .is_some_and(|output| !output.is_empty())
        {
            return;
        }
        let Ok(result) = rx.try_recv() else { return };
        let success = result.is_ok() && !self.dnd.export_cancelled;
        if !success && !self.dnd.export_cancelled {
            let _ = wire::send("t=E:i=1", Some("EIO:drag export failed"));
        }
        if let Some(id) = self.dnd.export_op.take() {
            self.core.send(Command::FinishExport { op: id, success });
        }
        self.dnd.export_done = None;
        self.dnd.export_output = None;
        self.dnd.offer = None;
    }

    pub(super) fn dnd_cancel_choice(&mut self) {
        if self.dnd.choice.take().is_none() {
            return;
        }
        let _ = wire::send("t=r:o=0:i=1", None);
        if let Some(op) = self.dnd.import_op.take() {
            self.core.send(Command::FinishImport { op, success: false });
        }
        self.dnd.receiving_uri = false;
        self.dnd.received.clear();
        self.dnd.result_kind = None;
        self.dnd.remote = None;
        self.dnd.staged = None;
    }

    fn dnd_error_with(&mut self, reason: impl std::fmt::Display) {
        tracing::warn!(%reason, "OSC 72 drop transfer failed");
        if self.dnd.choice.is_some() {
            self.dnd_cancel_choice();
        } else if let Some(active) = &self.dnd.active {
            // Keep staged sources alive until the worker acknowledges cancel.
            self.core.send(Command::Cancel(active.op));
        } else {
            let _ = wire::send("t=r:o=0:i=1", None);
        }
        self.note = Some((
            format!("Drag and drop transfer failed: {reason}"),
            NoteLevel::Error,
            Instant::now(),
        ));
    }

    fn dnd_error(&mut self) {
        self.dnd_error_with("invalid drop data");
    }

    fn dnd_sources(&self, x: i32, y: i32) -> Option<Vec<PathBuf>> {
        let (stack_index, row) = self.dnd_hit_row(x, y)?;
        let state = self.core.state();
        let stack = state.tabs.active().stacks.get(stack_index)?;
        let frame = stack.active();
        let entry = *state.rows(frame).get(row)?;
        let selection = state.selection_for_stack(stack_index);
        let sources: Vec<PathBuf> = if selection.is_marked(&entry.path) {
            selection.paths().map(PathBuf::from).collect()
        } else {
            vec![entry.path.clone()]
        };
        sources
            .into_iter()
            .map(|path| {
                if path.is_absolute() {
                    Some(path)
                } else {
                    Some(std::env::current_dir().ok()?.join(path))
                }
            })
            .collect()
    }

    fn dnd_hit_row(&self, x: i32, y: i32) -> Option<(usize, usize)> {
        let (x, y) = (u16::try_from(x).ok()?, u16::try_from(y).ok()?);
        let regions = self.layout.last.as_ref()?;
        if regions.hit(x, y) != Some(ModuleId::Stack) || self.overlays.is_open() {
            return None;
        }
        let area = regions.rect_of(ModuleId::Stack);
        if self.commander {
            let pane = usize::from(x >= area.x + area.width / 2);
            let rect = pane_rect(area, pane);
            let hit = panels::stack::hit(rect, &self.pane_view(pane), x, y)?;
            if let panels::stack::Hit::Row(row) = hit {
                Some((pane + 1, row))
            } else {
                None
            }
        } else {
            let hit = panels::stack::hit(area, &self.stack_view(), x, y)?;
            if let panels::stack::Hit::Row(row) = hit {
                Some((0, row))
            } else {
                None
            }
        }
    }

    fn dnd_target(&self, x: i32, y: i32) -> Option<PathBuf> {
        let (x, y) = (u16::try_from(x).ok()?, u16::try_from(y).ok()?);
        let regions = self.layout.last.as_ref()?;
        if regions.hit(x, y) != Some(ModuleId::Stack) || self.overlays.is_open() {
            return None;
        }
        let area = regions.rect_of(ModuleId::Stack);
        let (stack_index, rect): (usize, Rect) = if self.commander {
            let pane = usize::from(x >= area.x + area.width / 2);
            (pane + 1, pane_rect(area, pane))
        } else {
            (0, area)
        };
        if y == rect.y {
            return None;
        }
        let state = self.core.state();
        let stack = state.tabs.active().stacks.get(stack_index)?;
        let frame = stack.active();
        let hit = if self.commander {
            panels::stack::hit(rect, &self.pane_view(stack_index - 1), x, y)
        } else {
            panels::stack::hit(rect, &self.stack_view(), x, y)
        };
        let target = match hit {
            Some(panels::stack::Hit::Row(row)) => {
                let entry = *state.rows(frame).get(row)?;
                if entry.kind == EntryKind::Dir || entry.link_kind == Some(EntryKind::Dir) {
                    Some(entry.path.clone())
                } else {
                    // A file row is still part of this directory's drop
                    // surface. In a full listing there may be no blank row
                    // to target, so rejecting it makes desktop drops appear
                    // entirely unavailable.
                    Some(frame.dir.clone())
                }
            }
            Some(panels::stack::Hit::Crumb(i)) if !self.commander => {
                state.crumbs().get(i).map(|f| f.dir.clone())
            }
            _ => Some(frame.dir.clone()),
        }?;
        let target = if target.is_absolute() {
            target
        } else {
            std::env::current_dir().ok()?.join(target)
        };
        if self.dnd.offer.as_ref().is_some_and(|offer| {
            offer.sources.iter().any(|source| {
                target == *source
                    || std::fs::symlink_metadata(source)
                        .is_ok_and(|meta| meta.is_dir() && target.starts_with(source))
            })
        }) {
            return None;
        }
        Some(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skipped_move_keeps_the_desktop_source() {
        assert_eq!(
            final_drop_operation(Some((OpStatus::Done, 0)), OpKind::Move),
            Some(2)
        );
        assert_eq!(
            final_drop_operation(Some((OpStatus::Done, 1)), OpKind::Move),
            Some(1)
        );
        assert_eq!(
            final_drop_operation(Some((OpStatus::Failed, 0)), OpKind::Move),
            Some(0)
        );
        assert_eq!(
            final_drop_operation(Some((OpStatus::Running, 0)), OpKind::Move),
            None
        );
    }
}
