//! Semantic file actions and incremental preview navigation.
use super::*;
use crate::ui::overlays;
use std::path::Path;
impl App {
    pub(super) fn request_pdf_pages(&mut self, direction: i32) {
        use crate::fold::preview::model::Content;
        let Some(Preview::Document(d)) = self.view.preview.as_deref() else {
            return;
        };
        let Content::Pages(pages) = &d.content else {
            return;
        };
        let visible = self
            .layout
            .last
            .as_ref()
            .map(|r| panels::preview::content_rect(r.rect_of(ModuleId::Preview)).height as usize)
            .unwrap_or(10);
        let width = self
            .layout
            .last
            .as_ref()
            .map(|r| panels::preview::content_rect(r.rect_of(ModuleId::Preview)).width)
            .unwrap_or(80);
        let total = panels::preview::lines(self.view.preview.as_deref().unwrap(), width).len();
        let page = if direction < 0 && self.preview_scroll < 3 {
            pages
                .first()
                .and_then(|p| (p.number > 1).then(|| p.number.saturating_sub(3).max(1)))
        } else if direction > 0 && self.preview_scroll.saturating_add(visible + 3) >= total {
            d.next_page
        } else {
            None
        };
        let Some(page) = page else {
            return;
        };
        let generation = self.core.state().preview_generation;
        if self.pdf_requested == Some((generation, page)) {
            return;
        }
        self.pdf_requested = Some((generation, page));
        if let Some(path) = self.view.cursor_path.clone() {
            self.core.send(Command::PreviewPage {
                path,
                generation,
                page,
            });
        }
    }

    pub(super) fn open_file_menu(&mut self, x: u16, y: u16) {
        let state = self.core.state();
        let Some(entry) = state.cursor_entry() else {
            return;
        };
        let sources = if state.selection.is_marked(&entry.path) {
            state.selection.paths().map(Path::to_path_buf).collect()
        } else {
            vec![entry.path.clone()]
        };
        let destination = if self.commander {
            self.panes[1 - self.active_pane].dir.clone()
        } else {
            entry.path.parent().unwrap_or(Path::new(".")).to_path_buf()
        };
        let target = overlays::context::Target {
            clicked: entry.path.clone(),
            directory: entry.is_dir_like(),
            sources,
            destination,
        };
        drop(state);
        self.overlays.open_context(target, (x, y));
        self.repaint = true;
    }
    pub(super) fn context_action(
        &mut self,
        target: overlays::context::Target,
        action: overlays::context::Action,
    ) {
        use overlays::context::{Action as A, Request};
        match action {
            A::Open => {
                let current_matches = self
                    .core
                    .state()
                    .cursor_entry()
                    .is_some_and(|e| e.path == target.clicked);
                if target.directory {
                    self.core.send(Command::Push(target.clicked));
                } else if current_matches {
                    self.activate_entry();
                } else {
                    self.core.send(Command::OpenExternal(target.clicked));
                }
            }
            A::Preview => {
                self.layout.focus_set(ModuleId::Preview);
                self.core.send(Command::Preview(target.clicked));
            }
            A::Mark => self.core.send(Command::ToggleMarkPath(target.clicked)),
            A::Rename => self.overlays.open_rename(target.clicked),
            A::Delete => self.core.send(Command::QueueDeleteSources(target.sources)),
            A::Copy | A::Move => self.overlays.open_destination(Request {
                kind: if action == A::Copy {
                    OpKind::Copy
                } else {
                    OpKind::Move
                },
                sources: target.sources,
                destination: target.destination,
            }),
            A::Compress => {
                let name = if target.sources.len() == 1 {
                    target
                        .clicked
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                } else {
                    "Archive".into()
                };
                let destination = target
                    .clicked
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join(format!("{name}.zip"));
                self.overlays.open_destination(Request {
                    kind: OpKind::Compress(crate::fold::archive::Format::Zip),
                    sources: target.sources,
                    destination,
                });
            }
            A::Extract => {
                // Each archive has its own atomic destination and queue row.
                if target.sources.len() == 1 {
                    self.overlays.open_destination(Request {
                        kind: OpKind::Extract,
                        destination: crate::fold::archive::destination(&target.clicked),
                        sources: target.sources,
                    });
                } else {
                    self.overlays.open_destination(Request {
                        kind: OpKind::Extract,
                        destination: target.destination,
                        sources: target.sources,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> (App, crate::ui::fake::Fake) {
        let cfg = crate::config::Config::default();
        let (core, fake) = crate::ui::fake::handle(cfg.core());
        let mut app = App::new(
            core,
            cfg,
            PathBuf::from("/nonexistent/config.toml"),
            None,
            starkit::graphics::Graphics::disabled(),
        );
        fake.pump();
        app.tick();
        (app, fake)
    }
    fn cursor(app: &mut App, name: &str) {
        app.refresh();
        let index = app.view.rows.iter().position(|r| r.name == name).unwrap();
        app.core.send(Command::CursorTo(index));
        app.refresh();
    }
    #[test]
    fn context_target_does_not_capture_unrelated_marks() {
        let (mut app, fake) = app();
        cursor(&mut app, "blob.bin");
        app.core.send(Command::ToggleMark);
        cursor(&mut app, "notes.txt");
        app.open_file_menu(2, 2);
        let Some(Overlay::Context(menu)) = app.overlays.current() else {
            panic!()
        };
        assert_eq!(menu.target.sources, vec![fake.fixture.path("notes.txt")]);
        assert!(fake
            .state()
            .selection
            .is_marked(&fake.fixture.path("blob.bin")));
    }
    #[test]
    fn compression_is_visible_in_operations_before_it_runs() {
        let (mut app, fake) = app();
        let source = fake.fixture.path("blob.bin");
        app.after_overlay_answer(Answer::Operation(overlays::context::Request {
            kind: OpKind::Compress(crate::fold::archive::Format::Zip),
            sources: vec![source],
            destination: fake.fixture.path("test.zip"),
        }));
        app.refresh();
        assert_eq!(app.view.ops.len(), 1);
        assert!(!fake.fixture.path("test.zip").exists());
        app.core.send(Command::Run);
        fake.pump();
        assert!(fake.fixture.path("test.zip").exists());
        assert_eq!(
            fake.state().queue.iter().next().unwrap().status,
            crate::fold::ops::OpStatus::Done
        );
    }
    #[test]
    fn scrolling_loads_pdf_batches_and_keeps_the_scroll_anchor() {
        let (mut app, fake) = app();
        let path = fake.fixture.path("book.pdf");
        crate::fold::testing::write_pdf(&path, 32);
        app.core.send(Command::Reload);
        fake.pump();
        app.tick();
        cursor(&mut app, "book.pdf");
        app.core.send(Command::Preview(path));
        fake.pump();
        app.refresh();
        let area = Rect::new(0, 0, 100, 30);
        app.draw(area, &mut Buffer::empty(area));
        for expected in (6..=30).step_by(3) {
            app.preview_scroll = panels::preview::lines(app.view.preview.as_deref().unwrap(), 80)
                .len()
                .saturating_sub(1);
            app.request_pdf_pages(1);
            fake.pump();
            app.refresh();
            let Some(Preview::Document(d)) = app.view.preview.as_deref() else {
                panic!()
            };
            let crate::fold::preview::model::Content::Pages(p) = &d.content else {
                panic!()
            };
            assert_eq!(p.last().unwrap().number, expected);
            assert!(p.len() <= 24);
        }
        app.preview_scroll = 0;
        app.request_pdf_pages(-1);
        fake.pump();
        app.refresh();
        let Some(Preview::Document(d)) = app.view.preview.as_deref() else {
            panic!()
        };
        let crate::fold::preview::model::Content::Pages(p) = &d.content else {
            panic!()
        };
        assert_eq!(p.first().unwrap().number, 4);
        assert!(app.preview_scroll > 0);
    }
}
