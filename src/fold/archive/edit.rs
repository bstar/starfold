//! Staged ZIP changes. Only Save publishes a rebuilt archive via the extension.
use crate::fold::{
    location::{ArchiveSource, Location},
    ops::{progress::Progress, Conflict, ConflictPolicy, OpKind, Outcome, Plan},
};
use starfold_archive_protocol::{Change, Request};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};
const FIRST_ADDED: usize = 1_000_000;
struct Session {
    source: ArchiveSource,
    stamp: (u64, Option<SystemTime>, u64, u64, i64, i64),
    scratch: tempfile::TempDir,
    changes: BTreeMap<usize, Option<PathBuf>>,
    additions: BTreeMap<usize, (PathBuf, PathBuf, u64)>,
    next: usize,
}
fn sessions() -> &'static Mutex<BTreeMap<ArchiveSource, Session>> {
    static S: OnceLock<Mutex<BTreeMap<ArchiveSource, Session>>> = OnceLock::new();
    S.get_or_init(Default::default)
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
fn source(key: &Path) -> anyhow::Result<ArchiveSource> {
    match Location::from_key(key)? {
        Location::Archive { source, .. } => Ok(source),
        _ => anyhow::bail!("Not an archive location"),
    }
}
fn validate(source: &ArchiveSource) -> anyhow::Result<()> {
    anyhow::ensure!(
        source.nested.is_empty(),
        "Nested archives are read-only; use Save As to create a separate ZIP"
    );
    anyhow::ensure!(
        super::Format::from_path(&source.file) == Some(super::Format::Zip),
        "Editing is available for ZIP archives"
    );
    Ok(())
}
pub fn import_conflicts(sources: &[PathBuf], destination: &Path) -> anyhow::Result<Vec<Conflict>> {
    let Location::Archive {
        source,
        member: None,
        ..
    } = Location::from_key(destination)?
    else {
        anyhow::bail!("Drop into an archive folder");
    };
    validate(&source)?;
    let listing = super::browser::read(destination, &crate::fold::listing::ListConfig::default());
    if let Some(error) = listing.error {
        anyhow::bail!("{error}");
    }
    let mut conflicts = Vec::new();
    for path in sources {
        let name = path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Drop has no filename"))?;
        if let Some(entry) = listing
            .entries
            .iter()
            .find(|entry| entry.path.file_name() == Some(name))
        {
            conflicts.push(Conflict {
                source: path.clone(),
                dest: entry.path.clone(),
                both_dirs: false,
            });
        }
    }
    Ok(conflicts)
}
pub fn pending(key: &Path) -> usize {
    source(key)
        .ok()
        .and_then(|s| {
            sessions()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&s)
                .map(|s| s.changes.len() + s.additions.len())
        })
        .unwrap_or(0)
}
pub fn overlay(source: &ArchiveSource, entries: &[super::Entry]) -> Vec<(usize, super::Entry)> {
    let sessions = sessions().lock().unwrap_or_else(|e| e.into_inner());
    let session = sessions.get(source);
    let mut rows = vec![];
    for (i, e) in entries.iter().enumerate() {
        let mut e = e.clone();
        if let Some(change) = session.and_then(|s| s.changes.get(&i)) {
            let Some(name) = change else {
                continue;
            };
            e.name = name.to_string_lossy().into_owned();
        }
        rows.push((i, e));
    }
    if let Some(session) = session {
        rows.extend(session.additions.iter().map(|(i, (name, path, size))| {
            (
                *i,
                super::Entry {
                    name: name.to_string_lossy().into_owned(),
                    bytes: Some(*size),
                    directory: path.is_dir(),
                },
            )
        }));
    }
    rows
}
pub fn member(
    source: &ArchiveSource,
    index: usize,
    entries: &[super::Entry],
) -> Option<super::Entry> {
    let sessions = sessions().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(session) = sessions.get(source) {
        if let Some((name, path, size)) = session.additions.get(&index) {
            return Some(super::Entry {
                name: name.to_string_lossy().into_owned(),
                bytes: Some(*size),
                directory: path.is_dir(),
            });
        }
        if let Some(change) = session.changes.get(&index) {
            return change.as_ref().and_then(|name| {
                entries.get(index).map(|e| super::Entry {
                    name: name.to_string_lossy().into_owned(),
                    ..e.clone()
                })
            });
        }
    }
    entries.get(index).cloned()
}
pub fn staged(source: &ArchiveSource, index: usize) -> Option<PathBuf> {
    sessions()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(source)
        .and_then(|s| s.additions.get(&index).map(|(_, p, _)| p.clone()))
}
fn gather(
    path: &Path,
    name: &Path,
    output: &mut Vec<(PathBuf, PathBuf)>,
    depth: usize,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        depth <= 64 && output.len() < super::MAX_ENTRIES,
        "Archive additions exceed tree limits"
    );
    if crate::fold::location::is_archive(path) {
        let location = Location::from_key(path)?;
        let Location::Archive {
            directory, member, ..
        } = location
        else {
            unreachable!()
        };
        let base = if member.is_some() {
            directory
        } else {
            directory.parent().unwrap_or(Path::new("")).into()
        };
        for member in super::browser::members(path)? {
            let Location::Archive {
                member: Some(ref entry),
                ..
            } = Location::from_key(&member)?
            else {
                continue;
            };
            let suffix = entry.name.strip_prefix(&base)?;
            output.push((member, name.parent().unwrap_or(Path::new("")).join(suffix)));
        }
        return Ok(());
    }
    let m = std::fs::symlink_metadata(path)?;
    if m.is_dir() {
        output.push((path.into(), name.into()));
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            gather(
                &entry.path(),
                &name.join(entry.file_name()),
                output,
                depth + 1,
            )?;
        }
    } else {
        anyhow::ensure!(m.is_file(), "Archive additions do not follow links");
        output.push((path.into(), name.into()));
    }
    Ok(())
}
fn collect_additions(
    sources: &[PathBuf],
    directory: &Path,
) -> anyhow::Result<Vec<(PathBuf, PathBuf)>> {
    let mut files = vec![];
    for from in sources {
        let name = from
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Source has no filename"))?;
        gather(from, &directory.join(name), &mut files, 0)?;
    }
    let mut names = std::collections::BTreeSet::new();
    for (_, name) in &files {
        anyhow::ensure!(
            names.insert(name.clone()),
            "Multiple sources map to ZIP member {}; copy them separately with different names",
            name.display()
        );
    }
    Ok(files)
}
pub fn plan(kind: OpKind, sources: &[PathBuf], dest: Option<&Path>) -> anyhow::Result<Plan> {
    let key = dest
        .filter(|d| crate::fold::location::is_archive(d))
        .or_else(|| {
            sources
                .iter()
                .find(|p| crate::fold::location::is_archive(p))
                .map(PathBuf::as_path)
        })
        .ok_or_else(|| anyhow::anyhow!("Archive operation has no location"))?;
    let source = source(key)?;
    if kind != OpKind::ArchiveTest
        && !(kind == OpKind::ArchiveSave
            && dest.is_some_and(|d| !crate::fold::location::is_archive(d)))
    {
        validate(&source)?;
    }
    let total_bytes = if matches!(kind, OpKind::ArchiveSave) {
        std::fs::metadata(&source.file)?.len()
    } else {
        sources
            .iter()
            .filter_map(|s| std::fs::metadata(s).ok())
            .map(|m| m.len())
            .sum()
    };
    let mut plan = Plan {
        sources: sources.into(),
        dest: dest.unwrap_or(key).into(),
        total_bytes,
        total_items: sources.len(),
        ..Plan::default()
    };
    if kind == OpKind::Copy {
        let Location::Archive {
            directory,
            member: None,
            ..
        } = Location::from_key(key)?
        else {
            anyhow::bail!("Select an archive folder");
        };
        let entries = super::browser::index_entries(&source)?;
        let existing = overlay(&source, &entries);
        plan.total_bytes = 0;
        for (from, name) in collect_additions(sources, &directory)? {
            let size = if crate::fold::location::is_archive(&from) {
                super::browser::summary(&from)?.bytes
            } else {
                {
                    let meta = std::fs::metadata(&from)?;
                    if meta.is_dir() {
                        0
                    } else {
                        meta.len()
                    }
                }
            };
            plan.total_bytes = plan
                .total_bytes
                .checked_add(size)
                .ok_or_else(|| anyhow::anyhow!("Archive size overflow"))?;
            anyhow::ensure!(
                plan.total_bytes <= super::MAX_OUTPUT,
                "Archive addition size limit exceeded"
            );
            if let Some((index, _)) = existing.iter().find(|(_, e)| Path::new(&e.name) == name) {
                plan.conflicts.push(Conflict {
                    source: from,
                    dest: Location::Archive {
                        source: source.clone(),
                        directory: name.parent().unwrap_or(Path::new("")).into(),
                        member: Some(crate::fold::location::Member {
                            index: *index,
                            name,
                        }),
                    }
                    .key(),
                    both_dirs: false,
                });
            }
        }
    }
    Ok(plan)
}
pub fn run(
    kind: OpKind,
    plan: &Plan,
    policy: ConflictPolicy,
    progress: &Progress,
    rename_targets: &[(PathBuf, PathBuf)],
) -> Outcome {
    let result = run_inner(kind, plan, policy, progress, rename_targets);
    match result {
        Ok(()) => Outcome {
            done: plan.sources.len().max(1),
            ..Outcome::default()
        },
        Err(error) => Outcome {
            cancelled: progress.is_cancelled(),
            failed: if progress.is_cancelled() {
                vec![]
            } else {
                vec![(
                    plan.sources
                        .first()
                        .cloned()
                        .unwrap_or_else(|| plan.dest.clone()),
                    error.to_string(),
                )]
            },
            ..Outcome::default()
        },
    }
}
fn run_inner(
    kind: OpKind,
    plan: &Plan,
    policy: ConflictPolicy,
    progress: &Progress,
    rename_targets: &[(PathBuf, PathBuf)],
) -> anyhow::Result<()> {
    let key = if crate::fold::location::is_archive(&plan.dest) {
        &plan.dest
    } else {
        plan.sources
            .first()
            .ok_or_else(|| anyhow::anyhow!("No archive selected"))?
    };
    let archive = source(key)?;
    if kind == OpKind::ArchiveTest {
        let path = super::browser::container_path(&archive)?;
        super::service::request(
            Request::Test {
                source: path,
                password: super::browser::password_for(&archive),
            },
            progress,
            None,
        )?;
        return Ok(());
    }
    if kind != OpKind::ArchiveSave || crate::fold::location::is_archive(&plan.dest) {
        validate(&archive)?;
    }
    if kind == OpKind::ArchiveDiscard {
        sessions()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&archive);
        return Ok(());
    }
    if kind == OpKind::ArchiveSave {
        return save(
            &archive,
            (!crate::fold::location::is_archive(&plan.dest)).then_some(plan.dest.as_path()),
            progress,
        );
    }
    let original = super::browser::index_entries(&archive)?;
    let existing = overlay(&archive, &original);
    let mut resolved = std::collections::HashMap::new();
    let queued_files = if kind == OpKind::Copy {
        let Location::Archive {
            directory,
            member: None,
            ..
        } = Location::from_key(&plan.dest)?
        else {
            anyhow::bail!("Drop into an archive folder");
        };
        let files = collect_additions(&plan.sources, &directory)?;
        for (key, name) in &files {
            if crate::fold::location::is_archive(key) {
                if let Location::Archive {
                    source,
                    member: Some(member),
                    ..
                } = Location::from_key(key)?
                {
                    anyhow::ensure!(
                        source != archive || member.name != *name,
                        "Cannot copy an archive member onto itself"
                    );
                }
                resolved.insert(key.clone(), super::browser::materialize(key, progress)?);
            }
        }
        files
    } else {
        vec![]
    };

    let mut sessions = sessions().lock().unwrap_or_else(|e| e.into_inner());
    if !sessions.contains_key(&archive) {
        sessions.insert(
            archive.clone(),
            Session {
                source: archive.clone(),
                stamp: stamp(&archive.file)?,
                scratch: tempfile::Builder::new()
                    .prefix("starfold-zip-edit-")
                    .tempdir()?,
                changes: BTreeMap::new(),
                additions: BTreeMap::new(),
                next: FIRST_ADDED,
            },
        );
    }
    let session = sessions.get_mut(&archive).unwrap();
    anyhow::ensure!(
        session.stamp == stamp(&archive.file)?,
        "Archive changed on disk; discard staged changes and reopen it"
    );
    match kind {
        OpKind::Copy => {
            // Stage the entire batch privately before changing the manifest.
            let batch = tempfile::tempdir_in(session.scratch.path())?;
            let mut additions = vec![];
            let files = queued_files;
            for (i, (from, mut name)) in files.into_iter().enumerate() {
                if existing.iter().any(|(_, e)| Path::new(&e.name) == name) {
                    match policy {
                        ConflictPolicy::Ask => {
                            anyhow::bail!("ZIP member already exists; choose a conflict policy")
                        }
                        ConflictPolicy::Skip => continue,
                        ConflictPolicy::Overwrite => {}
                        ConflictPolicy::RenameNew => {
                            if let Some((_, chosen)) = plan
                                .conflicts
                                .iter()
                                .find(|c| c.source == from)
                                .and_then(|c| rename_targets.iter().find(|(old, _)| *old == c.dest))
                            {
                                name =
                                    name.with_file_name(chosen.file_name().ok_or_else(|| {
                                        anyhow::anyhow!("Invalid replacement name")
                                    })?);
                            } else {
                                let stem = name.file_stem().unwrap_or_default().to_string_lossy();
                                let ext = name
                                    .extension()
                                    .map(|e| format!(".{}", e.to_string_lossy()))
                                    .unwrap_or_default();
                                let mut n = 1;
                                loop {
                                    let candidate =
                                        name.with_file_name(format!("{stem} ({n}){ext}"));
                                    if !existing
                                        .iter()
                                        .any(|(_, e)| Path::new(&e.name) == candidate)
                                        && !additions.iter().any(|(p, _, _)| *p == candidate)
                                    {
                                        name = candidate;
                                        break;
                                    }
                                    n += 1;
                                }
                            }
                        }
                    }
                }
                super::safe_path(&name.to_string_lossy())?;
                anyhow::ensure!(
                    !additions.iter().any(|(path, _, _)| *path == name),
                    "Replacement name collides with another staged member"
                );
                let target = batch.path().join(i.to_string());
                let metadata = std::fs::metadata(resolved.get(&from).unwrap_or(&from))?;
                let size = if metadata.is_dir() { 0 } else { metadata.len() };
                if let Some((_, free)) = crate::fold::places::filesystem_space(batch.path()) {
                    anyhow::ensure!(
                        size <= free,
                        "Not enough scratch space to stage ZIP additions"
                    );
                }
                let from = resolved.remove(&from).unwrap_or(from);
                if metadata.is_dir() {
                    std::fs::create_dir(&target)?;
                    additions.push((name, target, 0));
                    continue;
                }
                use std::os::unix::fs::OpenOptionsExt;
                let mut input = std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(from)?;
                let mut output = std::fs::File::create(&target)?;
                let mut buffer = [0; 256 * 1024];
                use std::io::{Read, Write};
                loop {
                    anyhow::ensure!(!progress.is_cancelled(), "cancelled");
                    let n = input.read(&mut buffer)?;
                    if n == 0 {
                        break;
                    }
                    output.write_all(&buffer[..n])?;
                    progress.add(n as u64);
                }
                additions.push((name, target, size));
            }
            let _retained = batch.keep();
            for addition in additions {
                for (i, e) in original.iter().enumerate() {
                    if e.name == addition.0.to_string_lossy() {
                        session.changes.insert(i, None);
                    }
                }
                session
                    .additions
                    .retain(|_, (name, _, _)| name != &addition.0);
                session.additions.insert(session.next, addition);
                session.next += 1;
            }
        }
        OpKind::Delete(_) | OpKind::Rename => {
            let mut updates = vec![];
            for from in &plan.sources {
                let Location::Archive {
                    source,
                    member,
                    directory,
                } = Location::from_key(from)?
                else {
                    anyhow::bail!("Select ZIP members");
                };
                anyhow::ensure!(source == archive, "Edit one ZIP per operation");
                let old = member.as_ref().map(|m| m.name.clone()).unwrap_or(directory);
                let indices: Vec<_> = if let Some(member) = member {
                    vec![(member.index, old.clone())]
                } else {
                    existing
                        .iter()
                        .filter_map(|(i, e)| {
                            Path::new(&e.name)
                                .strip_prefix(&old)
                                .ok()
                                .map(|_| (*i, PathBuf::from(&e.name)))
                        })
                        .collect()
                };
                for (index, name) in indices {
                    let replacement = if kind == OpKind::Rename {
                        let basename = plan
                            .dest
                            .file_name()
                            .ok_or_else(|| anyhow::anyhow!("Enter a new name"))?;
                        let root = old.parent().unwrap_or(Path::new("")).join(basename);
                        Some(if name == old {
                            root
                        } else {
                            root.join(name.strip_prefix(&old)?)
                        })
                    } else {
                        None
                    };
                    if let Some(name) = &replacement {
                        super::safe_path(&name.to_string_lossy())?;
                    }
                    updates.push((index, replacement));
                }
            }
            for (index, name) in updates {
                if index >= FIRST_ADDED {
                    if let Some(name) = name {
                        if let Some((old, _, _)) = session.additions.get_mut(&index) {
                            *old = name;
                        }
                    } else {
                        session.additions.remove(&index);
                    }
                } else {
                    session.changes.insert(index, name);
                }
            }
        }
        _ => anyhow::bail!("This archive operation is unavailable"),
    }
    Ok(())
}
pub fn save(
    archive: &ArchiveSource,
    destination: Option<&Path>,
    progress: &Progress,
) -> anyhow::Result<()> {
    let container = super::browser::container_path(archive)?;
    let mut sessions = sessions().lock().unwrap_or_else(|e| e.into_inner());
    let Some(session) = sessions.get(archive) else {
        if let Some(target) = destination {
            anyhow::ensure!(!target.exists(), "Save As destination already exists");
            let mut file =
                tempfile::NamedTempFile::new_in(target.parent().unwrap_or(Path::new(".")))?;
            let before = stamp(&archive.file)?;
            let size = std::fs::metadata(&container)?.len();
            if let Some((_, free)) = crate::fold::places::filesystem_space(file.path()) {
                anyhow::ensure!(size <= free, "Not enough space for Save As");
            }
            let mut input = std::fs::File::open(&container)?;
            use std::io::{Read, Write};
            let mut buffer = [0; 256 * 1024];
            loop {
                anyhow::ensure!(!progress.is_cancelled(), "cancelled");
                let n = input.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                file.write_all(&buffer[..n])?;
                progress.add(n as u64);
            }
            anyhow::ensure!(
                before == stamp(&archive.file)?,
                "Archive changed during Save As"
            );
            file.as_file().sync_all()?;
            file.persist_noclobber(target)?;
        }
        return Ok(());
    };
    anyhow::ensure!(
        session.stamp == stamp(&archive.file)?,
        "Archive changed on disk; original and pending edits were retained"
    );
    let target = destination.unwrap_or(&archive.file);
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Destination has no parent"))?;
    let required = session.stamp.0.saturating_add(
        session
            .additions
            .values()
            .map(|(_, _, size)| size)
            .sum::<u64>(),
    );
    if let Some((_, free)) = crate::fold::places::filesystem_space(parent) {
        anyhow::ensure!(required <= free, "Not enough free space to save this ZIP");
    }
    let stage = tempfile::Builder::new()
        .prefix(".starfold-zip-save-")
        .tempdir_in(parent)?;
    let output = stage.path().join("archive.zip");
    let changes = session
        .changes
        .iter()
        .map(|(index, name)| Change {
            index: *index,
            name: name.clone(),
        })
        .collect();
    let additions = session
        .additions
        .values()
        .map(|(name, path, size)| starfold_archive_protocol::Item {
            from: path.clone(),
            to: Some(name.clone()),
            kind: if path.is_dir() {
                starfold_archive_protocol::ItemKind::Dir
            } else {
                starfold_archive_protocol::ItemKind::File(*size)
            },
        })
        .collect();
    super::service::request(
        Request::Rebuild {
            source: archive.file.clone(),
            output: output.clone(),
            changes,
            additions,
        },
        progress,
        None,
    )?;
    anyhow::ensure!(!progress.is_cancelled(), "cancelled");
    anyhow::ensure!(
        session.stamp == stamp(&archive.file)?,
        "Archive changed during saving; pending edits were retained"
    );
    if destination.is_some() {
        anyhow::ensure!(!target.exists(), "Save As destination already exists");
    }
    std::fs::set_permissions(&output, std::fs::metadata(&archive.file)?.permissions())?;
    if destination.is_some() {
        std::fs::hard_link(&output, target)?;
        std::fs::remove_file(output)?;
    } else {
        std::fs::rename(output, target)?;
    }
    std::fs::File::open(parent)?.sync_all()?;
    sessions.remove(archive);
    super::browser::invalidate(archive);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fold::{
        archive::browser,
        listing::ListConfig,
        ops::{exec, DeleteHow},
    };
    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("edit.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&file).unwrap());
        for name in ["folder/one.txt", "keep.txt"] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(name.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        let root = Location::Filesystem(file.clone())
            .enter_archive()
            .unwrap()
            .key();
        (temp, file, root)
    }
    fn run(kind: OpKind, sources: Vec<PathBuf>, dest: Option<&Path>, policy: ConflictPolicy) {
        let plan = plan(kind, &sources, dest).unwrap();
        let outcome = exec::run(
            kind,
            &plan,
            policy,
            &exec::RunOptions::default(),
            &Progress::new(0),
        );
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
    }
    #[test]
    fn delete_is_staged_then_save_publishes_and_discard_restores() {
        let (_temp, file, root) = fixture();
        let original = std::fs::read(&file).unwrap();
        let listing = browser::read(&root, &ListConfig::default());
        let keep = listing
            .entries
            .iter()
            .find(|e| e.display == "keep.txt")
            .unwrap()
            .path
            .clone();
        run(
            OpKind::Delete(DeleteHow::Permanent),
            vec![keep],
            None,
            ConflictPolicy::Ask,
        );
        assert_eq!(std::fs::read(&file).unwrap(), original);
        assert!(pending(&root) > 0);
        assert!(!browser::read(&root, &ListConfig::default())
            .entries
            .iter()
            .any(|e| e.display == "keep.txt"));
        run(
            OpKind::ArchiveDiscard,
            vec![root.clone()],
            None,
            ConflictPolicy::Ask,
        );
        assert_eq!(pending(&root), 0);
        assert_eq!(std::fs::read(&file).unwrap(), original);
        let keep = browser::read(&root, &ListConfig::default())
            .entries
            .into_iter()
            .find(|e| e.display == "keep.txt")
            .unwrap()
            .path;
        run(
            OpKind::Delete(DeleteHow::Permanent),
            vec![keep],
            None,
            ConflictPolicy::Ask,
        );
        run(
            OpKind::ArchiveSave,
            vec![root.clone()],
            None,
            ConflictPolicy::Ask,
        );
        assert_eq!(pending(&root), 0);
        let mut zip = zip::ZipArchive::new(std::fs::File::open(file).unwrap()).unwrap();
        assert_eq!(zip.len(), 1);
        assert_eq!(zip.by_index(0).unwrap().name(), "folder/one.txt");
    }
    #[test]
    fn empty_directories_survive_save_and_duplicate_additions_are_rejected() {
        let (temp, file, root) = fixture();
        let folder = temp.path().join("empty");
        std::fs::create_dir(&folder).unwrap();
        run(OpKind::Copy, vec![folder], Some(&root), ConflictPolicy::Ask);
        assert!(browser::read(&root, &ListConfig::default())
            .entries
            .iter()
            .any(|entry| entry.display == "empty"
                && entry.kind == crate::fold::entry::EntryKind::Dir));
        run(
            OpKind::ArchiveSave,
            vec![root.clone()],
            None,
            ConflictPolicy::Ask,
        );
        let mut zip = zip::ZipArchive::new(std::fs::File::open(file).unwrap()).unwrap();
        assert!(zip.by_name("empty/").unwrap().is_dir());

        let first = temp.path().join("first");
        let second = temp.path().join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        std::fs::write(first.join("same.txt"), b"first").unwrap();
        std::fs::write(second.join("same.txt"), b"second").unwrap();
        let error = plan(
            OpKind::Copy,
            &[first.join("same.txt"), second.join("same.txt")],
            Some(&root),
        )
        .unwrap_err();
        assert!(error.to_string().contains("Multiple sources"));
        assert_eq!(pending(&root), 0);
    }
    #[test]
    fn add_conflicts_preview_save_as_and_external_change_guard() {
        let (temp, file, root) = fixture();
        let original = std::fs::read(&file).unwrap();
        let conflicts = import_conflicts(&[PathBuf::from("/remote/keep.txt")], &root).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert!(crate::fold::location::is_archive(&conflicts[0].dest));
        let input = temp.path().join("keep.txt");
        std::fs::write(&input, b"replacement").unwrap();
        let plan = plan(OpKind::Copy, std::slice::from_ref(&input), Some(&root)).unwrap();
        assert_eq!(plan.conflicts.len(), 1);
        let rejected = exec::run(
            OpKind::Copy,
            &plan,
            ConflictPolicy::Ask,
            &exec::RunOptions::default(),
            &Progress::new(0),
        );
        assert!(!rejected.failed.is_empty());
        run(
            OpKind::Copy,
            vec![input],
            Some(&root),
            ConflictPolicy::Overwrite,
        );
        let selected = browser::read(&root, &ListConfig::default())
            .entries
            .into_iter()
            .find(|e| e.display == "keep.txt")
            .unwrap();
        assert_eq!(
            std::fs::read(browser::materialize(&selected.path, &Progress::new(0)).unwrap())
                .unwrap(),
            b"replacement"
        );
        let output = temp.path().join("saved.zip");
        run(
            OpKind::ArchiveSave,
            vec![root.clone()],
            Some(&output),
            ConflictPolicy::Ask,
        );
        assert_eq!(std::fs::read(&file).unwrap(), original);
        assert!(output.is_file());
        let keep = browser::read(&root, &ListConfig::default())
            .entries
            .into_iter()
            .find(|e| e.display == "keep.txt")
            .unwrap()
            .path;
        run(
            OpKind::Delete(DeleteHow::Permanent),
            vec![keep],
            None,
            ConflictPolicy::Ask,
        );
        std::fs::write(&file, b"changed externally").unwrap();
        let progress = Progress::new(0);
        assert!(save(&source(&root).unwrap(), None, &progress).is_err());
        assert!(pending(&root) > 0);
        run(
            OpKind::ArchiveDiscard,
            vec![root],
            None,
            ConflictPolicy::Ask,
        );
    }
}
