//! Icons replace only the existing two-cell icon slots, never text or marks.
use super::*;
use starkit::visual::{Icon, SurfaceCache, SurfaceKey};
#[derive(Default)]
pub(super) struct Decorations {
    cache: SurfaceCache,
    ids: HashSet<ImageId>,
    theme: String,
}
impl App {
    pub(super) fn decorate(&mut self, _area: Rect, buf: &mut Buffer) {
        if self.overlays.is_open()
            || self.tab_picker.is_some()
            || self.places.is_some()
            || self.dnd.drag_active
        {
            return;
        }
        let Some(cell) = self.graphics.cell_size() else {
            return;
        };
        if !self.graphics.pictures_available() {
            return;
        }
        let Some(regions) = self.layout.last.as_ref() else {
            return;
        };
        let mut slots = vec![];
        let count = if self.commander { self.panes.len() } else { 1 };
        for pane in 0..count {
            let rect = if self.commander {
                pane_rect(regions.rect_of(ModuleId::Stack), pane)
            } else {
                regions.rect_of(ModuleId::Stack)
            };
            let view = if self.commander {
                self.pane_view(pane)
            } else {
                self.stack_view()
            };
            if view.loading || view.error.is_some() {
                continue;
            }
            let body = starkit::chrome::frame::body(rect, &panels::words(ModuleId::Stack));
            let list = panels::stack::split(body, view.crumbs.len(), view.fold_rows).list;
            if list.width < 12 {
                continue;
            }
            for (i, row) in view
                .rows
                .iter()
                .enumerate()
                .skip(view.scroll)
                .take(list.height.into())
            {
                let icon = match row.kind {
                    panels::stack::Kind::Dir => Icon::Folder,
                    panels::stack::Kind::Symlink { .. } => Icon::Link,
                    _ => match row.ext.as_str() {
                        "png" | "jpg" | "jpeg" | "webp" | "gif" => Icon::Image,
                        "mp3" | "flac" | "wav" | "ogg" => Icon::Audio,
                        "mp4" | "mkv" | "webm" => Icon::Video,
                        "zip" | "tar" | "gz" | "rar" | "7z" => Icon::Archive,
                        _ => Icon::File,
                    },
                };
                let r = Rect::new(list.x + 4, list.y + (i - view.scroll) as u16, 2, 1);
                // Derive exact row colours from the text renderer, including marks/cursor.
                let c = &buf[(r.x, r.y)];
                let to_rgb = |c| match c {
                    starkit::ratatui::style::Color::Rgb(r, g, b) => {
                        starkit::theme::color::Rgb::new(r, g, b)
                    }
                    _ => self.theme.panel_bg,
                };
                slots.push((
                    r,
                    SurfaceKey {
                        icon,
                        width: cell.0.saturating_mul(2),
                        height: cell.1,
                        foreground: to_rgb(c.fg),
                        background: to_rgb(c.bg),
                    },
                ));
            }
        }
        let caps: Vec<_> = self
            .tab_hits
            .iter()
            .filter_map(|(rect, hit)| match hit {
                super::super::tabs::Hit::Tab(id)
                    if *id == self.core.state().tabs.active().id && rect.width >= 9 =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .collect();
        let Some(visual) = self.visual.as_mut() else {
            return;
        };
        if visual.theme != self.theme_name {
            for id in visual.ids.drain() {
                self.graphics.forget(id);
            }
            visual.cache.clear();
            visual.theme = self.theme_name.clone();
        }
        for (rect, key) in slots {
            let id = ImageId::of(&(
                "visual-icon",
                key.icon,
                key.width,
                key.height,
                key.foreground.r,
                key.foreground.g,
                key.foreground.b,
                key.background.r,
                key.background.g,
                key.background.b,
            ));
            let Some(image) = visual.cache.icon(key) else {
                continue;
            };
            if let Some(protocol) = self.graphics.raster(id, rect, |_, _| image.to_rgba8()) {
                use starkit::ratatui::widgets::Widget;
                starkit::ratatui_image::Image::new(protocol).render(rect, buf);
                visual.ids.insert(id);
            }
        }
        if cell.0 <= 512 && cell.1 <= 512 {
            let surface = self.theme.panel_bg.mix(self.theme.fg, 0.12);
            for tab in caps {
                for (left, x) in [(true, tab.x), (false, tab.right() - 2)] {
                    let rect = Rect::new(x, tab.y, 1, 1);
                    let id = ImageId::of(&("visual-tab-end", left, cell, self.theme_name.as_str()));
                    if let Some(protocol) = self.graphics.raster(id, rect, |_, _| {
                        starkit::visual::rounded_edge(cell.0, cell.1, self.theme.bg, surface, left)
                            .expect("validated dimensions")
                    }) {
                        use starkit::ratatui::widgets::Widget;
                        starkit::ratatui_image::Image::new(protocol).render(rect, buf);
                        visual.ids.insert(id);
                    }
                }
            }
        }
        // Font resize/scroll/theme combinations cannot grow encoded placements indefinitely.
        if visual.ids.len() > 256 {
            for id in visual.ids.drain() {
                self.graphics.forget(id);
            }
        }
    }
}

/// Visited stack levels, not filesystem ancestry. A jump keeps descendant frames.
#[derive(Clone, Debug)]
pub(super) struct FoldedLevel {
    pub index: usize,
    pub path: PathBuf,
    pub count: usize,
    pub cursor_name: Option<String>,
}
impl App {
    pub(super) fn visual_levels(&self, pane: usize) -> Vec<FoldedLevel> {
        let state = self.core.state();
        let index = if self.commander {
            pane + 1
        } else {
            state.tabs.active().active_stack
        };
        let Some(stack) = state.tabs.active().stacks.get(index) else {
            return vec![];
        };
        let frames = stack.crumbs();
        frames
            .iter()
            .take(frames.len().saturating_sub(1))
            .enumerate()
            .map(|(index, frame)| FoldedLevel {
                index,
                path: frame.dir.clone(),
                count: frame.rows.len(),
                cursor_name: frame
                    .cursor_name
                    .as_ref()
                    .map(|name| name.to_string_lossy().into_owned()),
            })
            .collect()
    }
}

/// Context actions retain the clicked path and that pane's marked set.
#[derive(Clone, Debug)]
pub(super) struct Target {
    pub path: PathBuf,
    pub sources: Vec<PathBuf>,
    pub destination: PathBuf,
    pub directory: bool,
}
impl App {
    pub(super) fn visual_target(&self, pane: usize, index: usize) -> Option<Target> {
        let (paths, rows) = if self.commander {
            let view = self.panes.get(pane)?;
            (&view.paths, &view.rows)
        } else {
            (&self.view.paths, &self.view.rows)
        };
        let path = paths.get(index)?.clone();
        let state = self.core.state();
        let selection = state.selection_for_stack(if self.commander {
            pane + 1
        } else {
            state.tabs.active().active_stack
        });
        let sources = if selection.is_marked(&path) {
            selection.paths().map(PathBuf::from).collect()
        } else {
            vec![path.clone()]
        };
        Some(Target {
            path: path.clone(),
            sources,
            directory: rows.get(index)?.kind == panels::stack::Kind::Dir
                || path
                    .parent()
                    .and_then(|dir| state.listing_of(dir))
                    .is_some_and(|listing| {
                        listing
                            .entries
                            .iter()
                            .any(|entry| entry.path == path && entry.is_dir_like())
                    }),
            destination: if self.commander {
                self.panes.get(1 - pane)?.dir.clone()
            } else {
                self.view.active_dir.clone()
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captured_target_survives_navigation_and_copies_exact_marked_sources() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let temporary = tempfile::tempdir().unwrap();
        let mut app = App::new(
            core,
            cfg,
            temporary.path().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.core.send(Command::ToggleView);
        fake.pump();
        app.tick();
        let source = fake.fixture.path("blob.bin");
        let index = app.panes[0]
            .paths
            .iter()
            .position(|path| *path == source)
            .unwrap();
        app.core.send(Command::ToggleMarkPath(source.clone()));
        app.refresh();
        let captured = app.visual_target(0, index).unwrap();
        app.focus_pane(1);
        app.core.send(Command::Push(fake.fixture.path("empty")));
        fake.pump();
        app.tick();
        assert_eq!(captured.sources, vec![source.clone()]);
        assert_eq!(captured.destination, fake.fixture.home());
        app.core.send(Command::QueueOperation {
            kind: OpKind::Copy,
            sources: captured.sources,
            dest: Some(fake.fixture.path("empty")),
        });
        app.refresh();
        assert_eq!(
            app.view.ops.len(),
            1,
            "queue is visible before the worker executes"
        );
        fake.pump();
        app.tick();
        assert_eq!(
            std::fs::read(fake.fixture.path("empty/blob.bin")).unwrap(),
            std::fs::read(source).unwrap()
        );
    }
    #[test]
    fn missing_graphics_keeps_both_standard_snapshot_sizes_unchanged() {
        let cfg = Config::default();
        let (core, _fake) = crate::ui::fake::handle(cfg.core());
        let temporary = tempfile::tempdir().unwrap();
        let mut app = App::new(
            core,
            cfg,
            temporary.path().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        app.visual = Some(Decorations::default());
        for (width, height) in [(100, 30), (60, 21)] {
            let area = Rect::new(0, 0, width, height);
            let mut buffer = Buffer::empty(area);
            app.draw(area, &mut buffer);
            let before = buffer.clone();
            app.decorate(area, &mut buffer);
            assert_eq!(before, buffer);
        }
    }
    #[test]
    fn folded_levels_are_visited_frames_and_jumps_preserve_filters() {
        let cfg = Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let temporary = tempfile::tempdir().unwrap();
        let mut app = App::new(
            core,
            cfg,
            temporary.path().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        assert!(
            app.visual_levels(0).is_empty(),
            "starting directory has no invented filesystem ancestors"
        );
        app.core.send(Command::Push(fake.fixture.path("projects")));
        fake.pump();
        app.tick();
        app.core.send(Command::SetFilter("star".into()));
        app.core
            .send(Command::Push(fake.fixture.path("projects/starwire")));
        fake.pump();
        app.tick();
        app.core.send(Command::SetFilter("Cargo".into()));
        app.refresh();
        let levels = app.visual_levels(0);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[1].path, fake.fixture.path("projects"));
        app.core.send(Command::JumpTo(levels[1].index));
        fake.pump();
        app.tick();
        assert_eq!(app.view.active_dir, fake.fixture.path("projects"));
        assert_eq!(app.view.filter, "star");
        app.core.send(Command::Forward);
        fake.pump();
        app.tick();
        assert_eq!(app.view.active_dir, fake.fixture.path("projects/starwire"));
        assert_eq!(app.view.filter, "Cargo");
    }
    /// Explicit performance experiment: fixture creation is outside listing and drawing timings.
    #[test]
    #[ignore]
    fn visual_performance_fixture() {
        let temporary = tempfile::tempdir().unwrap();
        for n in 0..10_000 {
            std::fs::write(
                temporary.path().join(format!("file-{n}.txt")),
                b"benchmark\n",
            )
            .unwrap();
        }
        let started = Instant::now();
        let listing = crate::fold::listing::read(
            temporary.path(),
            &crate::fold::listing::ListConfig {
                max_entries: 20_000,
                ..Default::default()
            },
        );
        let listing_ms = started.elapsed().as_secs_f64() * 1000.;
        assert_eq!(listing.entries.len(), 10_000);
        let cfg = Config::default();
        let (core, _fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            temporary.path().join("config.toml"),
            None,
            Graphics::disabled(),
        );
        let row = app.view.rows[0].clone();
        app.view.rows = vec![row; 100_000];
        app.view.loading = false;
        let mut metrics = starkit::visual::Metrics::default();
        for n in 0..200 {
            app.view.cursor = n * 499;
            app.scroll.insert(app.view.frame_id, n * 499);
            let mut buffer = Buffer::empty(Rect::new(0, 0, 100, 30));
            let started = Instant::now();
            app.draw(buffer.area, &mut buffer);
            metrics.frame(started.elapsed());
        }
        let mut cache = SurfaceCache::default();
        let key = SurfaceKey {
            icon: Icon::Folder,
            width: 48,
            height: 48,
            foreground: starkit::theme::color::Rgb::new(120, 180, 240),
            background: starkit::theme::color::Rgb::new(24, 26, 32),
        };
        let started = Instant::now();
        cache.icon(key).unwrap();
        let raster_us = started.elapsed().as_micros();
        let first = cache.icon(key).unwrap();
        assert!(Arc::ptr_eq(&first, &cache.icon(key).unwrap()));
        println!("VISUAL_BENCH listing_10k_ms={listing_ms:.3} terminal_100k_draw_p95_ms={:.3} icon_raster_us={raster_us} cache_bytes={}",metrics.p95_ms(),cache.bytes());
    }
}
