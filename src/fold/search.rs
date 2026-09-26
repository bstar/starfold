//! Bounded, cancellable filename search. The scan runs on the IO worker;
//! progress counters are atomic so the UI can report work without taking a
//! lock or waiting for a directory read to finish.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use super::entry::{self, Entry};

const MAX_VISITED: usize = 100_000;
const MAX_RESULTS: usize = 5_000;
const MAX_DEPTH: usize = 64;
const MAX_ERRORS: usize = 4;

#[derive(Debug, Default)]
pub struct Progress {
    pub cancel: AtomicBool,
    pub visited: AtomicUsize,
    pub directories: AtomicUsize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Running,
    Complete,
    Cancelled,
    Limited,
}

#[derive(Debug)]
pub struct Search {
    pub generation: u64,
    pub root: PathBuf,
    pub query: String,
    pub include_hidden: bool,
    pub results: Vec<Entry>,
    pub identities: HashMap<PathBuf, Identity>,
    pub cursor: usize,
    pub status: Status,
    pub errors: Vec<String>,
    pub progress: Arc<Progress>,
}

#[derive(Debug)]
pub struct Found {
    pub entries: Vec<Entry>,
    pub identities: HashMap<PathBuf, Identity>,
    pub status: Status,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    pub dev: u64,
    pub ino: u64,
}

impl Identity {
    pub fn of(path: &Path) -> std::io::Result<Self> {
        let meta = fs::symlink_metadata(path)?;
        Ok(Self {
            dev: meta.dev(),
            ino: meta.ino(),
        })
    }
}

fn note_error(errors: &mut Vec<String>, path: &Path, error: &std::io::Error) {
    if errors.len() < MAX_ERRORS {
        errors.push(format!("{}: {error}", path.display()));
    }
}

/// Search names, never descending through a symlink. A visited inode set also
/// stops bind-mount loops; depth and entry limits bound adversarial trees.
pub fn scan(root: &Path, query: &str, include_hidden: bool, progress: &Progress) -> Found {
    let needle = query.to_lowercase();
    let mut entries = Vec::new();
    let mut identities = HashMap::new();
    let mut errors = Vec::new();
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    let mut seen = HashSet::new();
    let mut status = Status::Complete;

    while let Some((dir, depth)) = pending.pop() {
        if progress.cancel.load(Ordering::Relaxed) {
            status = Status::Cancelled;
            break;
        }
        if depth > MAX_DEPTH {
            status = Status::Limited;
            continue;
        }
        // The chosen root may itself be a directory symlink the user entered.
        // Descendants still use no-follow metadata.
        let meta = match if depth == 0 {
            fs::metadata(&dir)
        } else {
            fs::symlink_metadata(&dir)
        } {
            Ok(meta) if meta.is_dir() => meta,
            Ok(_) => {
                note_error(&mut errors, &dir, &std::io::Error::other("not a directory"));
                continue;
            }
            Err(error) => {
                note_error(&mut errors, &dir, &error);
                continue;
            }
        };
        if !seen.insert((meta.dev(), meta.ino())) {
            continue;
        }
        progress.directories.fetch_add(1, Ordering::Relaxed);
        let children = match fs::read_dir(&dir) {
            Ok(children) => children,
            Err(error) => {
                note_error(&mut errors, &dir, &error);
                continue;
            }
        };
        for child in children {
            if progress.cancel.load(Ordering::Relaxed) {
                status = Status::Cancelled;
                break;
            }
            let child = match child {
                Ok(child) => child,
                Err(error) => {
                    note_error(&mut errors, &dir, &error);
                    continue;
                }
            };
            let name = child.file_name().to_string_lossy().into_owned();
            if !include_hidden && name.starts_with('.') {
                continue;
            }
            if progress.visited.fetch_add(1, Ordering::Relaxed) >= MAX_VISITED {
                status = Status::Limited;
                break;
            }
            let path = child.path();
            let file_type = match child.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    note_error(&mut errors, &path, &error);
                    continue;
                }
            };
            if name.to_lowercase().contains(&needle) {
                let identity = match Identity::of(&path) {
                    Ok(identity) => identity,
                    Err(error) => {
                        note_error(&mut errors, &path, &error);
                        continue;
                    }
                };
                let mut result = entry::stat(&path);
                result.display = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                identities.insert(path.clone(), identity);
                entries.push(result);
                if entries.len() >= MAX_RESULTS {
                    status = Status::Limited;
                    break;
                }
            }
            if file_type.is_dir() {
                pending.push((path, depth + 1));
            }
        }
        if status != Status::Complete {
            break;
        }
    }
    entries.sort_by_key(|entry| entry.display.to_lowercase());
    Found {
        entries,
        identities,
        status,
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_names_hidden_paths_symlink_loops_and_cancel() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("nested/.hidden")).unwrap();
        fs::write(dir.path().join("nested/needle.txt"), b"ok").unwrap();
        fs::write(dir.path().join("nested/.hidden/needle.txt"), b"hidden").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path(), dir.path().join("nested/loop")).unwrap();
        let progress = Progress::default();
        let visible = scan(dir.path(), "NEEDLE", false, &progress);
        assert_eq!(visible.status, Status::Complete);
        assert_eq!(visible.entries.len(), 1);
        assert_eq!(visible.entries[0].display, "nested/needle.txt");
        assert!(progress.visited.load(Ordering::Relaxed) < 10);
        let hidden = scan(dir.path(), "needle", true, &Progress::default());
        assert_eq!(hidden.entries.len(), 2);
        let cancelled = Progress::default();
        cancelled.cancel.store(true, Ordering::Relaxed);
        assert_eq!(
            scan(dir.path(), "needle", true, &cancelled).status,
            Status::Cancelled
        );
        let missing = scan(
            &dir.path().join("gone"),
            "needle",
            true,
            &Progress::default(),
        );
        assert_eq!(missing.status, Status::Complete);
        assert!(!missing.errors.is_empty());
        #[cfg(unix)]
        {
            let link = dir.path().join("root-link");
            std::os::unix::fs::symlink(dir.path().join("nested"), &link).unwrap();
            let via_link = scan(&link, "needle", false, &Progress::default());
            assert_eq!(via_link.entries.len(), 1);
        }
    }
}
