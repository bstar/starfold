//! Archive-backed browser listings and bounded, demand-driven materialization.
use super::{safe_path, MAX_ENTRIES, MAX_OUTPUT};
use crate::fold::{
    entry::{Entry, EntryKind},
    listing::{ListConfig, Listing},
    location::{ArchiveSource, Location, Member},
    ops::progress::Progress,
};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::SystemTime,
};
#[derive(Clone)]
struct Index {
    source: PathBuf,
    entries: Arc<Vec<super::Entry>>,
    stamp: (u64, Option<SystemTime>, u64, u64, i64, i64),
    container_stamp: (u64, Option<SystemTime>, u64, u64, i64, i64),
    inspection: Option<starfold_archive_protocol::Inspection>,
}
#[derive(Default)]
struct Cache {
    root: Option<tempfile::TempDir>,
    indexes: BTreeMap<ArchiveSource, Index>,
    passwords: BTreeMap<PathBuf, String>,
    files: BTreeMap<(ArchiveSource, usize), (PathBuf, u64)>,
}
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}
pub fn unlock(file: PathBuf, password: String) {
    cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .passwords
        .insert(file, password);
}
pub fn password_for(source: &ArchiveSource) -> Option<String> {
    cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .passwords
        .get(&source.file)
        .cloned()
}
fn stamp(path: &Path) -> anyhow::Result<(u64, Option<SystemTime>, u64, u64, i64, i64)> {
    let m = std::fs::metadata(path)?;
    use std::os::unix::fs::MetadataExt;
    Ok((
        m.len(),
        m.modified().ok(),
        m.dev(),
        m.ino(),
        m.ctime(),
        m.ctime_nsec(),
    ))
}
fn index(source: &ArchiveSource) -> anyhow::Result<Index> {
    super::edit::recover(source)?;
    super::edit::check_source(source)?;
    let current = stamp(&source.file)?;
    let path = container_path(source)?;
    let container_stamp = stamp(&path)?;
    if let Some(cached) = cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .indexes
        .get(source)
        .filter(|i| i.stamp == current && i.container_stamp == container_stamp)
        .cloned()
    {
        return Ok(cached);
    }
    let inspection = super::service::inspect(&path).ok();
    let (entries, partial) =
        super::service::list_password(&path, MAX_ENTRIES, password_for(source).as_deref())?;
    anyhow::ensure!(!partial, "Archive exceeds the member limit");
    for entry in &entries {
        if entry.name != "." && entry.name != "./" {
            safe_path(&entry.name)?;
        }
    }
    let index = Index {
        source: path,
        entries: Arc::new(entries),
        stamp: current,
        container_stamp,
        inspection,
    };
    let mut cache = cache().lock().unwrap_or_else(|e| e.into_inner());
    let index_bytes = |idx: &Index| {
        idx.entries
            .iter()
            .map(|e| e.name.len() + std::mem::size_of::<super::Entry>())
            .sum::<usize>()
    };
    if cache.indexes.len() >= 32
        || cache
            .indexes
            .values()
            .map(index_bytes)
            .sum::<usize>()
            .saturating_add(index_bytes(&index))
            > 64 * 1024 * 1024
    {
        cache.indexes.clear();
    }
    cache.files.retain(|(s, _), _| s != source);
    cache.indexes.insert(source.clone(), index.clone());
    Ok(index)
}
pub fn container_path(source: &ArchiveSource) -> anyhow::Result<PathBuf> {
    if source.nested.is_empty() {
        return Ok(source.file.clone());
    }
    anyhow::ensure!(
        source.nested.len() <= 8,
        "Nested archive depth exceeds eight levels"
    );
    let mut parent = source.clone();
    let member = parent.nested.pop().unwrap();
    materialize(
        &Location::Archive {
            source: parent,
            directory: member.name.parent().unwrap_or(Path::new("")).into(),
            member: Some(member),
        }
        .key(),
        &Progress::new(0),
    )
}
pub fn read(key: &Path, cfg: &ListConfig) -> Listing {
    let result = (|| -> anyhow::Result<Listing> {
        let Location::Archive {
            source,
            directory,
            member: None,
        } = Location::from_key(key)?
        else {
            anyhow::bail!("Not an archive directory");
        };
        let index = index(&source)?;
        let mut directories = BTreeMap::<PathBuf, Entry>::new();
        let mut files = vec![];
        for (ordinal, entry) in super::edit::overlay(&source, &index.entries) {
            if entry.name == "." || entry.name == "./" {
                continue;
            }
            let name = safe_path(&entry.name)?;
            let Ok(relative) = name.strip_prefix(&directory) else {
                continue;
            };
            let mut parts = relative.components();
            let Some(first) = parts.next() else {
                continue;
            };
            let display = first.as_os_str().to_string_lossy().into_owned();
            let is_directory = parts.next().is_some() || entry.directory;
            let subdirectory = directory.join(first.as_os_str());
            let member = (!is_directory).then(|| Member {
                index: ordinal,
                name: name.clone(),
            });
            let location = Location::Archive {
                source: source.clone(),
                directory: if is_directory {
                    subdirectory.clone()
                } else {
                    directory.clone()
                },
                member,
            };
            let row = Entry {
                path: location.key(),
                display: display.clone(),
                kind: if is_directory {
                    EntryKind::Dir
                } else {
                    EntryKind::File
                },
                link_kind: None,
                len: if is_directory {
                    0
                } else {
                    entry.bytes.unwrap_or(0)
                },
                modified: None,
                created: None,
                accessed: None,
                mode: 0,
                executable: false,
                hidden: display.starts_with('.'),
            };
            if is_directory {
                directories.insert(subdirectory, row);
            } else {
                files.push(row);
            }
        }
        let mut entries: Vec<_> = directories.into_values().chain(files).collect();
        let truncated = entries.len() > cfg.max_entries;
        entries.truncate(cfg.max_entries);
        Ok(Listing {
            dir: key.into(),
            entries,
            truncated,
            error: super::edit::recovery_notice(&source),
            dir_mtime: index.stamp.1,
            space: crate::fold::places::filesystem_space(&source.file),
            archive_changes: super::edit::pending(key),
            archive_writable: super::edit::validate(&source).is_ok(),
        })
    })();
    result.unwrap_or_else(|e| Listing::error(key, e.to_string()))
}
struct Checked<'a> {
    output: std::fs::File,
    remaining: u64,
    progress: &'a Progress,
}
impl Write for Checked<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.progress.is_cancelled() {
            return Err(std::io::Error::other("cancelled"));
        }
        if bytes.len() as u64 > self.remaining {
            return Err(std::io::Error::other("Archive expansion limit exceeded"));
        }
        let n = self.output.write(bytes)?;
        self.remaining -= n as u64;
        self.progress.add(n as u64);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.output.flush()
    }
}
pub fn materialize(key: &Path, progress: &Progress) -> anyhow::Result<PathBuf> {
    let Location::Archive {
        source,
        member: Some(member),
        ..
    } = Location::from_key(key)?
    else {
        anyhow::bail!("Select a file inside the archive");
    };
    if let Some(file) = super::edit::staged(&source, member.index) {
        return Ok(file);
    }
    let idx = index(&source)?;
    let entry = idx
        .entries
        .get(member.index)
        .ok_or_else(|| anyhow::anyhow!("Archive member disappeared"))?;
    anyhow::ensure!(!entry.directory, "Archive member is a directory");
    let current = super::edit::member(&source, member.index, &idx.entries)
        .ok_or_else(|| anyhow::anyhow!("Archive member disappeared"))?;
    anyhow::ensure!(
        safe_path(&current.name)? == member.name,
        "Archive member identity changed"
    );
    anyhow::ensure!(
        entry.bytes.unwrap_or(0) <= MAX_OUTPUT,
        "Archive member exceeds expansion limit"
    );
    let file_key = (source.clone(), member.index);
    let target = {
        let mut cache = cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some((path, _)) = cache.files.get(&file_key).filter(|(p, _)| p.exists()) {
            return Ok(path.clone());
        }
        if cache.root.is_none() {
            cache.root = Some(
                tempfile::Builder::new()
                    .prefix("starfold-archives-")
                    .tempdir()?,
            );
        }
        let used = cache.files.values().map(|(_, size)| *size).sum::<u64>();
        if cache.files.len() >= 24
            || used.saturating_add(entry.bytes.unwrap_or(0)) > 1024 * 1024 * 1024
        {
            for (path, _) in cache.files.values() {
                let _ = std::fs::remove_file(path);
            }
            cache.files.clear();
        }
        let root = cache.root.as_ref().unwrap();
        let folder = tempfile::Builder::new()
            .prefix("member-")
            .tempdir_in(root.path())?
            .keep();
        folder.join(
            member
                .name
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Archive member has no filename"))?,
        )
    };
    let size = entry.bytes.unwrap_or(0);
    if let Some((_, free)) = crate::fold::places::filesystem_space(target.parent().unwrap()) {
        anyhow::ensure!(size <= free, "Not enough space to preview this member");
    }
    let out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)?;
    let mut output = Checked {
        output: out,
        remaining: MAX_OUTPUT,
        progress,
    };
    if let Err(error) = super::service::copy_member_progress(
        &idx.source,
        member.index,
        &mut output,
        password_for(&source).as_deref(),
        progress,
    ) {
        let _ = std::fs::remove_file(&target);
        return Err(error);
    }
    output.flush()?;
    if stamp(&source.file)? != idx.stamp {
        let _ = std::fs::remove_file(&target);
        anyhow::bail!("Archive changed while reading the member");
    }
    cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .files
        .insert(file_key, (target.clone(), size));
    Ok(target)
}
pub fn members(key: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let Location::Archive {
        source,
        directory,
        member,
    } = Location::from_key(key)?
    else {
        return Ok(vec![key.into()]);
    };
    if member.is_some() {
        return Ok(vec![key.into()]);
    }
    let idx = index(&source)?;
    Ok(super::edit::overlay(&source, &idx.entries)
        .into_iter()
        .filter(|(_, e)| !e.directory)
        .filter_map(|(i, e)| {
            let name = safe_path(&e.name).ok()?;
            name.strip_prefix(&directory).ok()?;
            Some(
                Location::Archive {
                    source: source.clone(),
                    directory: name.parent().unwrap_or(Path::new("")).into(),
                    member: Some(Member { index: i, name }),
                }
                .key(),
            )
        })
        .collect())
}

pub fn indexed_members(key: &Path) -> anyhow::Result<Vec<(PathBuf, super::Entry)>> {
    let Location::Archive {
        source,
        directory,
        member,
    } = Location::from_key(key)?
    else {
        anyhow::bail!("Not an archive location");
    };
    let idx = index(&source)?;
    let mut rows = vec![];
    for (index, entry) in super::edit::overlay(&source, &idx.entries) {
        if entry.directory
            || !Path::new(&entry.name).starts_with(&directory)
            || member.as_ref().is_some_and(|m| m.index != index)
        {
            continue;
        }
        let name = safe_path(&entry.name)?;
        let path = Location::Archive {
            source: source.clone(),
            directory: name.parent().unwrap_or(Path::new("")).into(),
            member: Some(Member { index, name }),
        }
        .key();
        rows.push((path, entry));
    }
    Ok(rows)
}
pub fn summary(key: &Path) -> anyhow::Result<crate::fold::summary::DirSummary> {
    let Location::Archive {
        source,
        directory,
        member,
    } = Location::from_key(key)?
    else {
        anyhow::bail!("Not an archive member");
    };
    let idx = index(&source)?;
    let mut result = crate::fold::summary::DirSummary::default();
    let entries = if let Some(member) = member {
        super::edit::member(&source, member.index, &idx.entries)
            .into_iter()
            .collect()
    } else {
        super::edit::overlay(&source, &idx.entries)
            .into_iter()
            .map(|(_, e)| e)
            .filter(|e| Path::new(&e.name).starts_with(&directory))
            .collect::<Vec<_>>()
    };
    for entry in entries {
        if entry.directory {
            result.dirs += 1;
        } else {
            result.files += 1;
            result.bytes = result.bytes.saturating_add(entry.bytes.unwrap_or(0));
        }
    }
    Ok(result)
}

pub fn plan_copy(sources: &[PathBuf], dest: &Path) -> anyhow::Result<crate::fold::ops::Plan> {
    use crate::fold::ops::{Conflict, Item, ItemKind, Plan};
    anyhow::ensure!(
        !crate::fold::location::is_archive(dest),
        "Use Add files to stage changes in an archive"
    );
    anyhow::ensure!(
        std::fs::metadata(dest)?.is_dir(),
        "Destination is not a directory"
    );
    let mut plan = Plan {
        dest: dest.into(),
        ..Plan::default()
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut destinations = std::collections::BTreeSet::new();
    for source_key in sources {
        let location = Location::from_key(source_key)?;
        let Location::Archive {
            source,
            directory,
            member,
        } = location
        else {
            let part = crate::fold::ops::plan::plan(
                crate::fold::ops::OpKind::Copy,
                std::slice::from_ref(source_key),
                Some(dest),
            )?;
            let offset = plan.items.len();
            plan.sources.extend(part.sources);
            plan.roots
                .extend(part.roots.into_iter().map(|r| r.map(|i| i + offset)));
            plan.items.extend(part.items);
            plan.conflicts.extend(part.conflicts);
            plan.total_bytes += part.total_bytes;
            continue;
        };
        let idx = index(&source)?;
        let base = if member.is_some() {
            directory.clone()
        } else {
            directory.parent().unwrap_or(Path::new("")).into()
        };
        if member.is_none() {
            let mut dirs = std::collections::BTreeSet::new();
            if !directory.as_os_str().is_empty() {
                dirs.insert(directory.clone());
            }
            for (_, entry) in super::edit::overlay(&source, &idx.entries) {
                if entry.directory {
                    let name = safe_path(&entry.name)?;
                    if name.starts_with(&directory) {
                        dirs.insert(name);
                    }
                }
            }
            for name in dirs {
                let relative = name.strip_prefix(&base)?;
                let target = dest.join(relative);
                let resolved_target = std::fs::canonicalize(&target).unwrap_or_else(|_| {
                    std::fs::canonicalize(target.parent().unwrap_or(dest))
                        .unwrap_or_else(|_| dest.into())
                        .join(target.file_name().unwrap_or_default())
                });
                anyhow::ensure!(
                    resolved_target != std::fs::canonicalize(&source.file)?,
                    "Cannot overwrite the archive containing this member"
                );
                if destinations.insert(target.clone()) {
                    plan.sources.push(
                        Location::Archive {
                            source: source.clone(),
                            directory: name.clone(),
                            member: None,
                        }
                        .key(),
                    );
                    plan.roots.push(Some(plan.items.len()));
                    plan.items.push(Item {
                        from: source_key.clone(),
                        to: Some(target),
                        kind: ItemKind::Dir,
                    });
                }
            }
        }
        let selected = members(source_key)?;
        for key in selected {
            if !seen.insert(key.clone()) {
                continue;
            }
            let Location::Archive {
                member: Some(member),
                ..
            } = Location::from_key(&key)?
            else {
                continue;
            };
            let relative = member.name.strip_prefix(&base)?;
            let target = dest.join(relative);
            let resolved_target = std::fs::canonicalize(&target).unwrap_or_else(|_| {
                std::fs::canonicalize(target.parent().unwrap_or(dest))
                    .unwrap_or_else(|_| dest.into())
                    .join(target.file_name().unwrap_or_default())
            });
            anyhow::ensure!(
                resolved_target != std::fs::canonicalize(&source.file)?,
                "Cannot overwrite the archive containing this member"
            );
            anyhow::ensure!(destinations.insert(target.clone()),"Selected members have duplicate destination names; extract them separately with Rename");
            plan.sources.push(key.clone());
            plan.roots.push(Some(plan.items.len()));
            if let Some(parent) = target.parent() {
                plan.items.push(Item {
                    from: source_key.clone(),
                    to: Some(parent.into()),
                    kind: ItemKind::Dir,
                });
            }
            let len = super::edit::member(&source, member.index, &idx.entries)
                .ok_or_else(|| anyhow::anyhow!("Archive member disappeared"))?
                .bytes
                .unwrap_or(0);
            plan.total_bytes = plan
                .total_bytes
                .checked_add(len)
                .ok_or_else(|| anyhow::anyhow!("Archive size overflow"))?;
            anyhow::ensure!(
                plan.total_bytes <= MAX_OUTPUT,
                "Archive expansion limit exceeded"
            );
            if let Ok(meta) = std::fs::symlink_metadata(&target) {
                plan.conflicts.push(Conflict {
                    source: key.clone(),
                    dest: target.clone(),
                    both_dirs: false,
                });
                anyhow::ensure!(!meta.file_type().is_symlink(), "Destination is a symlink");
            }
            plan.items.push(Item {
                from: key,
                to: Some(target),
                kind: ItemKind::File(len),
            });
        }
    }
    plan.total_items = plan.items.len();
    Ok(plan)
}
pub fn copy_to(key: &Path, output: &mut dyn Write, progress: &Progress) -> anyhow::Result<()> {
    let Location::Archive {
        source,
        member: Some(member),
        ..
    } = Location::from_key(key)?
    else {
        anyhow::bail!("Select an archive file");
    };

    let idx = index(&source)?;
    let current = super::edit::member(&source, member.index, &idx.entries)
        .ok_or_else(|| anyhow::anyhow!("Archive member disappeared"))?;
    anyhow::ensure!(
        safe_path(&current.name)? == member.name,
        "Archive member identity changed"
    );
    struct Checked<'a> {
        inner: &'a mut dyn Write,
        progress: &'a Progress,
        left: u64,
    }
    impl Write for Checked<'_> {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            if self.progress.is_cancelled() {
                return Err(std::io::Error::other("cancelled"));
            }
            if b.len() as u64 > self.left {
                return Err(std::io::Error::other("Archive expansion limit exceeded"));
            }
            let n = self.inner.write(b)?;
            self.left -= n as u64;
            self.progress.add(n as u64);
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.inner.flush()
        }
    }
    let mut checked = Checked {
        inner: output,
        progress,
        left: MAX_OUTPUT,
    };
    if let Some(file) = super::edit::staged(&source, member.index) {
        std::io::copy(&mut std::fs::File::open(file)?, &mut checked)?;
        return Ok(());
    }
    super::service::copy_member_progress(
        &idx.source,
        member.index,
        &mut checked,
        password_for(&source).as_deref(),
        progress,
    )?;
    anyhow::ensure!(
        stamp(&source.file)? == idx.stamp,
        "Archive changed while copying the member"
    );
    Ok(())
}

pub fn index_entries(source: &ArchiveSource) -> anyhow::Result<Arc<Vec<super::Entry>>> {
    Ok(index(source)?.entries)
}
pub fn invalidate(source: &ArchiveSource) {
    let mut cache = cache().lock().unwrap_or_else(|e| e.into_inner());
    cache.indexes.remove(source);
    cache.files.retain(|(s, _), _| s != source);
}

pub fn materialize_cancellable(
    key: &Path,
    stale: &(impl Fn() -> bool + Sync),
) -> anyhow::Result<PathBuf> {
    let progress = Progress::new(0);
    let stop = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                if stale() {
                    progress.cancel();
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        });
        let result = materialize(key, &progress);
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        result
    })
}

pub fn writable(source: &ArchiveSource) -> anyhow::Result<bool> {
    Ok(index(source)?.inspection.is_some_and(|i| i.editable))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, data) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }
    #[test]
    fn implicit_directories_selective_copy_and_nested_browsing() {
        let temp = tempfile::tempdir().unwrap();
        let inner = temp.path().join("inner.zip");
        zip(&inner, &[("hello.txt", b"nested")]);
        let file = temp.path().join("outer.zip");
        let inner_bytes = std::fs::read(&inner).unwrap();
        zip(
            &file,
            &[
                ("folder/wanted.txt", b"yes"),
                ("folder/unwanted.txt", b"no"),
                ("inner.zip", &inner_bytes),
            ],
        );
        let root = Location::Filesystem(file).enter_archive().unwrap().key();
        let listing = read(&root, &ListConfig::default());
        assert!(listing.error.is_none(), "{:?}", listing.error);
        let folder = listing
            .entries
            .iter()
            .find(|e| e.display == "folder")
            .unwrap();
        let nested = listing
            .entries
            .iter()
            .find(|e| e.display == "inner.zip")
            .unwrap();
        let folder_listing = read(&folder.path, &ListConfig::default());
        let selected = folder_listing
            .entries
            .iter()
            .find(|e| e.display == "wanted.txt")
            .unwrap();
        let output = temp.path().join("out");
        std::fs::create_dir(&output).unwrap();
        let plan = plan_copy(std::slice::from_ref(&selected.path), &output).unwrap();
        assert_eq!(plan.total_bytes, 3);
        let outcome = crate::fold::ops::exec::run(
            crate::fold::ops::OpKind::Copy,
            &plan,
            crate::fold::ops::ConflictPolicy::Ask,
            &crate::fold::ops::exec::RunOptions::default(),
            &Progress::new(0),
        );
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert_eq!(std::fs::read(output.join("wanted.txt")).unwrap(), b"yes");
        assert!(!output.join("unwanted.txt").exists());
        let nested_root = Location::from_key(&nested.path)
            .unwrap()
            .enter_archive()
            .unwrap()
            .key();
        let nested_listing = read(&nested_root, &ListConfig::default());
        assert!(nested_listing.error.is_none(), "{:?}", nested_listing.error);
        let local = materialize(&nested_listing.entries[0].path, &Progress::new(0)).unwrap();
        assert_eq!(std::fs::read(local).unwrap(), b"nested");
    }
    #[test]
    fn source_change_invalidates_index_and_materialized_members() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("a.zip");
        zip(&file, &[("a.txt", b"old")]);
        let root = Location::Filesystem(file.clone())
            .enter_archive()
            .unwrap()
            .key();
        let row = read(&root, &ListConfig::default()).entries.remove(0);
        let first = materialize(&row.path, &Progress::new(0)).unwrap();
        assert_eq!(std::fs::read(first).unwrap(), b"old");
        zip(&file, &[("a.txt", b"new content")]);
        let changed = materialize(&row.path, &Progress::new(0)).unwrap();
        assert_eq!(std::fs::read(changed).unwrap(), b"new content");
    }
    #[test]
    fn traversal_is_rejected_before_browsing() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("bad.zip");
        zip(&file, &[("../escape", b"bad")]);
        let root = Location::Filesystem(file).enter_archive().unwrap().key();
        assert!(read(&root, &ListConfig::default()).error.is_some());
    }
}
