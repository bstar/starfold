//! Trash recovery and session-only undo. All filesystem work runs on workers.
use super::ops::{progress::Progress, OpKind, Outcome, Plan};
use std::{
    fs,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp([u8; 32]);
impl Stamp {
    pub fn read(path: &Path) -> Result<Self, String> {
        let mut pending = vec![path.to_path_buf()];
        let mut items = Vec::new();
        while let Some(current) = pending.pop() {
            if items.len() >= 50_000 {
                return Err("Recovery verification exceeds 50,000 entries".into());
            }
            let meta = fs::symlink_metadata(&current)
                .map_err(|e| format!("{}: {e}", current.display()))?;
            let relative = current
                .strip_prefix(path)
                .map_err(|e| e.to_string())?
                .to_path_buf();
            if relative.components().count() > 64 {
                return Err("Recovery verification exceeds 64 directory levels".into());
            }
            items.push((
                relative,
                [
                    meta.dev() as i128,
                    meta.ino() as i128,
                    meta.len() as i128,
                    meta.mode() as i128,
                    meta.mtime() as i128,
                    meta.mtime_nsec() as i128,
                    meta.ctime() as i128,
                    meta.ctime_nsec() as i128,
                ],
            ));
            if meta.is_dir() {
                for entry in fs::read_dir(&current).map_err(|e| e.to_string())? {
                    if items.len() + pending.len() >= 50_000 {
                        return Err("Recovery verification exceeds 50,000 entries".into());
                    }
                    pending.push(entry.map_err(|e| e.to_string())?.path());
                }
            }
        }
        items.sort_by(|a, b| a.0.cmp(&b.0));
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        for (path, fields) in items {
            let bytes = path.as_os_str().as_bytes();
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
            for field in fields {
                hash.update(field.to_le_bytes());
            }
        }
        Ok(Self(hash.finalize().into()))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Trash,
    Undo,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub original_parent: Option<(u64, u64)>,
    pub mode: Mode,
    pub from: PathBuf,
    pub original: PathBuf,
    pub stamp: Stamp,
    pub cleanup: Option<PathBuf>,
}
fn parent_identity(original: &Path) -> Option<(u64, u64)> {
    let meta = fs::metadata(original.parent()?).ok()?;
    Some((meta.dev(), meta.ino()))
}
impl Record {
    fn validate_parent(&self) -> Result<(), String> {
        if self.original_parent.is_some() && parent_identity(&self.original) != self.original_parent
        {
            return Err(
                "The original directory changed or disappeared; recovery was refused.".into(),
            );
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct Item {
    pub record: Record,
    pub collision: bool,
    pub suggestion: String,
}
impl Item {
    fn new(record: Record) -> Self {
        let collision = fs::symlink_metadata(&record.original).is_ok();
        let suggestion = super::ops::exec::suggested_rename(&record.original);
        Self {
            record,
            collision,
            suggestion,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guard {
    pub record: Record,
}

/// Capture only an entirely successful, non-overwriting move or rename.
/// A merged or replaced destination cannot be reversed safely.
pub fn receipt(kind: OpKind, plan: &Plan, outcome: &Outcome) -> Vec<Record> {
    if !matches!(kind, OpKind::Move | OpKind::Rename)
        || outcome.cancelled
        || !outcome.failed.is_empty()
        || outcome.skipped != 0
        || !plan.conflicts.is_empty()
        || outcome.done != plan.sources.len()
    {
        return Vec::new();
    }
    plan.sources
        .iter()
        .zip(&plan.roots)
        .filter_map(|(original, root)| {
            let from = plan.items.get((*root)?)?.to.clone()?;
            let stamp = match Stamp::read(&from) {
                Ok(stamp) => stamp,
                Err(error) => {
                    tracing::warn!(%error, "operation completed but undo verification unavailable");
                    return None;
                }
            };
            Some(Record {
                original_parent: parent_identity(original),
                mode: Mode::Undo,
                from,
                original: original.clone(),
                stamp,
                cleanup: None,
            })
        })
        .collect()
}

/// Atomically refuse a destination that exists, including dangling symlinks.
fn rename_exclusive(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    let from = CString::new(from.as_os_str().as_bytes())?;
    let to = CString::new(to.as_os_str().as_bytes())?;
    // SAFETY: both C strings are valid and live for the duration of the syscall.
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(target_os = "macos")]
    let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn collision_error(error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        "The original name is occupied. Restore with another name.".into()
    } else {
        error.to_string()
    }
}

/// Across devices, stage a complete copy, revalidate the source, and publish
/// exclusively before removing the old item. A cleanup failure retains the
/// recovered copy and is reported rather than silently losing data.
fn recover_across_devices(
    record: &Record,
    target: &Path,
    progress: &Progress,
) -> Result<(), String> {
    let parent = target.parent().ok_or("No original directory")?;
    let stage = tempfile::Builder::new()
        .prefix(".starfold-recovery-")
        .tempdir_in(parent)
        .map_err(|e| e.to_string())?;
    let plan = super::ops::plan::plan(
        OpKind::Copy,
        std::slice::from_ref(&record.from),
        Some(stage.path()),
    )
    .map_err(|e| e.to_string())?;
    let options = super::ops::exec::RunOptions {
        preserve_times: true,
        force_copy: false,
    };
    let copied = super::ops::exec::run(
        OpKind::Copy,
        &plan,
        super::ops::ConflictPolicy::Ask,
        &options,
        progress,
    );
    if copied.cancelled {
        return Err("Recovery cancelled".into());
    }
    if let Some((_, error)) = copied.failed.first() {
        return Err(error.clone());
    }
    if copied.done != 1 {
        return Err("Recovery copy did not complete".into());
    }
    if Stamp::read(&record.from)? != record.stamp {
        return Err("The item changed while being recovered; recovery was refused.".into());
    }
    if progress.is_cancelled() {
        return Err("Recovery cancelled".into());
    }
    let staged = stage
        .path()
        .join(record.from.file_name().ok_or("No file name")?);
    record.validate_parent()?;
    rename_exclusive(&staged, target).map_err(collision_error)?;
    let metadata = fs::symlink_metadata(&record.from).map_err(|e| {
        format!(
            "Recovered copy is at {}; could not inspect the previous item: {e}",
            target.display()
        )
    })?;
    let removed = if metadata.is_dir() {
        fs::remove_dir_all(&record.from)
    } else {
        fs::remove_file(&record.from)
    };
    removed.map_err(|e| {
        format!(
            "Recovered copy is at {}; the previous item could not be fully removed: {e}",
            target.display()
        )
    })
}

pub fn run(guard: &Guard, target: &Path, progress: &Progress) -> Outcome {
    let record = &guard.record;
    let mut outcome = Outcome::default();
    progress.set_total(1);
    if progress.is_cancelled() {
        outcome.cancelled = true;
        return outcome;
    }
    let result = (|| {
        record.validate_parent()?;
        if target.parent() != record.original.parent() {
            return Err("Recovery must remain in the original directory".into());
        }
        if Stamp::read(&record.from)? != record.stamp {
            return Err(
                "The item changed since recovery was recorded; recovery was refused.".into(),
            );
        }
        let parent = target.parent().ok_or("No original directory")?;
        if !parent.is_dir() {
            return Err("The original directory is missing. Recreate it before restoring.".into());
        }
        if progress.is_cancelled() {
            return Err("Recovery cancelled".into());
        }
        match rename_exclusive(&record.from, target) {
            Ok(()) => Ok(()),
            Err(error) if error.raw_os_error() == Some(libc::EXDEV) => {
                recover_across_devices(record, target, progress)
            }
            Err(error) => Err(collision_error(error)),
        }
    })();
    match result {
        Ok(()) => {
            outcome.done = 1;
            progress.add(1);
            if let Some(info) = &record.cleanup {
                // The file is already restored; metadata cleanup is not a failed restore.
                if let Err(error) = fs::remove_file(info) {
                    tracing::warn!(path=%info.display(), %error, "recovery metadata cleanup failed");
                }
            }
        }
        Err(error) if error == "Recovery cancelled" && progress.is_cancelled() => {
            outcome.cancelled = true
        }
        Err(error) => outcome.failed.push((record.from.clone(), error)),
    }
    outcome
}

pub fn list(mode: Mode, undo: Vec<Record>, journal: Option<&Path>) -> Result<Vec<Item>, String> {
    let records = match mode {
        Mode::Undo => undo,
        Mode::Trash => list_trash(journal)?,
    };
    let mut items: Vec<_> = records
        .into_iter()
        .rev()
        .filter(|r| fs::symlink_metadata(&r.from).is_ok())
        .map(Item::new)
        .collect();
    if mode == Mode::Trash {
        items.sort_by(|a, b| a.record.original.cmp(&b.record.original));
    }
    Ok(items)
}

#[cfg(target_os = "linux")]
fn list_trash(_journal: Option<&Path>) -> Result<Vec<Record>, String> {
    let mut records = Vec::new();
    for item in trash::os_limited::list().map_err(|e| e.to_string())? {
        let info = PathBuf::from(&item.id);
        let Some(root) = info.parent().and_then(Path::parent) else {
            continue;
        };
        let Some(name) = info.file_stem() else {
            continue;
        };
        let from = root.join("files").join(name);
        if fs::symlink_metadata(&from).is_err() {
            continue;
        }
        records.push(Record {
            original_parent: parent_identity(&item.original_path()),
            mode: Mode::Trash,
            original: item.original_path(),
            stamp: Stamp::read(&from)?,
            from,
            cleanup: Some(info),
        });
        if records.len() >= 10_000 {
            return Err(
                "Trash contains more than 10,000 items; narrow it using the desktop Trash browser."
                    .into(),
            );
        }
    }
    Ok(records)
}

#[cfg(target_os = "macos")]
#[derive(serde::Serialize, serde::Deserialize)]
struct Journal {
    created: Option<(u64, u32)>,
    original: Vec<u8>,
    actual: Option<Vec<u8>>,
    dev: u64,
    ino: u64,
    roots: Vec<Vec<u8>>,
}
#[cfg(target_os = "macos")]
fn created(meta: &fs::Metadata) -> Option<(u64, u32)> {
    let when = meta
        .created()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    Some((when.as_secs(), when.subsec_nanos()))
}
#[cfg(target_os = "macos")]
fn path(bytes: Vec<u8>) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(bytes).into()
}
#[cfg(target_os = "macos")]
fn list_trash(journal: Option<&Path>) -> Result<Vec<Record>, String> {
    let Some(directory) = journal else {
        return Err("Trash recovery journal is unavailable".into());
    };
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    for (n, entry) in fs::read_dir(directory)
        .map_err(|e| e.to_string())?
        .enumerate()
    {
        if n >= 10_000 {
            return Err("Recovery journal exceeds 10,000 entries".into());
        }
        let info = entry.map_err(|e| e.to_string())?.path();
        if info.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        if fs::metadata(&info).map_err(|e| e.to_string())?.len() > 65_536 {
            return Err(format!("{}: recovery record is too large", info.display()));
        }
        let bytes = fs::read(&info).map_err(|e| e.to_string())?;
        let data: Journal =
            serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", info.display()))?;
        let from = match data.actual {
            Some(actual) => Some(path(actual)).filter(|p| fs::symlink_metadata(p).is_ok()),
            None => {
                let mut found = None;
                for root in data.roots.into_iter().map(path) {
                    let entries = match fs::read_dir(&root) {
                        Ok(entries) => entries,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(error) => return Err(format!("{}: {error}", root.display())),
                    };
                    for (index, entry) in entries.enumerate() {
                        if index >= 10_000 {
                            return Err(format!(
                                "{}: Trash search exceeds 10,000 entries",
                                root.display()
                            ));
                        }
                        let entry = entry.map_err(|error| error.to_string())?;
                        let meta = fs::symlink_metadata(entry.path())
                            .map_err(|error| error.to_string())?;
                        if meta.dev() == data.dev
                            && meta.ino() == data.ino
                            && created(&meta) == data.created
                        {
                            found = Some(entry.path());
                            break;
                        }
                    }
                    if found.is_some() {
                        break;
                    }
                }
                found
            }
        };
        if let Some(from) = from {
            let meta = fs::symlink_metadata(&from).map_err(|e| e.to_string())?;
            if meta.dev() != data.dev || meta.ino() != data.ino || created(&meta) != data.created {
                continue;
            }
            records.push(Record {
                original_parent: parent_identity(&path(data.original.clone())),
                mode: Mode::Trash,
                original: path(data.original),
                stamp: Stamp::read(&from)?,
                from,
                cleanup: Some(info),
            });
        }
    }
    Ok(records)
}

pub fn trash_one(source: &Path, journal: Option<&Path>) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let _ = journal;
        trash::delete(source).map_err(|e| e.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSFileManager, NSString, NSURL};
        use std::os::unix::fs::PermissionsExt;
        let directory = journal.ok_or("Trash recovery journal is unavailable")?;
        fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        let original = source
            .parent()
            .ok_or("No source directory")?
            .canonicalize()
            .map_err(|e| e.to_string())?
            .join(source.file_name().ok_or("No file name")?);
        let meta = fs::symlink_metadata(&original).map_err(|e| e.to_string())?;
        let mut mount = original.parent().unwrap().to_path_buf();
        while let Some(parent) = mount.parent() {
            if fs::metadata(parent).map_err(|e| e.to_string())?.dev() != meta.dev() {
                break;
            }
            mount = parent.to_path_buf();
        }
        let mut roots = vec![mount
            .join(".Trashes")
            .join(unsafe { libc::getuid() }.to_string())];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join(".Trash"));
        }
        let mut data = Journal {
            created: created(&meta),
            original: original.as_os_str().as_bytes().into(),
            actual: None,
            dev: meta.dev(),
            ino: meta.ino(),
            roots: roots
                .iter()
                .map(|p| p.as_os_str().as_bytes().into())
                .collect(),
        };
        let name = original
            .to_str()
            .ok_or("macOS Trash requires a UTF-8 path")?;
        let entry = tempfile::Builder::new()
            .prefix("trash-")
            .suffix(".json")
            .tempfile_in(directory)
            .map_err(|e| e.to_string())?;
        let (_, info) = entry.keep().map_err(|e| e.to_string())?;
        // Durable intent precedes the OS move, so an interrupted process can
        // recover the original location by locating the inode in the Trash.
        if let Err(error) = starkit::fs::write_private(
            &info,
            &serde_json::to_vec(&data).map_err(|e| e.to_string())?,
        ) {
            let _ = fs::remove_file(&info);
            return Err(error.to_string());
        }
        let url = NSURL::fileURLWithPath(&NSString::from_str(name));
        let mut resulting = None;
        if let Err(error) = NSFileManager::defaultManager()
            .trashItemAtURL_resultingItemURL_error(&url, Some(&mut resulting))
        {
            let _ = fs::remove_file(&info);
            return Err(error.to_string());
        }
        if let Some(actual) = resulting.and_then(|u| u.path()) {
            let actual_path = PathBuf::from(actual.to_string());
            if let Ok(meta) = fs::symlink_metadata(&actual_path) {
                data.dev = meta.dev();
                data.ino = meta.ino();
                data.created = created(&meta);
            }
            data.actual = Some(actual.to_string().into_bytes());
            if let Err(error) = starkit::fs::write_private(
                &info,
                &serde_json::to_vec(&data).map_err(|e| e.to_string())?,
            ) {
                tracing::warn!(%error, "Trash moved; durable intent retained for recovery");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(from: &Path, original: &Path, mode: Mode) -> Record {
        Record {
            original_parent: parent_identity(original),
            mode,
            from: from.into(),
            original: original.into(),
            stamp: Stamp::read(from).unwrap(),
            cleanup: None,
        }
    }
    #[test]
    fn restores_a_file_and_removes_only_its_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("trashed");
        let original = dir.path().join("original");
        let info = dir.path().join("trashed.trashinfo");
        fs::write(&from, b"recover me").unwrap();
        fs::write(&info, b"metadata").unwrap();
        let mut r = record(&from, &original, Mode::Trash);
        r.cleanup = Some(info.clone());
        let result = run(&Guard { record: r }, &original, &Progress::new(0));
        assert_eq!(result.done, 1);
        assert!(result.failed.is_empty());
        assert_eq!(fs::read(&original).unwrap(), b"recover me");
        assert!(!from.exists());
        assert!(!info.exists());
    }
    #[test]
    fn restore_collision_never_overwrites_and_can_use_another_name() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("trashed");
        let original = dir.path().join("file.txt");
        fs::write(&from, b"old").unwrap();
        fs::write(&original, b"new").unwrap();
        let r = record(&from, &original, Mode::Trash);
        let item = Item::new(r.clone());
        assert!(item.collision);
        assert_eq!(item.suggestion, "file (1).txt");
        let guard = Guard { record: r };
        assert_eq!(run(&guard, &original, &Progress::new(0)).failed.len(), 1);
        assert_eq!(fs::read(&original).unwrap(), b"new");
        assert_eq!(fs::read(&from).unwrap(), b"old");
        let alternate = dir.path().join(item.suggestion);
        assert_eq!(run(&guard, &alternate, &Progress::new(0)).done, 1);
    }
    #[test]
    fn undo_refuses_a_changed_file_and_keeps_both_names_safe() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("renamed");
        let original = dir.path().join("original");
        fs::write(&from, b"before").unwrap();
        let r = record(&from, &original, Mode::Undo);
        fs::write(&from, b"edited after rename").unwrap();
        let outcome = run(&Guard { record: r }, &original, &Progress::new(0));
        assert!(outcome.failed[0].1.contains("changed"));
        assert!(from.exists());
        assert!(!original.exists());
    }
    #[test]
    fn undo_detects_nested_edits_without_following_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("moved");
        let original = dir.path().join("original");
        fs::create_dir(&from).unwrap();
        fs::write(from.join("child"), b"before").unwrap();
        std::os::unix::fs::symlink(".", from.join("loop")).unwrap();
        let r = record(&from, &original, Mode::Undo);
        fs::write(from.join("child"), b"after").unwrap();
        assert_eq!(
            run(&Guard { record: r }, &original, &Progress::new(0))
                .failed
                .len(),
            1
        );
        assert!(from.exists());
    }
    #[test]
    fn undo_refuses_a_replaced_original_directory() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("original-parent");
        fs::create_dir(&parent).unwrap();
        let original = parent.join("file");
        let from = dir.path().join("moved");
        fs::write(&from, b"safe").unwrap();
        let r = record(&from, &original, Mode::Undo);
        fs::rename(&parent, dir.path().join("old-parent")).unwrap();
        fs::create_dir(&parent).unwrap();
        assert_eq!(
            run(&Guard { record: r }, &original, &Progress::new(0))
                .failed
                .len(),
            1
        );
        assert!(from.exists());
        assert!(!original.exists());
    }

    #[test]
    fn cancellation_and_missing_parent_leave_trash_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("trashed");
        let original = dir.path().join("missing/file");
        fs::write(&from, b"safe").unwrap();
        let guard = Guard {
            record: record(&from, &original, Mode::Trash),
        };
        let progress = Progress::new(0);
        progress.cancel();
        assert!(run(&guard, &original, &progress).cancelled);
        assert!(from.exists());
        assert_eq!(run(&guard, &original, &Progress::new(0)).failed.len(), 1);
        assert!(from.exists());
    }
    #[test]
    fn recovery_refuses_destinations_outside_the_original_directory() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("trashed");
        let original = dir.path().join("original");
        fs::write(&from, b"safe").unwrap();
        let guard = Guard {
            record: record(&from, &original, Mode::Trash),
        };
        assert_eq!(
            run(&guard, &dir.path().join("other/file"), &Progress::new(0))
                .failed
                .len(),
            1
        );
        assert!(from.exists());
    }
    #[test]
    fn dangling_symlink_at_original_name_counts_as_a_collision() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("trashed");
        let original = dir.path().join("original");
        fs::write(&from, b"safe").unwrap();
        std::os::unix::fs::symlink("missing", &original).unwrap();
        let r = record(&from, &original, Mode::Trash);
        assert!(Item::new(r.clone()).collision);
        assert_eq!(
            run(&Guard { record: r }, &original, &Progress::new(0))
                .failed
                .len(),
            1
        );
        assert_eq!(fs::read_link(&original).unwrap(), Path::new("missing"));
        assert!(from.exists());
    }
    #[test]
    fn failed_or_overwriting_operations_are_not_undoable() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("file");
        let target = dir.path().join("renamed");
        fs::write(&source, b"safe").unwrap();
        fs::write(&target, b"existing").unwrap();
        let plan = super::super::ops::plan::plan(
            OpKind::Rename,
            std::slice::from_ref(&source),
            Some(&target),
        )
        .unwrap();
        assert!(receipt(
            OpKind::Rename,
            &plan,
            &Outcome {
                done: 1,
                ..Default::default()
            }
        )
        .is_empty());
        assert!(receipt(
            OpKind::Rename,
            &plan,
            &Outcome {
                failed: vec![(source, "failure".into())],
                ..Default::default()
            }
        )
        .is_empty());
    }
    #[test]
    fn staged_recovery_copies_metadata_and_never_overwrites_a_collision() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("trashed");
        let original = dir.path().join("original");
        fs::write(&from, b"recover across devices").unwrap();
        fs::set_permissions(&from, fs::Permissions::from_mode(0o640)).unwrap();
        let r = record(&from, &original, Mode::Undo);
        recover_across_devices(&r, &original, &Progress::new(0)).unwrap();
        assert!(!from.exists());
        assert_eq!(fs::read(&original).unwrap(), b"recover across devices");
        assert_eq!(fs::metadata(&original).unwrap().mode() & 0o777, 0o640);
        fs::write(&from, b"another item").unwrap();
        let r = record(&from, &original, Mode::Undo);
        assert!(recover_across_devices(&r, &original, &Progress::new(0)).is_err());
        assert_eq!(fs::read(&from).unwrap(), b"another item");
        assert_eq!(fs::read(&original).unwrap(), b"recover across devices");
        assert!(!fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| e
                .file_name()
                .to_string_lossy()
                .starts_with(".starfold-recovery-")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn private_trash_survives_a_process_restart() {
        let dir = tempfile::tempdir().unwrap();
        for phase in ["trash", "restore"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "fold::recovery::tests::private_trash_helper",
                    "--nocapture",
                ])
                .env("STARFOLD_PRIVATE_TRASH_TEST", dir.path())
                .env("STARFOLD_PRIVATE_TRASH_PHASE", phase)
                .env("HOME", dir.path())
                .env("XDG_DATA_HOME", dir.path().join("data"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
        assert_eq!(
            fs::read(dir.path().join("original.txt")).unwrap(),
            b"keep existing"
        );
        assert_eq!(
            fs::read(dir.path().join("original (1).txt")).unwrap(),
            b"trashed content"
        );
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn private_trash_helper() {
        let Some(root) = std::env::var_os("STARFOLD_PRIVATE_TRASH_TEST") else {
            return;
        };
        let root = PathBuf::from(root);
        let original = root.join("original.txt");
        if std::env::var("STARFOLD_PRIVATE_TRASH_PHASE").unwrap() == "trash" {
            fs::write(&original, b"trashed content").unwrap();
            trash_one(&original, None).unwrap();
            assert!(!original.exists());
        } else {
            fs::write(&original, b"keep existing").unwrap();
            let item = list(Mode::Trash, vec![], None)
                .unwrap()
                .into_iter()
                .find(|item| item.record.original == original)
                .unwrap();
            assert!(item.collision);
            assert_eq!(item.suggestion, "original (1).txt");
            let target = root.join(&item.suggestion);
            assert_eq!(
                run(
                    &Guard {
                        record: item.record
                    },
                    &target,
                    &Progress::new(0)
                )
                .done,
                1
            );
        }
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_durable_intent_recovers_after_restart_with_a_fake_trash_adapter() {
        let dir = tempfile::tempdir().unwrap();
        let journal = dir.path().join("journal");
        let bin = dir.path().join("fake-trash");
        fs::create_dir(&journal).unwrap();
        fs::create_dir(&bin).unwrap();
        let original = dir.path().join("original");
        let actual = bin.join("renamed-by-os");
        fs::write(&original, b"recover me").unwrap();
        let meta = fs::metadata(&original).unwrap();
        let data = Journal {
            created: created(&meta),
            original: original.as_os_str().as_bytes().into(),
            actual: None,
            dev: meta.dev(),
            ino: meta.ino(),
            roots: vec![bin.as_os_str().as_bytes().into()],
        };
        let info = journal.join("record.json");
        fs::write(&info, serde_json::to_vec(&data).unwrap()).unwrap();
        fs::rename(&original, &actual).unwrap();
        let items = list(Mode::Trash, vec![], Some(&journal)).unwrap();
        assert_eq!(items.len(), 1);
        drop(items);
        let restored = list(Mode::Trash, vec![], Some(&journal)).unwrap().remove(0);
        assert_eq!(
            run(
                &Guard {
                    record: restored.record
                },
                &original,
                &Progress::new(0)
            )
            .done,
            1
        );
        assert_eq!(fs::read(&original).unwrap(), b"recover me");
        assert!(!info.exists());
    }

    proptest::proptest! {
        #[test]
        fn exclusive_restore_preserves_any_existing_content(old in proptest::collection::vec(proptest::prelude::any::<u8>(),0..256), current in proptest::collection::vec(proptest::prelude::any::<u8>(),0..256)) {
            let dir=tempfile::tempdir().unwrap(); let from=dir.path().join("trashed"); let original=dir.path().join("original");
            fs::write(&from,&old).unwrap(); fs::write(&original,&current).unwrap();
            let guard=Guard {record:record(&from,&original,Mode::Trash)};
            proptest::prop_assert_eq!(run(&guard,&original,&Progress::new(0)).failed.len(),1);
            proptest::prop_assert_eq!(fs::read(&from).unwrap(),old);
            proptest::prop_assert_eq!(fs::read(&original).unwrap(),current);
        }
    }
}
