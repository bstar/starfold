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
    pub(super) fn dnd_autoscroll(&mut self) {
        let Some((x, y)) = self.dnd.hover_coords else {
            self.dnd_edge = None;
            return;
        };
        let Some(regions) = self.layout.last.as_ref() else {
            self.dnd_edge = None;
            return;
        };
        if self.bars.held().is_some()
            || self.overlays.is_open()
            || regions.hit(x, y) != Some(ModuleId::Stack)
        {
            self.dnd_edge = None;
            return;
        }

        let area = regions.rect_of(ModuleId::Stack);
        let (bar, stack_index) = if self.commander {
            let pane = usize::from(x >= area.x + area.width / 2);
            (Bar::Commander(pane), pane + 1)
        } else {
            (Bar::Stack, 0)
        };
        if self.dnd.drag_active
            && self.dnd.offer.as_ref().is_some_and(|offer| {
                offer.source_stack == stack_index
                    && offer.source_tab == self.core.state().tabs.active().id
            })
        {
            self.dnd_edge = None;
            return;
        }

        let Some(track) = self.bars.track_of(bar) else {
            self.dnd_edge = None;
            return;
        };
        if track.height < 4 || y < track.y || y >= track.bottom() {
            self.dnd_edge = None;
            return;
        }
        let direction = if y < track.y + 2 {
            -1
        } else if y >= track.bottom() - 2 {
            1
        } else {
            self.dnd_edge = None;
            return;
        };

        let now = Instant::now();
        if self
            .dnd_edge
            .is_some_and(|(held_bar, held_direction, last)| {
                held_bar == bar
                    && held_direction == direction
                    && now.duration_since(last) < Duration::from_millis(120)
            })
        {
            return;
        }
        self.dnd_edge = Some((bar, direction, now));

        self.dnd_scroll_destination(x, y, i32::from(direction));
    }

    pub(super) fn dnd_scroll_destination(&mut self, x: u16, y: u16, direction: i32) {
        let Some(regions) = self.layout.last.as_ref() else {
            return;
        };
        if regions.hit(x, y) != Some(ModuleId::Stack)
            || self.bars.held().is_some()
            || self.overlays.is_open()
        {
            return;
        }
        let area = regions.rect_of(ModuleId::Stack);
        let (bar, stack_index) = if self.commander {
            let pane = usize::from(x >= area.x + area.width / 2);
            (Bar::Commander(pane), pane + 1)
        } else {
            (Bar::Stack, 0)
        };
        if self.dnd.offer.as_ref().is_some_and(|o| {
            o.source_stack == stack_index && o.source_tab == self.core.state().tabs.active().id
        }) {
            return;
        }
        let Some(track) = self.bars.track_of(bar) else {
            return;
        };
        let (above, total) = match bar {
            Bar::Commander(pane) => {
                let p = &self.panes[pane];
                (self.scroll.get(&p.key).copied().unwrap_or(0), p.rows.len())
            }
            Bar::Stack => (
                self.scroll.get(&self.view.frame_id).copied().unwrap_or(0),
                self.view.rows.len(),
            ),
            _ => return,
        };
        let max = total.saturating_sub(usize::from(track.height));
        let next = if direction < 0 {
            above.saturating_sub(direction.unsigned_abs() as usize)
        } else {
            above.saturating_add(direction as usize).min(max)
        };
        if next == above {
            return;
        }
        let active_pane = self.active_pane;
        let focus = self.layout.focus();
        self.scroll_bar_to(bar, next as u32);
        // Edge scrolling a drop destination must not steal selection from
        // the source pane. Only the destination viewport should move.
        if self.commander && self.active_pane != active_pane {
            self.focus_pane(active_pane);
            self.layout.focus_set(focus);
        }

        // A stationary pointer now rests on a different row. Update both
        // STAR/FOLD's highlight and Kitty's acceptance without waiting for
        // another pixel-motion message.
        let target = self.dnd_target(i32::from(x), i32::from(y));
        if target != self.dnd.hover {
            if target.is_some() {
                let action = if self.dnd.hover_allowed == 2 { 2 } else { 1 };
                let _ = wire::send(&format!("t=m:o={action}:i=1"), Some("text/uri-list"));
            } else {
                let _ = wire::send("t=m:o=0:i=1", None);
            }
            self.dnd.hover = target;
            self.dnd.hover_coords = self.dnd.hover.as_ref().map(|_| (x, y));
        }
    }

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
        // A scrollbar owns the gesture from its first press through release.
        // Kitty may still ask for a file offer while that mouse button is held.
        if self.bars.held().is_some()
            || self.suppress_dnd_offer_until_press
            || self.dnd_on_scrollbar(x, y)
        {
            return;
        }
        if self.dnd.choice.is_some() {
            return;
        }
        #[cfg(feature = "terminal-graphics")]
        let captured = self.graphical.as_ref().and_then(|g| g.drag_sources());
        #[cfg(not(feature = "terminal-graphics"))]
        let captured = None;
        let Some((source_stack, sources)) = captured.or_else(|| self.dnd_sources(x, y)) else {
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
            source_stack,
            source_tab: self.core.state().tabs.active().id,
        });
        self.dnd.drag_active = true;
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
        let previous_allowed = self.dnd.hover_allowed;
        let previous_mime = self.dnd.offered_uri;
        self.dnd.hover_allowed = allowed;
        if x >= 0 && y >= 0 && !m.payload.is_empty() {
            self.dnd.offered_uri = m.payload.split_whitespace().any(|s| s == "text/uri-list");
        }
        let mime_ok = self.dnd.offered_uri;
        let target = (self.bars.held().is_none() && mime_ok && allowed != 0 && x >= 0 && y >= 0)
            .then(|| self.dnd_target(x, y))
            .flatten();
        // Native terminals can emit another hover when acceptance is updated.
        // Reply only when acceptance changes; otherwise this forms a feedback
        // loop that floods the scene relay and starves mouse release events.
        let acceptance_changed = target.is_some() != self.dnd.hover.is_some()
            || allowed != previous_allowed
            || self.dnd.offered_uri != previous_mime;
        #[cfg(feature = "terminal-graphics")]
        let should_reply = self.graphical.is_none() || acceptance_changed;
        #[cfg(not(feature = "terminal-graphics"))]
        let should_reply = {
            let _ = acceptance_changed;
            true
        };
        tracing::debug!(
            x,
            y,
            allowed,
            mime_ok,
            accepts = target.is_some(),
            "OSC 72 drag hover received"
        );
        let coords = target.as_ref().map(|_| (x as u16, y as u16));
        self.dnd.hover = target;
        self.dnd.hover_coords = coords;
        if !should_reply {
            return;
        }
        if self.dnd.hover.is_some() {
            let action = if allowed == 2 { 2 } else { 1 };
            let _ = wire::send(&format!("t=m:o={action}:i=1"), Some("text/uri-list"));
        } else {
            let _ = wire::send("t=m:o=0:i=1", None);
        }
        // The regular frame draw updates the hover cells. A full terminal
        // resize here clears the screen on every pointer movement.
    }

    fn dnd_drop(&mut self, m: &Message<'_>) {
        self.dnd.drag_active = false;
        self.dnd.hover = None;
        self.dnd.hover_coords = None;
        self.dnd.offered_uri = false;
        self.dnd.hover_allowed = 0;
        self.dnd_edge = None;
        if self.bars.held().is_some() {
            let _ = wire::send("t=r:o=0:i=1", None);
            return;
        }
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
        tracing::debug!(
            allowed,
            own_sources = own_sources.as_ref().map_or(0, Vec::len),
            "Drop choice source ownership"
        );
        if allowed == 0 {
            let _ = wire::send("t=r:o=0:i=1", None);
            return;
        }
        // Our own source paths remain valid without Kitty's data offer. End
        // the desktop gesture BEFORE presenting a choice/conflict dialog;
        // waiting for UI input must not keep an OS drag session alive.
        let terminal_drop_open = own_sources.is_none();
        if !terminal_drop_open {
            if let Err(error) = wire::send("t=r:o=0:i=1", None) {
                self.dnd_error_with(error);
                return;
            }
        }
        self.dnd.result_kind = None;
        self.dnd.prepared_paths = None;
        self.dnd.choice = Some(Choice {
            dest,
            own_sources,
            allowed,
            mime_index,
            remote: false,
            terminal_drop_open,
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
            // Request metadata while the user chooses. Desktop sources may
            // retain their drag grab until the destination requests its data.
            // Keep the terminal drop open: promised/SSH files need it later.
            if terminal_drop_open {
                self.dnd_request_uri();
            }
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
            tracing::warn!(?kind, "Drop choice has no pending transfer");
            return;
        };
        tracing::debug!(?kind, "Drop choice selected");
        if (kind == OpKind::Move && choice.allowed & 2 == 0)
            || (kind == OpKind::Copy && choice.allowed & 1 == 0)
        {
            self.dnd_cancel_choice();
            return;
        }
        if let Some(sources) = choice.own_sources.clone() {
            self.dnd_queue(sources, kind);
        } else if choice.mime_index.is_some() {
            self.dnd_requested_kind(kind);
            if let Some(paths) = self.dnd.prepared_paths.take() {
                self.dnd_received_paths(paths);
            } else if !self.dnd.receiving_uri {
                self.dnd_request_uri();
            }
        } else {
            self.dnd_cancel_choice();
        }
    }

    fn dnd_request_uri(&mut self) {
        let Some(index) = self.dnd.choice.as_ref().and_then(|c| c.mime_index) else {
            return;
        };
        self.dnd.receiving_uri = true;
        self.dnd.received.clear();
        if let Err(err) = wire::send(&format!("t=r:x={index}:i=1"), None) {
            self.dnd_error_with(err);
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
                    let Some(op) = self.dnd.import_op.take() else {
                        self.dnd_error_with("missing import operation");
                        return;
                    };
                    let result = self.dnd.result_kind.unwrap_or(OpKind::Copy);
                    self.dnd.choice = None;
                    self.core
                        .send(Command::CompleteImport { op, sources: roots });
                    self.dnd.active = Some(Active {
                        op,
                        result_operation: result,
                    });
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
            self.dnd_received_paths(paths);
        }
    }

    fn dnd_received_paths(&mut self, paths: Vec<std::path::PathBuf>) {
        // Linux's native URI list names existing local files, unlike macOS
        // file promises or SSH handles. Hand these paths to our queue before
        // asking for UI input. Hyprland retains its pointer grab until the
        // offer ends, so leaving it open makes the action menu unclickable.
        // Cancel the desktop transfer: only our queue may remove a source,
        // after the user selects Move and the operation succeeds.
        #[cfg(target_os = "linux")]
        if self
            .dnd
            .choice
            .as_ref()
            .is_some_and(|choice| !choice.remote && choice.terminal_drop_open)
            // A different Kitty window can expose downloaded SSH files as
            // local URLs. Those remain owned by its active drag session.
            && !paths.iter().any(|path| {
                path.components().any(|part| part.as_os_str().to_string_lossy().starts_with("dnd-drag-"))
            })
        {
            if let Err(error) = wire::send("t=r:o=0:i=1", None) {
                self.dnd_error_with(error);
                return;
            }
            let choice = self.dnd.choice.as_mut().unwrap();
            choice.terminal_drop_open = false;
            choice.own_sources = Some(paths.clone());
            tracing::debug!(
                files = paths.len(),
                "Local drop handed off; desktop pointer released"
            );
        }
        let Some(kind) = self.dnd.result_kind else {
            if self
                .dnd
                .choice
                .as_ref()
                .is_some_and(|c| c.terminal_drop_open)
            {
                self.dnd.prepared_paths = Some(paths);
            }
            return;
        };
        if self.dnd.choice.as_ref().is_some_and(|choice| choice.remote) {
            tracing::info!(files = paths.len(), "receiving remote drop files");
            let choice = self.dnd.choice.as_ref().unwrap();
            let Some(_) = choice.mime_index else {
                self.dnd_error();
                return;
            };
            let previous = self.core.state().queue.iter().last().map(|op| op.id);
            self.core.send(Command::BeginImport {
                sources: paths.clone(),
                dest: choice.dest.clone(),
            });
            let latest = {
                let state = self.core.state();
                state.queue.iter().last().map(|op| op.id)
            };
            let Some(op_id) = latest.filter(|id| previous.is_none_or(|previous| *id > previous))
            else {
                self.dnd_error_with("copy in progress; destination is locked");
                return;
            };
            self.dnd.import_op = Some(op_id);
            self.dnd.pending_paths = Some(paths);
        } else {
            if self
                .dnd
                .choice
                .as_ref()
                .is_some_and(|c| !c.terminal_drop_open)
            {
                self.dnd_queue(paths, kind);
                return;
            }
            // An external source owns removal on successful Move completion.
            self.dnd_queue(paths, OpKind::Copy);
            if let Some(active) = &mut self.dnd.active {
                active.result_operation = kind;
            }
        }
    }

    pub(super) fn dnd_start_remote(&mut self, op: crate::fold::ops::OpId) {
        if self.dnd.import_op != Some(op) {
            return;
        }
        if !self
            .core
            .state()
            .queue
            .iter()
            .any(|entry| entry.id == op && entry.status == OpStatus::Running)
        {
            return;
        }
        let Some(paths) = self.dnd.pending_paths.take() else {
            return;
        };
        let Some(choice) = self.dnd.choice.as_ref() else {
            return;
        };
        let Some(index) = choice.mime_index else {
            self.dnd_error_with("missing drop format");
            return;
        };
        let dest = choice.dest.clone();
        let found = {
            let state = self.core.state();
            let found = state
                .queue
                .iter()
                .find(|entry| entry.id == op)
                .map(|entry| {
                    let skip = if entry.policy == crate::fold::ops::ConflictPolicy::Skip {
                        entry
                            .plan
                            .as_ref()
                            .map(|plan| {
                                paths
                                    .iter()
                                    .enumerate()
                                    .filter_map(|(i, path)| {
                                        plan.conflicts
                                            .iter()
                                            .any(|conflict| conflict.source == *path)
                                            .then_some(i)
                                    })
                                    .collect()
                            })
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    (Arc::clone(&entry.progress), skip)
                });
            found
        };
        let Some((progress, skip)) = found else {
            self.dnd_error_with("missing import operation");
            return;
        };
        if skip.len() == paths.len() {
            self.dnd.choice = None;
            self.dnd.import_op = None;
            self.core.send(Command::FinishImport { op, success: true });
            self.dnd.active = Some(Active {
                op,
                result_operation: self.dnd.result_kind.unwrap_or(OpKind::Copy),
            });
            return;
        }
        match wire::Remote::new_filtered(&dest, &paths, index, progress, &skip) {
            Ok(mut remote) => match remote.request_next() {
                Ok(false) => self.dnd.remote = Some(remote),
                Ok(true) => self.dnd_error_with("remote drop had no files"),
                Err(error) => self.dnd_error_with(error),
            },
            Err(error) => self.dnd_error_with(error),
        }
    }

    fn dnd_offered_request(&mut self, m: &Message<'_>) {
        if m.number("x") == Some(5) && m.number("y") == Some(0) {
            if let Some(offer) = &self.dnd.offer {
                let _ = wire::send_data("t=e:y=0:i=1", offer.uri_text.as_bytes());
            }
        }
        if m.number("x") == Some(4) {
            self.dnd.drag_active = false;
            self.dnd.hover = None;
            self.dnd.hover_coords = None;
            self.dnd_edge = None;
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
            let previous = self.core.state().queue.iter().last().map(|op| op.id);
            self.core.send(Command::BeginExport(offer.sources.clone()));
            let state = self.core.state();
            let Some(op) = state
                .queue
                .iter()
                .last()
                .filter(|op| previous.is_none_or(|id| op.id > id))
            else {
                drop(state);
                let _ = wire::send("t=E:i=1", Some("EBUSY: copy in progress"));
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
        let Some(choice) = self.dnd.choice.as_ref() else {
            return;
        };
        if sources.is_empty() {
            self.dnd_error();
            return;
        }
        let terminal_drop_open = choice.terminal_drop_open;
        let previous = self.core.state().queue.iter().last().map(|op| op.id);
        self.core.send(Command::QueueDrop {
            kind,
            sources,
            dest: choice.dest.clone(),
        });
        let op = self
            .core
            .state()
            .queue
            .iter()
            .last()
            .map(|op| op.id)
            .filter(|id| previous.is_none_or(|previous| *id > previous));
        if let Some(op) = op {
            self.dnd.choice.take();
            self.dnd.active = terminal_drop_open.then_some(Active {
                op,
                result_operation: kind,
            });
        } else {
            self.dnd_error_with("copy in progress; file is locked");
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
        #[cfg(feature = "terminal-graphics")]
        let limit = if self.graphical.is_some() {
            16.min(wire::graphical_space())
        } else {
            2048
        };
        #[cfg(not(feature = "terminal-graphics"))]
        let limit = 2048;
        for _ in 0..limit {
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
        #[cfg(feature = "terminal-graphics")]
        if self.graphical.as_ref().is_some_and(|state| {
            state.output_pending || state.wire.as_ref().is_some_and(|rx| !rx.is_empty())
        }) {
            return;
        }

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
        let Some(choice) = self.dnd.choice.take() else {
            return;
        };
        if choice.terminal_drop_open {
            let _ = wire::send("t=r:o=0:i=1", None);
        }
        if let Some(op) = self.dnd.import_op.take() {
            self.core.send(Command::FinishImport { op, success: false });
        }
        self.dnd.receiving_uri = false;
        self.dnd.received.clear();
        self.dnd.pending_paths = None;
        self.dnd.prepared_paths = None;
        self.dnd.result_kind = None;
        self.dnd.remote = None;
        self.dnd.staged = None;
        self.dnd_edge = None;
    }

    fn dnd_error_with(&mut self, reason: impl std::fmt::Display) {
        self.dnd.drag_active = false;
        self.dnd.hover = None;
        self.dnd.hover_coords = None;
        self.dnd_edge = None;
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

    pub(super) fn dnd_sources(&self, x: i32, y: i32) -> Option<(usize, Vec<PathBuf>)> {
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
        let paths = sources
            .into_iter()
            .map(|path| {
                if path.is_absolute() {
                    Some(path)
                } else {
                    Some(std::env::current_dir().ok()?.join(path))
                }
            })
            .collect::<Option<Vec<_>>>()?;
        Some((stack_index, paths))
    }

    fn dnd_hit_row(&self, x: i32, y: i32) -> Option<(usize, usize)> {
        if self.dnd_on_scrollbar(x, y) {
            return None;
        }
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

    fn dnd_on_scrollbar(&self, x: i32, y: i32) -> bool {
        let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
            return false;
        };
        [Bar::Stack, Bar::Commander(0), Bar::Commander(1)]
            .into_iter()
            .filter_map(|bar| self.bars.track_of(bar))
            .any(|track| x >= track.x && x < track.right() && y >= track.y && y < track.bottom())
    }

    pub(super) fn dnd_target(&self, x: i32, y: i32) -> Option<PathBuf> {
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
        if !rect.contains((x, y).into()) || y == rect.y {
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
                    || source.parent() == Some(target.as_path())
                    || (target.starts_with(source)
                        && std::fs::symlink_metadata(source).is_ok_and(|meta| meta.is_dir()))
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
