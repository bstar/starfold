//! Native rack contents, painted from the controller's cached views.
use super::*;
use crate::ui::rack::{self, Colors};
use starkit::native_surface::{PixelRect as R, Surface};

#[derive(Clone, Copy)]
pub(super) enum RackHit {
    Action(Action),
    Word(panels::Word),
    Pane(usize, Action),
    Operation(usize),
}

struct Canvas<'a> {
    rect: Rect,
    cell: (u16, u16),
    colors: &'a Colors,
    surface: Surface,
    hits: Vec<(Rect, RackHit)>,
}
impl<'a> Canvas<'a> {
    fn new(rect: Rect, cell: (u16, u16), colors: &'a Colors) -> Self {
        let mut surface = Surface::new(
            rect.width * cell.0,
            rect.height * cell.1,
            rack::hex(colors.frame),
        );
        let bounds = R::new(0, 0, surface.width, surface.height);
        rack::bevel(&mut surface, bounds, colors, false);
        surface.fill(
            R::new(
                1,
                1,
                bounds.width.saturating_sub(2),
                bounds.height.saturating_sub(2),
            ),
            &rack::hex(colors.frame),
            0,
        );
        Self {
            rect,
            cell,
            colors,
            surface,
            hits: vec![],
        }
    }
    fn pixels(&self, r: Rect) -> R {
        R::new(
            (r.x - self.rect.x) * self.cell.0,
            (r.y - self.rect.y) * self.cell.1,
            r.width * self.cell.0,
            r.height * self.cell.1,
        )
    }
    fn fill(&mut self, r: Rect, color: starkit::theme::color::Rgb) {
        self.surface.fill(self.pixels(r), &rack::hex(color), 0);
    }
    fn text(&mut self, r: Rect, text: impl Into<String>, accent: bool, small: bool) {
        let color = if accent {
            self.colors.accent
        } else {
            self.colors.muted
        };
        let font = starkit::native_surface::Metrics::from_cell(self.cell.0, self.cell.1)
            .font
            .saturating_sub(if small { 4 } else { 2 })
            .max(1);
        let mut pixels = self.pixels(r);
        pixels.x += 6;
        pixels.width = pixels.width.saturating_sub(12);
        rack::mono(&mut self.surface, pixels, text, color, font, false);
    }
    fn filename(&mut self, r: Rect, name: String, highlighted: bool) {
        let mut px = self.pixels(r);
        px.x += 6;
        px.width = px.width.saturating_sub(12);
        let font = starkit::native_surface::Metrics::from_cell(self.cell.0, self.cell.1)
            .font
            .saturating_sub(2)
            .max(1);
        rack::mono(
            &mut self.surface,
            px,
            name,
            if highlighted {
                self.colors.accent
            } else {
                self.colors.foreground
            },
            font,
            false,
        );
    }
    fn meter(&mut self, r: Rect, fraction: f64) {
        let px = self.pixels(r);
        let count = px.width / 8;
        for index in 0..count {
            self.surface.fill(
                R::new(px.x + index * 8, px.y + px.height / 2, 6, 4.min(px.height)),
                &rack::hex(if f64::from(index) < fraction * f64::from(count) {
                    self.colors.accent
                } else {
                    self.colors.inset
                }),
                0,
            );
        }
    }
    fn well(&mut self, r: Rect) {
        let px = self.pixels(r);
        rack::bevel(&mut self.surface, px, self.colors, true);
        let mut px = self.pixels(r);
        px.x += 1;
        px.y += 1;
        px.width = px.width.saturating_sub(2);
        px.height = px.height.saturating_sub(2);
        self.surface.fill(px, &rack::hex(self.colors.inset), 0);
    }
    fn button(&mut self, r: Rect, label: &str, hit: RackHit, pressed: bool) {
        let mut px = self.pixels(r);
        px.x += 2;
        px.y += 2;
        px.width = px.width.saturating_sub(4);
        px.height = px.height.saturating_sub(4);
        rack::bevel(&mut self.surface, px, self.colors, pressed);
        let font = starkit::native_surface::Metrics::from_cell(self.cell.0, self.cell.1)
            .font
            .saturating_sub(2)
            .max(1);
        let text_width = (starkit::wrap::width_of(label) * font * 3 / 5).min(px.width);
        let text_rect = R::new(
            px.x + px.width.saturating_sub(text_width) / 2,
            px.y,
            text_width,
            px.height,
        );
        rack::mono(
            &mut self.surface,
            text_rect,
            label,
            if pressed {
                self.colors.accent
            } else {
                self.colors.foreground
            },
            font,
            false,
        );
        self.hits.push((r, hit));
    }
    fn title(&mut self, label: &str) {
        let r = Rect::new(
            self.rect.x + 1,
            self.rect.y + 1,
            self.rect.width.saturating_sub(2),
            1,
        );
        self.fill(r, self.colors.title);
        self.text(r, label, false, true);
    }
    fn finish(self, scene: &mut Scene, hits: &mut Vec<(Rect, RackHit)>) {
        scene.components.push(Component::Surface {
            rect: self.rect.into(),
            surface: self.surface,
        });
        hits.extend(self.hits);
    }
}

/// Ordinary image inspection uses a thumbnail beside its metadata. Movie,
/// document and embedded-player geometries remain owned by their renderers.
pub(super) fn image_rect(area: Rect) -> Rect {
    let body = panels::preview::content_rect(area);
    Rect::new(
        body.x,
        body.y,
        body.width * 3 / 10,
        body.height.saturating_sub(2),
    )
}

impl App {
    pub(in crate::ui::app) fn rack_queue_rect(&self, regions: &Regions) -> Option<Rect> {
        let stack = regions.rect_of(ModuleId::Stack);
        let area = regions.rect_of(ModuleId::Operations);
        (self.layout.native_rack && stack.width >= 100 && stack.height >= 18 && area.height >= 14)
            .then(|| Rect::new(area.x + 2, area.y + 7, area.width - 4, area.height - 11))
    }

    pub(super) fn rack_background_click(&mut self, x: u16, y: u16) -> bool {
        if self.graphical.as_ref().unwrap().rack_hits.is_empty() {
            return false;
        }
        let Some(regions) = self.layout.last.as_ref() else {
            return false;
        };
        if [
            Bar::Commander(0),
            Bar::Commander(1),
            Bar::Stack,
            Bar::Operations,
            Bar::Preview,
        ]
        .iter()
        .any(|bar| {
            self.bars
                .track_of(*bar)
                .is_some_and(|r| r.contains((x, y).into()))
        }) {
            return false;
        }
        let stack = regions.rect_of(ModuleId::Stack);
        if stack.contains((x, y).into()) {
            let pane = usize::from(self.commander && x >= stack.x + stack.width / 2);
            let rect = if self.commander {
                pane_rect(stack, pane)
            } else {
                stack
            };
            let view = if self.commander {
                self.pane_view(pane)
            } else {
                self.stack_view()
            };
            let split = panels::stack::split(
                starkit::chrome::frame::body(rect, &panels::words(ModuleId::Stack)),
                view.crumbs.len(),
                view.fold_rows,
            );
            if split.list.contains((x, y).into()) || split.crumbs.contains((x, y).into()) {
                return false;
            }
            if self.commander {
                self.focus_pane(pane);
            }
            return true;
        }
        if self.rack_queue_rect(regions).is_some()
            && regions
                .rect_of(ModuleId::Operations)
                .contains((x, y).into())
        {
            self.layout.focus_set(ModuleId::Operations);
            return true;
        }
        if matches!(self.view.preview.as_deref(), Some(Preview::Image { .. }))
            && regions.rect_of(ModuleId::Preview).height >= 14
            && regions.rect_of(ModuleId::Preview).contains((x, y).into())
        {
            self.layout.focus_set(ModuleId::Preview);
            return true;
        }
        regions.status.height >= 3 && regions.status.contains((x, y).into())
    }
    pub(super) fn rack_click(&mut self, hit: RackHit) {
        match hit {
            RackHit::Action(action) => self.act(action),
            RackHit::Word(word) => {
                if matches!(
                    word,
                    panels::Word::Filter
                        | panels::Word::Actions
                        | panels::Word::Places
                        | panels::Word::Hidden
                ) {
                    self.layout.focus_set(ModuleId::Stack);
                }
                self.word_click(word);
            }
            RackHit::Pane(pane, action) => {
                if self.commander {
                    self.focus_pane(pane);
                }
                self.act(action);
            }
            RackHit::Operation(index) => {
                self.layout.focus_set(ModuleId::Operations);
                self.ops_cursor = index;
            }
        }
    }

    pub(super) fn rack_scene(&mut self, scene: &mut Scene, regions: &Regions, cell: (u16, u16)) {
        let stack = regions.rect_of(ModuleId::Stack);
        // Compact windows keep their established, independently tested chrome.
        if stack.width < 100 || stack.height < 18 {
            return;
        }
        let palette = self.theme.rack.clone();
        let p = &palette;
        let mut hits = vec![];
        let mut c = Canvas::new(stack, cell, p);
        c.title(if self.commander {
            "S T A R / F O L D   /   COMMANDER"
        } else {
            "S T A R / F O L D   /   FOLD"
        });
        let controls = [
            ("Places", RackHit::Word(panels::Word::Places), 10),
            ("Filter", RackHit::Word(panels::Word::Filter), 10),
            ("Actions", RackHit::Word(panels::Word::Actions), 11),
            ("Help", RackHit::Action(Action::Help), 8),
        ];
        let mut x = stack.right() - 41;
        for (label, hit, width) in controls {
            c.button(Rect::new(x, stack.y + 1, width, 1), label, hit, false);
            x += width;
        }
        for pane in 0..if self.commander { 2 } else { 1 } {
            let rect = if self.commander {
                pane_rect(stack, pane)
            } else {
                stack
            };
            let view = if self.commander {
                self.pane_view(pane)
            } else {
                self.stack_view()
            };
            let body = starkit::chrome::frame::body(rect, &panels::words(ModuleId::Stack));
            let split = panels::stack::split(body, view.crumbs.len(), view.fold_rows);
            let list = split.list;
            for span in &scene.spans {
                if split.crumbs.contains((span.x, span.y).into()) {
                    c.text(
                        Rect::new(span.x, span.y, split.crumbs.right() - span.x, 1),
                        &span.text,
                        false,
                        true,
                    );
                }
            }
            let well_x = if self.commander && pane == 1 {
                rect.x
            } else {
                rect.x + 1
            };
            let well_right = if self.commander && pane == 0 {
                rect.right()
            } else {
                rect.right() - 1
            };
            c.well(Rect::new(
                well_x,
                split.rule.y,
                well_right - well_x,
                list.bottom().saturating_sub(split.rule.y) + 1,
            ));
            let path = if self.commander {
                &self.panes[pane].dir
            } else {
                &self.view.active_dir
            };
            let path_area = Rect::new(
                split.rule.x + 2,
                split.rule.y,
                split
                    .rule
                    .width
                    .saturating_sub(2 + breadcrumb_detail_width(&view)),
                1,
            );
            c.text(
                Rect::new(body.x, split.rule.y, 2, 1),
                if pane == 0 { "L" } else { "R" },
                true,
                true,
            );
            // Keep each breadcrumb's actual hit columns; clipped glyphs cannot
            // move a path segment into a neighbour's click target.
            for (r, label, _) in breadcrumb_slots(path_area, path, &self.view.home) {
                let mut offset = 0;
                for ch in label.chars() {
                    let width = starkit::wrap::width_of(&ch.to_string());
                    if width > 0 {
                        let px = c.pixels(Rect::new(r.x + offset, r.y, width, 1));
                        rack::mono(
                            &mut c.surface,
                            px,
                            ch.to_string(),
                            p.accent,
                            14.min(cell.1),
                            false,
                        );
                        offset += width;
                    }
                }
            }
            let editing = (pane == self.active_pane)
                .then_some(self.filter.as_ref())
                .flatten();
            if let Some(input) = editing {
                let mut query = input.text().to_string();
                query.insert(input.cursor(), '│');
                c.fill(split.rule, p.inset);
                c.text(split.rule, format!("FILTER / {query}"), true, false);
                c.hits
                    .push((split.rule, RackHit::Word(panels::Word::Filter)));
            } else if let Some(query) = view.filter {
                c.fill(split.rule, p.inset);
                c.text(split.rule, format!("FILTER / {query}"), true, false);
                c.hits
                    .push((split.rule, RackHit::Word(panels::Word::Filter)));
            }
            let name_width = if list.width >= 55 {
                list.width - 24
            } else {
                list.width.saturating_sub(15)
            };
            let size_x = list.x + name_width;
            let time_x = size_x + 8;
            let kind_x = if list.width >= 55 { time_x + 9 } else { time_x };
            let heading = Rect::new(list.x, list.y - 1, list.width, 1);
            c.fill(
                Rect::new(well_x, heading.y, well_right - well_x, 1),
                p.title,
            );
            c.text(
                Rect::new(list.x, heading.y, name_width, 1),
                "NAME",
                false,
                true,
            );
            c.text(Rect::new(size_x, heading.y, 8, 1), "SIZE", false, true);
            c.text(Rect::new(kind_x, heading.y, 7, 1), "TYPE", false, true);
            if list.width >= 55 {
                c.text(Rect::new(time_x, heading.y, 9, 1), "MODIFIED", false, true);
            }
            for (index, row) in view
                .rows
                .iter()
                .enumerate()
                .skip(view.scroll)
                .take(usize::from(list.height))
            {
                let y = list.y + (index - view.scroll) as u16;
                let selected = view.focused && index == view.cursor;
                if selected {
                    c.fill(
                        Rect::new(list.x, y, list.width, 1),
                        p.inset.mix(p.accent, 0.22),
                    );
                }
                let icon = match row.kind {
                    panels::stack::Kind::Dir => "▸",
                    panels::stack::Kind::Symlink { .. } => "↗",
                    _ => "·",
                };
                let marked = row.mark == panels::stack::Mark::Marked;
                c.filename(
                    Rect::new(list.x, y, name_width, 1),
                    format!("{} {icon} {}", if marked { "■" } else { " " }, row.name),
                    selected || marked,
                );
                c.text(Rect::new(size_x, y, 8, 1), &row.size, false, true);
                c.text(Rect::new(kind_x, y, 7, 1), &row.ext, false, true);
                if list.width >= 55 {
                    c.text(Rect::new(time_x, y, 9, 1), &row.time, false, true);
                }
            }
            if view.loading || view.error.is_some() {
                c.text(
                    list,
                    view.error.unwrap_or("Reading directory…"),
                    false,
                    false,
                );
            }
            let marked = view
                .rows
                .iter()
                .filter(|r| r.mark == panels::stack::Mark::Marked)
                .count();
            c.text(
                Rect::new(list.x, list.bottom(), list.width, 1),
                format!(
                    "{} entries · {marked} marked{}",
                    view.rows.len(),
                    if pane == self.active_pane {
                        " · ACTIVE PANE"
                    } else {
                        ""
                    }
                ),
                true,
                true,
            );
            let toolbar_y = list.bottom() + 1;
            let mut bx = body.x;
            for (label, action, width) in [
                ("↑ Up", Action::Pop, 8),
                ("Yank", Action::Yank, 8),
                ("Paste", Action::Paste, 9),
                ("Move", Action::QueueMove, 8),
            ] {
                if bx + width <= body.right() {
                    c.button(
                        Rect::new(bx, toolbar_y, width, 1),
                        label,
                        RackHit::Pane(pane, action),
                        false,
                    );
                    bx += width;
                }
            }
            if bx + 10 <= body.right() {
                c.button(
                    Rect::new(bx, toolbar_y, 10, 1),
                    "Hidden",
                    RackHit::Word(panels::Word::Hidden),
                    self.view.show_hidden,
                );
            }
            c.text(
                Rect::new(body.x, toolbar_y + 1, body.width, 1),
                view.space
                    .map(|(_, free)| {
                        format!(
                            "{} free · {}",
                            crate::fold::format::size(free),
                            home_relative(path, &self.view.home)
                        )
                    })
                    .unwrap_or_else(|| home_relative(path, &self.view.home)),
                false,
                true,
            );
        }
        c.finish(scene, &mut hits);
        self.rack_operations(scene, regions, cell, &mut hits);
        self.rack_inspection(scene, regions, cell, &mut hits);
        if regions.status.height >= 3 {
            let area = regions.status;
            let mut c = Canvas::new(area, cell, p);
            c.well(Rect::new(area.x + 1, area.y, area.width - 2, 2));
            let status = self.status_view(Instant::now());
            let message = status
                .note
                .filter(|(_, _, at)| status.now.duration_since(*at) < status::NOTE_FOR)
                .map(|(text, _, _)| text.as_str())
                .unwrap_or_else(|| status.progress.unwrap_or("Ready"));
            c.text(
                Rect::new(area.x + 2, area.y, area.width - 4, 2),
                format!(
                    "NORMAL  :  {} · {} · {message}",
                    if self.commander {
                        if self.active_pane == 0 {
                            "Left pane"
                        } else {
                            "Right pane"
                        }
                    } else {
                        "Fold"
                    },
                    if self.view.marked.is_empty() {
                        "0 marked"
                    } else {
                        &self.view.marked
                    }
                ),
                true,
                false,
            );
            c.text(Rect::new(area.x + 1, area.y + 2, area.width - 2, 1), "? help       b places       / filter       space mark       y yank       p paste       m move       i inspect       j k navigate", false, true);
            c.hits.push((
                Rect::new(area.x + 1, area.y + 2, 8, 1),
                RackHit::Action(Action::Help),
            ));
            c.finish(scene, &mut hits);
        }
        if let Some(tabs) = regions.tabs.filter(|r| r.height >= 9) {
            let row = Rect::new(tabs.x + 1, tabs.bottom() - 2, 13, 2);
            let mut c = Canvas::new(row, cell, p);
            let mut x = row.x;
            for (label, action, width) in [
                ("↑", Action::Pop, 4),
                ("⌂", Action::GoHome, 4),
                ("↻", Action::Reload, 4),
            ] {
                c.button(
                    Rect::new(x, row.y, width, 2),
                    label,
                    RackHit::Action(action),
                    false,
                );
                x += width;
            }
            c.finish(scene, &mut hits);
            let row = Rect::new(tabs.right() - 16, tabs.bottom() - 2, 15, 2);
            let mut c = Canvas::new(row, cell, p);
            c.button(
                row,
                if self.commander { "Commander" } else { "Fold" },
                RackHit::Action(Action::ToggleView),
                true,
            );
            c.finish(scene, &mut hits);
        }
        self.graphical.as_mut().unwrap().rack_hits = hits;
    }

    fn rack_operations(
        &mut self,
        scene: &mut Scene,
        regions: &Regions,
        cell: (u16, u16),
        hits: &mut Vec<(Rect, RackHit)>,
    ) {
        let area = regions.rect_of(ModuleId::Operations);
        if area.height < 14 {
            return;
        }
        let p = &self.theme.rack;
        let mut c = Canvas::new(area, cell, p);
        c.title("OPERATIONS / TRANSFER QUEUE");
        let view = self.operations_view();
        let (fraction, estimate) = {
            let state = self.core.state();
            let running = state.queue.iter().find(|op| op.status == OpStatus::Running);
            let fraction = running.map(|op| op.progress.fraction()).unwrap_or_else(|| {
                if !view.rows.is_empty()
                    && view
                        .rows
                        .iter()
                        .all(|r| r.tone == panels::operations::Tone::Done)
                {
                    1.0
                } else {
                    0.0
                }
            });
            let estimate = running
                .filter(|op| matches!(op.kind, OpKind::Copy | OpKind::Move))
                .and_then(|op| op.progress.transfer_estimate());
            (fraction, estimate)
        };
        let width = (area.width - 4) / 3;
        let stats = [
            (
                "TRANSFER",
                estimate
                    .map(|(rate, _)| format!("{}/s", crate::fold::format::size(rate)))
                    .unwrap_or_else(|| "—".into()),
            ),
            (
                "REMAINING",
                estimate
                    .and_then(|(_, eta)| eta)
                    .map(|eta| format!("{}:{:02}", eta.as_secs() / 60, eta.as_secs() % 60))
                    .unwrap_or_else(|| "—".into()),
            ),
            ("QUEUE", format!("{:02}", view.rows.len())),
        ];
        for (index, (label, value)) in stats.into_iter().enumerate() {
            let r = Rect::new(area.x + 2 + index as u16 * width, area.y + 3, width - 1, 3);
            c.well(r);
            c.text(Rect::new(r.x, r.y, r.width, 1), label, false, true);
            c.text(Rect::new(r.x, r.y + 1, r.width, 2), value, true, false);
        }
        let queue = self.rack_queue_rect(regions).unwrap();
        c.text(
            Rect::new(queue.x, queue.y - 1, queue.width, 1),
            if view.paused {
                "REQUEST / PAUSED"
            } else {
                "REQUEST / STATE"
            },
            false,
            true,
        );
        c.well(queue);
        let mut y = queue.y;
        for activity in [view.incoming.as_deref(), view.unmounting]
            .into_iter()
            .flatten()
        {
            if y + 2 > queue.bottom() {
                break;
            }
            c.text(
                Rect::new(queue.x + 1, y, queue.width - 2, 2),
                activity,
                true,
                true,
            );
            y += 2;
        }
        for (index, row) in view.rows.iter().enumerate().skip(view.scroll) {
            if y + 2 > queue.bottom() {
                break;
            }
            let r = Rect::new(queue.x + 1, y, queue.width - 2, 2);
            if view.focused && view.cursor == index {
                c.fill(r, p.inset.mix(p.accent, 0.22));
            }
            c.text(Rect::new(r.x, y, r.width, 1), &row.title, true, true);
            c.text(Rect::new(r.x, y + 1, r.width, 1), &row.status, false, true);
            c.hits.push((r, RackHit::Operation(index)));
            y += 2;
        }
        if view.rows.is_empty() && view.incoming.is_none() && view.unmounting.is_none() {
            c.text(queue, "No queued operations", false, true);
        }
        c.meter(
            Rect::new(area.x + 2, area.bottom() - 4, area.width - 11, 1),
            fraction,
        );
        c.text(
            Rect::new(area.right() - 8, area.bottom() - 4, 6, 1),
            format!("{:.0}%", fraction * 100.0),
            true,
            true,
        );
        let mut x = area.x + 2;
        for (label, action, width) in [
            ("Stop", Action::CancelRun, 9),
            ("Resume", Action::RunQueue, 10),
            ("Clear", Action::ClearQueue, 9),
        ] {
            c.button(
                Rect::new(x, area.bottom() - 3, width, 2),
                label,
                RackHit::Action(action),
                false,
            );
            x += width;
        }
        let total = view.rows.len() as u32;
        let above = view.scroll as u32;
        let visible = (queue.height / 2)
            .saturating_sub(
                u16::from(view.incoming.is_some()) + u16::from(view.unmounting.is_some()),
            )
            .max(1);
        c.finish(scene, hits);
        self.bars.record_viewport(
            Bar::Operations,
            Rect::new(queue.right() - 1, queue.y, 1, queue.height),
            total,
            above,
            u32::from(visible),
        );
    }

    fn rack_inspection(
        &self,
        scene: &mut Scene,
        regions: &Regions,
        cell: (u16, u16),
        hits: &mut Vec<(Rect, RackHit)>,
    ) {
        let area = regions.rect_of(ModuleId::Preview);
        if area.height < 14
            || !self.layout.preview_open
            || self.audio_here()
            || self.editor.is_some()
        {
            return;
        }
        let Some(Preview::Image {
            width,
            height,
            format,
            ..
        }) = self.view.preview.as_deref()
        else {
            return;
        };
        let p = &self.theme.rack;
        let mut c = Canvas::new(area, cell, p);
        c.title("INSPECTION / SELECTED FILE");
        let art = image_rect(area);
        c.well(art);
        let info = Rect::new(
            art.right() + 2,
            art.y,
            area.right().saturating_sub(art.right() + 4),
            art.height,
        );
        c.text(
            Rect::new(info.x, info.y, info.width, 2),
            self.view.preview_name.as_deref().unwrap_or("Image"),
            true,
            false,
        );
        c.text(
            Rect::new(info.x, info.y + 2, info.width, 1),
            format!("{} · {width} × {height}", format.to_uppercase()),
            false,
            true,
        );
        let row = self
            .view
            .rows
            .iter()
            .find(|r| Some(r.name.trim_end_matches('/')) == self.view.preview_name.as_deref());
        for (i, (label, value)) in [
            (
                "SIZE",
                row.map(|r| r.size.clone()).unwrap_or_else(|| "—".into()),
            ),
            (
                "MODIFIED",
                row.map(|r| r.time.clone()).unwrap_or_else(|| "—".into()),
            ),
            ("SCALE", format!("{:?}", self.cfg.preview.image_scale)),
        ]
        .into_iter()
        .enumerate()
        {
            let y = info.y + 4 + i as u16 * 2;
            if y + 2 <= info.bottom() {
                c.text(Rect::new(info.x, y, info.width, 1), label, false, true);
                c.text(Rect::new(info.x, y + 1, info.width, 1), value, true, true);
            }
        }
        c.button(
            Rect::new(area.x + 2, area.bottom() - 3, 12, 2),
            "Open",
            RackHit::Action(Action::OpenExternal),
            false,
        );
        c.button(
            Rect::new(area.x + 15, area.bottom() - 3, 10, 2),
            "Yank",
            RackHit::Action(Action::Yank),
            false,
        );
        c.button(
            Rect::new(area.x + 26, area.bottom() - 3, 10, 2),
            "Scale",
            RackHit::Word(panels::Word::PictureScale(self.cfg.preview.image_scale)),
            false,
        );
        c.button(
            Rect::new(area.right() - 12, area.bottom() - 3, 10, 2),
            "Close",
            RackHit::Word(panels::Word::Close),
            false,
        );
        c.finish(scene, hits);
    }
}
