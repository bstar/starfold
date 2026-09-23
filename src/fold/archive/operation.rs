//! Transactional archive jobs. Planning captures sources; execution writes
//! only private staging paths, then publishes through conflict policy.
use super::{Format, MAX_ENTRIES};
use crate::fold::ops::{
    progress::Progress, Conflict, ConflictPolicy, Item, ItemKind, OpKind, Outcome, Plan,
};
use std::{
    fs,
    path::{Path, PathBuf},
};
pub fn plan(kind: OpKind, sources: &[PathBuf], dest: &Path) -> anyhow::Result<Plan> {
    anyhow::ensure!(!sources.is_empty(), "No archive sources");
    let parent = dest
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Destination needs a parent directory"))?;
    let parent = fs::canonicalize(parent)?;
    let target = parent.join(
        dest.file_name()
            .ok_or_else(|| anyhow::anyhow!("Destination needs a filename"))?,
    );
    let mut p = Plan {
        sources: sources.to_vec(),
        dest: target.clone(),
        ..Plan::default()
    };
    for source in sources {
        let canonical = fs::canonicalize(source)?;
        anyhow::ensure!(
            canonical != target && !parent.starts_with(&canonical),
            "Archive destination is inside its source"
        );
    }
    if fs::symlink_metadata(&target).is_ok() {
        p.conflicts.push(Conflict {
            source: sources[0].clone(),
            dest: target,
            both_dirs: false,
        });
    }
    match kind {
        OpKind::Compress(format) => {
            anyhow::ensure!(format.writable(), "Read-only archive format");
            let mut names = std::collections::HashSet::new();
            for source in sources {
                let name = PathBuf::from(
                    source
                        .file_name()
                        .ok_or_else(|| anyhow::anyhow!("Cannot archive a filesystem root"))?,
                );
                anyhow::ensure!(names.insert(name.clone()), "Duplicate source filenames");
                walk(source, &name, &mut p, 0)?;
            }
        }
        OpKind::Extract => {
            anyhow::ensure!(sources.len() == 1, "Extract one archive per operation");
            anyhow::ensure!(
                Format::from_path(&sources[0]).is_some(),
                "Unsupported archive format"
            );
            // Header enumeration is deliberately deferred to staged execution;
            // a compressed tar cannot enumerate without decompression.
            p.total_bytes = 0;
        }
        _ => anyhow::bail!("Not an archive operation"),
    }
    p.total_items = p.items.len();
    Ok(p)
}
fn walk(path: &Path, name: &Path, p: &mut Plan, depth: usize) -> anyhow::Result<()> {
    anyhow::ensure!(
        depth <= 64 && p.items.len() < MAX_ENTRIES,
        "Archive source tree exceeds limits"
    );
    let m = fs::symlink_metadata(path)?;
    let kind = if m.is_dir() {
        ItemKind::Dir
    } else {
        anyhow::ensure!(
            m.is_file(),
            "Archive links and special files are unsupported"
        );
        p.total_bytes = p
            .total_bytes
            .checked_add(m.len())
            .ok_or_else(|| anyhow::anyhow!("Archive size overflow"))?;
        ItemKind::File(m.len())
    };
    p.items.push(Item {
        from: path.into(),
        to: Some(name.into()),
        kind,
    });
    if m.is_dir() {
        let mut entries = fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            walk(&e.path(), &name.join(e.file_name()), p, depth + 1)?;
        }
    }
    Ok(())
}
pub fn run(kind: OpKind, p: &Plan, policy: ConflictPolicy, progress: &Progress) -> Outcome {
    let result = (|| -> anyhow::Result<bool> {
        anyhow::ensure!(!progress.is_cancelled(), "cancelled");
        let mut dest = p.dest.clone();
        if fs::symlink_metadata(&dest).is_ok() {
            match policy {
                ConflictPolicy::Skip => return Ok(false),
                ConflictPolicy::Ask => {
                    anyhow::bail!("Archive destination already exists; choose a conflict policy")
                }
                ConflictPolicy::RenameNew => {
                    let name = dest.file_name().unwrap().to_string_lossy().into_owned();
                    for n in 1u64.. {
                        let candidate = dest.with_file_name(format!("{name} ({n})"));
                        if fs::symlink_metadata(&candidate).is_err() {
                            dest = candidate;
                            break;
                        }
                    }
                }
                ConflictPolicy::Overwrite => {}
            }
        }
        let parent = dest.parent().unwrap();
        let stage = tempfile::Builder::new()
            .prefix(".starfold-archive-")
            .tempdir_in(parent)?;
        let payload = stage.path().join("payload");
        super::connection::run(kind, &payload, p, progress)?;
        anyhow::ensure!(!progress.is_cancelled(), "cancelled");
        // Recheck after potentially long compression. Preserve the old target
        // until a successful publish, including rollback on rename failure.
        let backup = stage.path().join("previous");
        let exists = fs::symlink_metadata(&dest).is_ok();
        if exists {
            anyhow::ensure!(
                policy == ConflictPolicy::Overwrite,
                "Destination appeared while operation was running"
            );
            fs::rename(&dest, &backup)?;
        }
        if let Err(e) = publish(&payload, &dest) {
            if exists {
                if let Err(restore) = fs::rename(&backup, &dest) {
                    let recovery = stage.keep();
                    anyhow::bail!(
                        "Publish failed: {e}; restore failed: {restore}; original retained at {}",
                        recovery.display()
                    );
                }
            }
            return Err(e.into());
        }
        Ok(true)
    })();
    match result {
        Ok(true) => Outcome {
            done: p.sources.len(),
            ..Outcome::default()
        },
        Ok(false) => Outcome {
            skipped: p.sources.len(),
            ..Outcome::default()
        },
        Err(e) => Outcome {
            failed: if progress.is_cancelled() {
                vec![]
            } else {
                p.sources
                    .iter()
                    .map(|source| (source.clone(), e.to_string()))
                    .collect()
            },
            cancelled: progress.is_cancelled(),
            ..Outcome::default()
        },
    }
}
/// Publish without replacing a name that appeared after the conflict check.
fn publish(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let from = std::ffi::CString::new(from.as_os_str().as_bytes())?;
    let to = std::ffi::CString::new(to.as_os_str().as_bytes())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_writable_format_round_trips_a_tree() {
        for (format, extension) in [
            (Format::Zip, "zip"),
            (Format::Tar, "tar"),
            (Format::TarGz, "tar.gz"),
            (Format::TarZst, "tar.zst"),
            (Format::SevenZip, "7z"),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let source = tmp.path().join("source");
            fs::create_dir(&source).unwrap();
            fs::write(source.join("hello.txt"), b"Hello from Starfold\n").unwrap();
            fs::create_dir(source.join("empty")).unwrap();
            let archive = tmp.path().join(format!("out.{extension}"));
            let p = plan(OpKind::Compress(format), &[source], &archive).unwrap();
            let out = run(
                OpKind::Compress(format),
                &p,
                ConflictPolicy::Ask,
                &Progress::default(),
            );
            assert!(out.failed.is_empty(), "{extension}: {out:?}");
            let (entries, partial) = super::super::list(&archive, 400).unwrap();
            assert!(!partial);
            assert!(entries.iter().any(|e| e.name == "source/hello.txt"));
            let dest = tmp.path().join("extracted");
            let p = plan(OpKind::Extract, &[archive], &dest).unwrap();
            let out = run(
                OpKind::Extract,
                &p,
                ConflictPolicy::Ask,
                &Progress::default(),
            );
            assert!(out.failed.is_empty(), "{extension}: {out:?}");
            assert_eq!(
                fs::read(dest.join("source/hello.txt")).unwrap(),
                b"Hello from Starfold\n"
            );
            assert!(dest.join("source/empty").is_dir());
        }
    }
    #[test]
    fn failed_extraction_preserves_existing_destination_and_cleans_staging() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("bad.zip");
        fs::write(&archive, b"broken").unwrap();
        let dest = tmp.path().join("destination");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("keep"), b"original").unwrap();
        let p = plan(OpKind::Extract, &[archive], &dest).unwrap();
        let out = run(
            OpKind::Extract,
            &p,
            ConflictPolicy::Overwrite,
            &Progress::default(),
        );
        assert!(!out.failed.is_empty());
        assert_eq!(fs::read(dest.join("keep")).unwrap(), b"original");
        assert!(!fs::read_dir(tmp.path()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".starfold-archive-")));
    }
    #[test]
    fn cancelled_compression_publishes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("file");
        fs::write(&src, b"data").unwrap();
        let dest = tmp.path().join("out.zip");
        let p = plan(OpKind::Compress(Format::Zip), &[src], &dest).unwrap();
        let progress = Progress::default();
        progress.cancel();
        assert!(
            run(
                OpKind::Compress(Format::Zip),
                &p,
                ConflictPolicy::Ask,
                &progress
            )
            .cancelled
        );
        assert!(!dest.exists());
    }
    #[test]
    fn traversal_archive_never_publishes_or_escapes() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("attack.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        zip.start_file("../escape", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"bad").unwrap();
        zip.finish().unwrap();
        let dest = tmp.path().join("output");
        let p = plan(OpKind::Extract, &[archive], &dest).unwrap();
        assert!(!run(
            OpKind::Extract,
            &p,
            ConflictPolicy::Ask,
            &Progress::default()
        )
        .failed
        .is_empty());
        assert!(!dest.exists());
        assert!(!tmp.path().join("escape").exists());
    }
    #[test]
    fn source_tree_cannot_contain_destination_or_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("source");
        fs::create_dir(&dir).unwrap();
        assert!(plan(
            OpKind::Compress(Format::Zip),
            std::slice::from_ref(&dir),
            &dir.join("out.zip")
        )
        .is_err());
        std::os::unix::fs::symlink("/etc/passwd", dir.join("link")).unwrap();
        assert!(plan(
            OpKind::Compress(Format::Zip),
            &[dir],
            &tmp.path().join("out.zip")
        )
        .is_err());
    }
}
