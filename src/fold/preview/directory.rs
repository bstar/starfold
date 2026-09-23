//! Bounded directory discovery. The renderer only receives this tree of values.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::PreviewConfig;
use crate::fold::{file_type::FileType, summary::DirSummary};

#[derive(Debug, Clone)]
pub struct Tree {
    pub entries: Vec<Entry>,
    /// Counts and bytes for the entries actually shown, not a recursive disk usage.
    pub summary: DirSummary,
    pub notice: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Directory,
    Symlink,
    File(FileType),
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    pub children: Vec<Entry>,
    pub notice: Option<String>,
}

struct Pending {
    path: PathBuf,
    name: String,
    kind: Kind,
    bytes: u64,
}

struct Walk<'a> {
    remaining: usize,
    inspected: usize,
    budget: usize,
    deadline: Instant,
    stale: &'a dyn Fn() -> bool,
    summary: DirSummary,
}

impl Walk<'_> {
    fn stopped(&self) -> bool {
        self.remaining == 0 || (self.stale)() || Instant::now() >= self.deadline
    }

    fn directory(&mut self, path: &Path, depth: usize) -> (Vec<Entry>, Option<String>) {
        if self.stopped() {
            self.summary.truncated = true;
            return (vec![], Some("Preview limit reached".into()));
        }
        let entries = match std::fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) => {
                self.summary.truncated = true;
                return (vec![], Some(super::model::clean(&error.to_string(), 256)));
            }
        };
        let mut pending = Vec::new();
        let mut notice = None;
        for entry in entries {
            // Bound both discovery (before sorting) and the displayed tree. A
            // huge single directory must not be collected just to sort it.
            if self.stopped() || self.inspected >= self.budget || pending.len() >= self.remaining {
                notice = Some("Preview limit reached".into());
                break;
            }
            self.inspected += 1;
            let item = entry.and_then(|entry| {
                let path = entry.path();
                let metadata = std::fs::symlink_metadata(&path)?;
                let kind = if metadata.is_symlink() {
                    Kind::Symlink
                } else if metadata.is_dir() {
                    Kind::Directory
                } else {
                    Kind::File(crate::fold::file_type::classify(&path, &[]))
                };
                Ok(Pending {
                    name: display_name(&entry.file_name().to_string_lossy()),
                    path,
                    kind,
                    bytes: if metadata.is_file() {
                        metadata.len()
                    } else {
                        0
                    },
                })
            });
            match item {
                Ok(item) => pending.push(item),
                Err(_) => notice = Some("Some entries could not be read".into()),
            }
        }
        pending.sort_by(|a, b| {
            (a.kind != Kind::Directory, &a.name).cmp(&(b.kind != Kind::Directory, &b.name))
        });
        let mut shown = Vec::new();
        for item in pending {
            if self.stopped() {
                notice = Some("Preview limit reached".into());
                break;
            }
            self.remaining -= 1;
            let (children, child_notice) = if item.kind == Kind::Directory {
                self.summary.dirs += 1;
                if depth >= 4 {
                    self.summary.truncated = true;
                    (vec![], Some("Depth limit".into()))
                } else {
                    self.directory(&item.path, depth + 1)
                }
            } else {
                self.summary.files += 1;
                self.summary.bytes = self.summary.bytes.saturating_add(item.bytes);
                (vec![], None)
            };
            shown.push(Entry {
                name: item.name,
                kind: item.kind,
                children,
                notice: child_notice,
            });
        }
        if notice.is_some() {
            self.summary.truncated = true;
        }
        (shown, notice)
    }
}

fn display_name(name: &str) -> String {
    super::model::clean(name, 1024)
        .chars()
        .map(|c| if c.is_control() { '�' } else { c })
        .collect()
}

pub fn build(path: &Path, cfg: &PreviewConfig, stale: &dyn Fn() -> bool) -> Tree {
    let mut walk = Walk {
        remaining: cfg.max_lines.min(400).min(cfg.dir_budget),
        inspected: 0,
        budget: cfg.dir_budget,
        deadline: Instant::now() + Duration::from_millis(cfg.timeout_ms.min(200)),
        stale,
        summary: DirSummary::default(),
    };
    let (entries, notice) = walk.directory(path, 1);
    Tree {
        entries,
        summary: walk.summary,
        notice,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fold::testing::Fixture;
    use proptest::prelude::*;

    #[test]
    fn directories_sort_first_and_links_are_leaves() {
        let f = Fixture::tree();
        let root = f.path("empty");
        std::fs::create_dir(root.join("z-dir")).unwrap();
        std::fs::write(root.join("z-dir/song.mp3"), b"tags").unwrap();
        std::fs::write(root.join("a.txt"), b"text").unwrap();
        std::os::unix::fs::symlink(&root, root.join("loop")).unwrap();
        let tree = build(&root, &PreviewConfig::default(), &|| false);
        assert_eq!(
            tree.entries
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["z-dir", "a.txt", "loop"]
        );
        assert_eq!(
            tree.entries[0].children[0].kind,
            Kind::File(FileType::Audio)
        );
        assert_eq!(tree.entries[2].kind, Kind::Symlink);
        assert!(tree.entries[2].children.is_empty());
        assert_eq!(
            (tree.summary.dirs, tree.summary.files, tree.summary.bytes),
            (1, 3, 8)
        );
        assert!(!tree.summary.truncated);
    }

    #[test]
    fn limits_and_cancellation_are_explicit() {
        let f = Fixture::tree();
        let root = f.path("projects/starwire");
        for cfg in [
            PreviewConfig {
                max_lines: 2,
                ..PreviewConfig::default()
            },
            PreviewConfig {
                dir_budget: 2,
                ..PreviewConfig::default()
            },
        ] {
            let tree = build(&root, &cfg, &|| false);
            assert!(tree.summary.files + tree.summary.dirs <= 2);
            assert!(tree.summary.truncated);
        }
        let tree = build(&root, &PreviewConfig::default(), &|| true);
        assert!(tree.entries.is_empty());
        assert!(tree.notice.is_some());
        let tree = build(
            &root,
            &PreviewConfig {
                timeout_ms: 0,
                ..PreviewConfig::default()
            },
            &|| false,
        );
        assert!(tree.entries.is_empty());
        assert!(tree.summary.truncated);
    }

    #[test]
    fn cancellation_is_checked_during_discovery() {
        let f = Fixture::tree();
        let calls = std::cell::Cell::new(0);
        let tree = build(f.home(), &PreviewConfig::default(), &|| {
            calls.set(calls.get() + 1);
            calls.get() > 4
        });
        assert!(tree.summary.truncated);
        assert!(calls.get() > 4);
    }

    #[test]
    fn deep_trees_stop_at_four_levels() {
        let f = Fixture::tree();
        let root = f.path("empty");
        std::fs::create_dir_all(root.join("a/b/c/d/e/f")).unwrap();
        let tree = build(&root, &PreviewConfig::default(), &|| false);
        assert_eq!(tree.summary.dirs, 4);
        assert!(tree.summary.truncated);
        let deepest = &tree.entries[0].children[0].children[0].children[0];
        assert!(deepest.children.is_empty());
        assert_eq!(deepest.notice.as_deref(), Some("Depth limit"));
    }

    #[test]
    fn missing_directory_reports_an_error() {
        let f = Fixture::tree();
        let tree = build(&f.path("missing"), &PreviewConfig::default(), &|| false);
        assert!(tree.entries.is_empty());
        assert!(tree.notice.is_some());
        assert!(tree.summary.truncated);
    }

    proptest! {
        #[test]
        fn foreign_names_stay_on_one_terminal_row(name in ".{0,300}") {
            let shown = display_name(&name);
            prop_assert!(shown.chars().all(|c| !c.is_control()));
            prop_assert!(!shown.contains(['\u{202e}', '\u{2066}']), "directional control");
        }
    }
}
