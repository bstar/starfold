//! Bounded, cancellable filename and content search. The scan runs on the IO worker;
//! progress counters are atomic so the UI can report work without taking a
//! lock or waiting for a directory read to finish.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use super::entry::{self, Entry};

const MAX_VISITED: usize = 100_000;
const MAX_RESULTS: usize = 5_000;
const MAX_DEPTH: usize = 64;
const MAX_ERRORS: usize = 4;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Names,
    Contents,
}

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
    Partial,
}

#[derive(Debug)]
pub struct Search {
    pub generation: u64,
    pub root: PathBuf,
    pub query: String,
    pub mode: Mode,
    pub include_hidden: bool,
    pub results: Vec<Entry>,
    pub identities: HashMap<PathBuf, Identity>,
    pub excerpts: HashMap<PathBuf, String>,
    pub skipped_binary: usize,
    pub skipped_large: usize,
    pub cursor: usize,
    pub status: Status,
    pub errors: Vec<String>,
    pub progress: Arc<Progress>,
}

#[derive(Debug)]
pub struct Found {
    pub entries: Vec<Entry>,
    pub identities: HashMap<PathBuf, Identity>,
    pub excerpts: HashMap<PathBuf, String>,
    pub skipped_binary: usize,
    pub skipped_large: usize,
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
    scan_mode(root, query, Mode::Names, include_hidden, progress)
}

pub fn scan_mode(
    root: &Path,
    query: &str,
    mode: Mode,
    include_hidden: bool,
    progress: &Progress,
) -> Found {
    let needle = query.to_lowercase();
    let mut entries = Vec::new();
    let mut identities = HashMap::new();
    let mut excerpts = HashMap::new();
    let mut skipped_binary = 0;
    let mut skipped_large = 0;
    let mut bytes_read = 0;
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
            let matched = match mode {
                Mode::Names => name
                    .to_lowercase()
                    .contains(&needle)
                    .then_some((None, None)),
                Mode::Contents if file_type.is_file() => {
                    let size = match child.metadata() {
                        Ok(meta) => meta.len(),
                        Err(error) => {
                            note_error(&mut errors, &path, &error);
                            continue;
                        }
                    };
                    if size > MAX_FILE_BYTES {
                        skipped_large += 1;
                        continue;
                    }
                    if size > MAX_TOTAL_BYTES.saturating_sub(bytes_read) {
                        status = Status::Limited;
                        break;
                    }
                    match content_match(&path, &needle, progress) {
                        Ok(ContentMatch::Match(excerpt, read, identity)) => {
                            bytes_read += read;
                            Some((Some(excerpt), Some(identity)))
                        }
                        Ok(ContentMatch::Miss(read)) => {
                            bytes_read += read;
                            None
                        }
                        Ok(ContentMatch::Binary(read)) => {
                            bytes_read += read;
                            skipped_binary += 1;
                            None
                        }
                        Ok(ContentMatch::Large(read)) => {
                            bytes_read += read;
                            skipped_large += 1;
                            None
                        }
                        Ok(ContentMatch::Cancelled) => {
                            status = Status::Cancelled;
                            break;
                        }
                        Err(error) => {
                            note_error(&mut errors, &path, &error);
                            None
                        }
                    }
                }
                Mode::Contents => None,
            };
            if let Some((excerpt, matched_identity)) = matched {
                let identity = if let Some(identity) = matched_identity {
                    identity
                } else {
                    match Identity::of(&path) {
                        Ok(identity) => identity,
                        Err(error) => {
                            note_error(&mut errors, &path, &error);
                            continue;
                        }
                    }
                };
                let mut result = entry::stat(&path);
                result.display = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                identities.insert(path.clone(), identity);
                if let Some(excerpt) = excerpt {
                    excerpts.insert(path.clone(), excerpt);
                }
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
    if mode == Mode::Contents
        && status == Status::Complete
        && (skipped_large > 0 || !errors.is_empty())
    {
        status = Status::Partial;
    }
    Found {
        entries,
        identities,
        excerpts,
        skipped_binary,
        skipped_large,
        status,
        errors,
    }
}

enum ContentMatch {
    Match(String, u64, Identity),
    Miss(u64),
    Binary(u64),
    Large(u64),
    Cancelled,
}

fn content_match(path: &Path, needle: &str, progress: &Progress) -> std::io::Result<ContentMatch> {
    let mut file = fs::File::open(path)?;
    // A path replaced with a symlink between directory enumeration and open
    // must not make the scan read its target.
    let opened = file.metadata()?;
    let named = fs::symlink_metadata(path)?;
    if !named.is_file() || (opened.dev(), opened.ino()) != (named.dev(), named.ino()) {
        return Ok(ContentMatch::Miss(0));
    }
    let identity = Identity {
        dev: opened.dev(),
        ino: opened.ino(),
    };
    let mut bytes = Vec::with_capacity(opened.len().min(MAX_FILE_BYTES) as usize);
    let mut chunk = [0u8; 8192];
    loop {
        if progress.cancel.load(Ordering::Relaxed) {
            return Ok(ContentMatch::Cancelled);
        }
        let count = file.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        if bytes.len() + count > MAX_FILE_BYTES as usize {
            return Ok(ContentMatch::Large((bytes.len() + count) as u64));
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    let read = bytes.len() as u64;
    if bytes.contains(&0) {
        return Ok(ContentMatch::Binary(read));
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(ContentMatch::Binary(read));
    };
    for (index, line) in text.lines().enumerate() {
        if progress.cancel.load(Ordering::Relaxed) {
            return Ok(ContentMatch::Cancelled);
        }
        if line.to_lowercase().contains(needle) {
            let excerpt: String = line.chars().filter(|c| !c.is_control()).take(100).collect();
            return Ok(ContentMatch::Match(
                format!("line {}: {excerpt}", index + 1),
                read,
                identity,
            ));
        }
    }
    Ok(ContentMatch::Miss(read))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn arbitrary_file_bytes_are_bounded_and_only_utf8_text_is_searched(
            bytes in proptest::collection::vec(any::<u8>(), 0..4096),
        ) {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("input");
            fs::write(&path, &bytes).unwrap();
            let result = content_match(&path, "needle", &Progress::default()).unwrap();
            let binary = bytes.contains(&0) || std::str::from_utf8(&bytes).is_err();
            if binary {
                prop_assert!(matches!(&result, ContentMatch::Binary(_)));
            } else {
                prop_assert!(matches!(&result, ContentMatch::Miss(_) | ContentMatch::Match(_, _, _)));
            }
            let read = match result {
                ContentMatch::Match(_, n, _) | ContentMatch::Miss(n) | ContentMatch::Binary(n)
                | ContentMatch::Large(n) => n,
                ContentMatch::Cancelled => 0,
            };
            prop_assert!(read <= MAX_FILE_BYTES);
        }
    }

    #[test]
    fn content_search_reports_excerpts_and_skips_binary_large_and_hidden_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("nested/.hidden")).unwrap();
        let match_path = dir.path().join("nested/note.txt");
        fs::write(&match_path, "first line\nFind THIS phrase\nlast line\n").unwrap();
        fs::write(dir.path().join("nested/blob.bin"), b"find\0this").unwrap();
        fs::write(dir.path().join("nested/.hidden/secret.txt"), "find this").unwrap();
        fs::File::create(dir.path().join("nested/large.txt"))
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&match_path, dir.path().join("nested/link.txt")).unwrap();

        let found = scan_mode(
            dir.path(),
            "FIND THIS",
            Mode::Contents,
            false,
            &Progress::default(),
        );
        assert_eq!(found.status, Status::Partial);
        assert_eq!(found.entries.len(), 1);
        assert_eq!(found.entries[0].path, match_path);
        assert_eq!(found.excerpts[&match_path], "line 2: Find THIS phrase");
        assert_eq!(found.skipped_binary, 1);
        assert_eq!(found.skipped_large, 1);
        assert_eq!(
            found.identities[&match_path],
            Identity::of(&match_path).unwrap()
        );

        let with_hidden = scan_mode(
            dir.path(),
            "find this",
            Mode::Contents,
            true,
            &Progress::default(),
        );
        assert_eq!(with_hidden.entries.len(), 2);
        let cancelled = Progress::default();
        cancelled.cancel.store(true, Ordering::Relaxed);
        assert_eq!(
            scan_mode(dir.path(), "find", Mode::Contents, true, &cancelled).status,
            Status::Cancelled
        );
    }

    #[test]
    fn content_search_labels_total_work_limit() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..=MAX_TOTAL_BYTES / MAX_FILE_BYTES {
            fs::File::create(dir.path().join(format!("file-{index:03}.txt")))
                .unwrap()
                .set_len(MAX_FILE_BYTES)
                .unwrap();
        }
        let found = scan_mode(
            dir.path(),
            "needle",
            Mode::Contents,
            false,
            &Progress::default(),
        );
        assert_eq!(found.status, Status::Limited);
        assert!(found.entries.is_empty());
    }

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
