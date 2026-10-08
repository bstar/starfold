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
type Stamp = (u64, Option<SystemTime>, u64, u64, i64, i64);
struct Scratch {
    path: PathBuf,
    cleanup: bool,
}
impl Scratch {
    fn new() -> anyhow::Result<Self> {
        #[cfg(not(test))]
        let dir = {
            use std::os::unix::fs::PermissionsExt;
            let root = crate::PATHS.cache_dir()?.join("archive-edits");
            std::fs::create_dir_all(&root)?;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
            tempfile::Builder::new()
                .prefix("session-")
                .tempdir_in(root)?
        };
        #[cfg(test)]
        let dir = tempfile::Builder::new()
            .prefix("starfold-archive-test-")
            .tempdir()?;
        Ok(Self {
            path: dir.keep(),
            cleanup: true,
        })
    }
    fn path(&self) -> &Path {
        &self.path
    }
    fn disable_cleanup(&mut self, keep: bool) {
        self.cleanup = !keep;
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        if self.cleanup {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

struct Session {
    source: ArchiveSource,
    stamp: Stamp,
    scratch: Scratch,
    changes: BTreeMap<usize, Option<PathBuf>>,
    additions: BTreeMap<usize, (PathBuf, PathBuf, u64)>,
    next: usize,
    replacements: BTreeMap<usize, PathBuf>,
    working: BTreeMap<usize, (PathBuf, Stamp)>,
}
fn sessions() -> &'static Mutex<BTreeMap<ArchiveSource, Session>> {
    static S: OnceLock<Mutex<BTreeMap<ArchiveSource, Session>>> = OnceLock::new();
    S.get_or_init(Default::default)
}
fn stamp(path: &Path) -> anyhow::Result<Stamp> {
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
pub fn check_source(source: &ArchiveSource) -> anyhow::Result<()> {
    let current = stamp(&source.file)?;
    let sessions = sessions().lock().unwrap_or_else(|e| e.into_inner());
    anyhow::ensure!(sessions.iter().filter(|(s,_)|s.file==source.file).all(|(_,session)|session.stamp==current),
        "Archive changed on disk; pending edits were retained. Discard them or recover from the archive-edits cache before reopening.");
    Ok(())
}

pub fn validate(source: &ArchiveSource) -> anyhow::Result<()> {
    let mut ancestor = source.clone();
    loop {
        anyhow::ensure!(
            super::browser::writable(&ancestor)?,
            "Editing requires a ZIP at every archive level; extract this member to edit it"
        );
        if ancestor.nested.pop().is_none() {
            break;
        }
    }
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
    let Ok(source) = source(key) else {
        return 0;
    };
    sessions()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|(s, _)| s.file == source.file && s.nested.starts_with(&source.nested))
        .map(|(_, s)| s.changes.len() + s.additions.len() + s.replacements.len())
        .sum()
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
        if let Some(path) = session.and_then(|s| s.replacements.get(&i)) {
            e.bytes = std::fs::metadata(path).ok().map(|m| m.len());
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
        .and_then(|s| {
            s.replacements
                .get(&index)
                .cloned()
                .or_else(|| s.additions.get(&index).map(|(_, p, _)| p.clone()))
        })
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
    if !matches!(kind, OpKind::ArchiveTest | OpKind::ArchiveDiscard)
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
    if !matches!(kind, OpKind::ArchiveSave | OpKind::ArchiveDiscard) {
        validate(&archive)?;
    }
    if kind == OpKind::ArchiveDiscard {
        let mut sessions = sessions().lock().unwrap_or_else(|e| e.into_inner());
        let keys: Vec<_> = sessions
            .keys()
            .filter(|s| s.file == archive.file && s.nested.starts_with(&archive.nested))
            .cloned()
            .collect();
        for key in &keys {
            if let Some(mut s) = sessions.remove(key) {
                s.scratch.disable_cleanup(false);
            }
        }
        drop(sessions);
        for key in keys {
            super::browser::invalidate(&key);
        }
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
        sessions.insert(archive.clone(), new_session(&archive)?);
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
    journal(session)?;
    Ok(())
}
/// Publish all staged descendants through the writable ZIP chain in one transaction.
pub fn save(
    archive: &ArchiveSource,
    destination: Option<&Path>,
    progress: &Progress,
) -> anyhow::Result<()> {
    sync_working()?;
    let root = if destination.is_some() {
        archive.clone()
    } else {
        ArchiveSource {
            file: archive.file.clone(),
            nested: vec![],
        }
    };
    if let Some(target) = destination {
        if validate(&root).is_err() {
            return save_converted(&root, target, progress);
        }
    }
    validate(&root)?;
    let mut sources: Vec<_> = sessions()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .filter(|s| s.file == root.file && s.nested.starts_with(&root.nested))
        .cloned()
        .collect();
    sources.push(root.clone());
    let mut parents = vec![];
    for source in &sources {
        let mut parent = source.clone();
        while parent.nested.len() > root.nested.len() {
            parent.nested.pop();
            parents.push(parent.clone());
        }
    }
    sources.extend(parents);
    sources.sort();
    sources.dedup();
    let mut containers = BTreeMap::new();
    for source in &sources {
        validate(source)?;
        containers.insert(source.clone(), super::browser::container_path(source)?);
    }
    let mut sessions = sessions().lock().unwrap_or_else(|e| e.into_inner());
    let before = stamp(&archive.file)?;
    for source in &sources {
        if let Some(session) = sessions.get(source) {
            anyhow::ensure!(
                session.stamp == before,
                "Archive changed on disk; original and pending edits were retained"
            );
        }
    }
    let target = destination.unwrap_or(&root.file);
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Destination has no parent"))?;
    if destination.is_some() {
        anyhow::ensure!(!target.exists(), "Save As destination already exists");
    }
    let required = sources
        .iter()
        .map(|s| {
            let base = std::fs::metadata(&containers[s]).map_or(0, |m| m.len());
            let edits = sessions.get(s).map_or(0, |session| {
                session.additions.values().map(|(_, _, n)| *n).sum::<u64>()
                    + session
                        .replacements
                        .values()
                        .filter_map(|p| std::fs::metadata(p).ok())
                        .map(|m| m.len())
                        .sum::<u64>()
            });
            base.saturating_add(edits)
        })
        .fold(0u64, u64::saturating_add)
        .saturating_mul(2);
    if let Some((_, free)) = crate::fold::places::filesystem_space(parent) {
        anyhow::ensure!(
            required <= free,
            "Not enough free space to save this ZIP chain"
        );
    }
    let stage = tempfile::Builder::new()
        .prefix(".starfold-zip-save-")
        .tempdir_in(parent)?;
    sources.sort_by_key(|s| std::cmp::Reverse(s.nested.len()));
    let mut propagated = BTreeMap::<ArchiveSource, BTreeMap<usize, PathBuf>>::new();
    let mut final_output = None;
    for (ordinal, source) in sources.iter().enumerate() {
        anyhow::ensure!(!progress.is_cancelled(), "cancelled");
        let output = stage.path().join(format!("rebuilt-{ordinal}.zip"));
        let session = sessions.get(source);
        let changes = session
            .map(|s| {
                s.changes
                    .iter()
                    .map(|(index, name)| Change {
                        index: *index,
                        name: name.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let additions = session
            .map(|s| {
                s.additions
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
                    .collect()
            })
            .unwrap_or_default();
        let mut replacements = session.map(|s| s.replacements.clone()).unwrap_or_default();
        if let Some(children) = propagated.remove(source) {
            replacements.extend(children);
        }
        super::service::request(
            Request::RebuildEdited {
                source: containers[source].clone(),
                output: output.clone(),
                changes,
                additions,
                replacements: replacements
                    .into_iter()
                    .map(|(index, from)| starfold_archive_protocol::Replacement { index, from })
                    .collect(),
                password: super::browser::password_for(source),
            },
            progress,
            None,
        )?;
        if source == &root {
            final_output = Some(output);
        } else {
            let mut parent = source.clone();
            let member = parent.nested.pop().unwrap();
            propagated
                .entry(parent)
                .or_default()
                .insert(member.index, output);
        }
    }
    anyhow::ensure!(!progress.is_cancelled(), "cancelled");
    anyhow::ensure!(
        before == stamp(&archive.file)?,
        "Archive changed during saving; pending edits were retained"
    );
    let output = final_output.ok_or_else(|| anyhow::anyhow!("Missing rebuilt ZIP"))?;
    std::fs::set_permissions(&output, std::fs::metadata(&archive.file)?.permissions())?;
    if destination.is_some() {
        std::fs::hard_link(&output, target)?;
    } else {
        std::fs::rename(&output, target)?;
    }
    std::fs::File::open(parent)?.sync_all()?;
    for source in &sources {
        if let Some(mut session) = sessions.remove(source) {
            session.scratch.disable_cleanup(false);
        }
    }
    drop(sessions);
    for source in &sources {
        super::browser::invalidate(source);
    }
    Ok(())
}

fn new_session(source: &ArchiveSource) -> anyhow::Result<Session> {
    Ok(Session {
        source: source.clone(),
        stamp: stamp(&source.file)?,
        scratch: Scratch::new()?,
        changes: BTreeMap::new(),
        additions: BTreeMap::new(),
        next: FIRST_ADDED,
        replacements: BTreeMap::new(),
        working: BTreeMap::new(),
    })
}

/// Editors always receive their own copy, separate from extraction/preview caches.
pub fn working_copy(key: &Path, local: &Path) -> anyhow::Result<PathBuf> {
    let Location::Archive {
        source,
        member: Some(member),
        ..
    } = Location::from_key(key)?
    else {
        anyhow::bail!("Select a member");
    };
    validate(&source)?;
    let mut sessions = sessions().lock().unwrap_or_else(|e| e.into_inner());
    if !sessions.contains_key(&source) {
        sessions.insert(source.clone(), new_session(&source)?);
    }
    let session = sessions.get_mut(&source).unwrap();
    anyhow::ensure!(
        session.stamp == stamp(&source.file)?,
        "Archive changed; reopen it before editing"
    );
    if let Some((path, _)) = session.working.get(&member.index) {
        return Ok(path.clone());
    }
    let folder = session
        .scratch
        .path()
        .join(format!("work-{}", member.index));
    std::fs::create_dir_all(&folder)?;
    let path = folder.join(
        member
            .name
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Member has no filename"))?,
    );
    std::fs::copy(local, &path)?;
    session
        .working
        .insert(member.index, (path.clone(), stamp(&path)?));
    Ok(path)
}

/// Snapshot completed editor writes. Metadata notices atomic editor replacements.
pub fn sync_working() -> anyhow::Result<Vec<ArchiveSource>> {
    let mut sessions = sessions().lock().unwrap_or_else(|e| e.into_inner());
    let mut changed = vec![];
    for (source, session) in sessions.iter_mut() {
        for (index, (working, previous)) in &mut session.working {
            let current = match stamp(working) {
                Ok(stamp) => stamp,
                Err(_e) if !working.exists() => continue,
                Err(e) => return Err(e),
            };
            if &current == previous {
                continue;
            }
            anyhow::ensure!(
                current.0 <= super::MAX_OUTPUT,
                "Edited member exceeds size limit"
            );
            let snapshot = session.scratch.path().join(format!("replacement-{index}"));
            let mut staged = tempfile::NamedTempFile::new_in(session.scratch.path())?;
            std::fs::copy(&*working, staged.path())?;
            if stamp(working)? != current {
                continue;
            }
            staged.as_file_mut().sync_all()?;
            staged.persist(&snapshot)?;
            if let Some((_, path, size)) = session.additions.get_mut(index) {
                *path = snapshot;
                *size = current.0;
            } else {
                session.replacements.insert(*index, snapshot);
            }
            *previous = current;
            changed.push(source.clone());
        }
        if changed.contains(source) {
            journal(session)?;
        }
    }
    changed.sort();
    changed.dedup();
    Ok(changed)
}

fn journal(session: &mut Session) -> anyhow::Result<()> {
    let manifest = serde_json::json!({"version":1,"source":session.source,"stamp":session.stamp,"changes":session.changes,"additions":session.additions,"replacements":session.replacements,"next":session.next});
    let mut file = tempfile::NamedTempFile::new_in(session.scratch.path())?;
    use std::io::Write;
    serde_json::to_writer(&mut file, &manifest)?;
    file.flush()?;
    file.as_file().sync_all()?;
    file.persist(session.scratch.path().join("recovery.json"))?;
    session.scratch.disable_cleanup(true);
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
    fn editor_saves_stage_private_copies_and_nested_save_publishes_entire_chain() {
        use std::io::Write;
        let (temp, inner, _) = fixture();
        let outer = temp.path().join("outer.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&outer).unwrap());
        zip.start_file("nested.zip", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&std::fs::read(&inner).unwrap()).unwrap();
        zip.start_file("untouched.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"keep me").unwrap();
        zip.finish().unwrap();
        let original = std::fs::read(&outer).unwrap();
        let root = Location::Filesystem(outer.clone())
            .enter_archive()
            .unwrap()
            .key();
        let nested = browser::read(&root, &ListConfig::default())
            .entries
            .into_iter()
            .find(|e| e.display == "nested.zip")
            .unwrap()
            .path;
        let nested_root = Location::from_key(&nested)
            .unwrap()
            .enter_archive()
            .unwrap()
            .key();
        let member = browser::read(&nested_root, &ListConfig::default())
            .entries
            .into_iter()
            .find(|e| e.display == "keep.txt")
            .unwrap()
            .path;
        let local = browser::materialize(&member, &Progress::new(0)).unwrap();
        let working = working_copy(&member, &local).unwrap();
        assert_ne!(working, local);
        let replacement = temp.path().join("editor-atomic-save");
        std::fs::write(&replacement, b"new contents from editor").unwrap();
        std::fs::rename(&replacement, &working).unwrap();
        sync_working().unwrap();
        assert_eq!(std::fs::read(&local).unwrap(), b"keep.txt");
        assert_eq!(std::fs::read(&outer).unwrap(), original);
        assert_eq!(
            std::fs::read(browser::materialize(&member, &Progress::new(0)).unwrap()).unwrap(),
            b"new contents from editor"
        );
        let Location::Archive { source, .. } = Location::from_key(&nested_root).unwrap() else {
            unreachable!()
        };
        save(&source, None, &Progress::new(0)).unwrap();
        assert_eq!(pending(&nested_root), 0);
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&outer).unwrap()).unwrap();
        use std::io::Read;
        let mut inner = vec![];
        zip.by_name("nested.zip")
            .unwrap()
            .read_to_end(&mut inner)
            .unwrap();
        let mut nested = zip::ZipArchive::new(std::io::Cursor::new(inner)).unwrap();
        let mut text = String::new();
        nested
            .by_name("keep.txt")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "new contents from editor");
        text.clear();
        zip.by_name("untouched.txt")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "keep me");
    }

    #[test]
    fn recovery_restores_saved_snapshots_and_retains_source_conflicts() {
        let (temp, file, root) = fixture();
        let Location::Archive { source, .. } = Location::from_key(&root).unwrap() else {
            unreachable!()
        };
        let recovery_root = temp.path().join("recovery");
        std::fs::create_dir(&recovery_root).unwrap();
        let folder = recovery_root.join("session");
        std::fs::create_dir(&folder).unwrap();
        let snapshot = folder.join("replacement-1");
        std::fs::write(&snapshot, b"recovered").unwrap();
        let manifest = serde_json::json!({"version":1,"source":source,"stamp":stamp(&file).unwrap(),"changes":{},"additions":{},"replacements":{"1":snapshot},"next":FIRST_ADDED});
        std::fs::write(
            folder.join("recovery.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        recover_at(&source, &recovery_root).unwrap();
        assert_eq!(pending(&root), 1);
        assert_eq!(
            sessions()
                .lock()
                .unwrap()
                .get(&source)
                .unwrap()
                .replacements
                .get(&1),
            Some(&snapshot)
        );
        // Simulate restart by releasing the recovered session while retaining its journal.
        sessions().lock().unwrap().remove(&source);
        std::fs::write(&file, b"changed source").unwrap();
        recover_at(&source, &recovery_root).unwrap();
        assert_eq!(pending(&root), 0);
        assert!(snapshot.exists());
        assert!(recovery_notice(&source)
            .unwrap()
            .contains("Recovery files retained"));
    }

    #[test]
    fn cancelled_editor_save_retains_original_and_staging() {
        let (_temp, file, root) = fixture();
        let original = std::fs::read(&file).unwrap();
        let member = browser::read(&root, &ListConfig::default())
            .entries
            .into_iter()
            .find(|e| e.display == "keep.txt")
            .unwrap()
            .path;
        let local = browser::materialize(&member, &Progress::new(0)).unwrap();
        let working = working_copy(&member, &local).unwrap();
        std::fs::write(working, b"edited").unwrap();
        sync_working().unwrap();
        let progress = Progress::new(0);
        progress.cancel();
        let Location::Archive { source, .. } = Location::from_key(&root).unwrap() else {
            unreachable!()
        };
        assert!(save(&source, None, &progress).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), original);
        assert!(pending(&root) > 0);
        std::fs::write(&file, b"externally replaced archive").unwrap();
        assert!(check_source(&source).is_err());
        assert!(browser::read(&root, &ListConfig::default())
            .error
            .unwrap()
            .contains("pending edits were retained"));
        run(
            OpKind::ArchiveDiscard,
            vec![root],
            None,
            ConflictPolicy::Ask,
        );
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

pub fn is_working(key: &Path) -> bool {
    let Ok(Location::Archive {
        source,
        member: Some(member),
        ..
    }) = Location::from_key(key)
    else {
        return false;
    };
    sessions()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&source)
        .is_some_and(|s| s.working.contains_key(&member.index))
}

#[derive(serde::Deserialize)]
struct Recovery {
    version: u32,
    source: ArchiveSource,
    stamp: Stamp,
    changes: BTreeMap<usize, Option<PathBuf>>,
    additions: BTreeMap<usize, (PathBuf, PathBuf, u64)>,
    replacements: BTreeMap<usize, PathBuf>,
    next: usize,
}
fn recovery_notices() -> &'static Mutex<BTreeMap<PathBuf, String>> {
    static NOTICES: OnceLock<Mutex<BTreeMap<PathBuf, String>>> = OnceLock::new();
    NOTICES.get_or_init(Default::default)
}
pub fn recovery_notice(source: &ArchiveSource) -> Option<String> {
    recovery_notices()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&source.file)
        .cloned()
}

pub fn recover(source: &ArchiveSource) -> anyhow::Result<()> {
    #[cfg(test)]
    {
        let _ = source;
        Ok(())
    }
    #[cfg(not(test))]
    {
        static CHECKED: OnceLock<Mutex<std::collections::BTreeSet<PathBuf>>> = OnceLock::new();
        let mut checked = CHECKED
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if checked.contains(&source.file) {
            return Ok(());
        }
        recover_at(source, &crate::PATHS.cache_dir()?.join("archive-edits"))?;
        checked.insert(source.file.clone());
        Ok(())
    }
}

fn recover_at(source: &ArchiveSource, root: &Path) -> anyhow::Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(root)? {
        let folder = entry?.path();
        let manifest = folder.join("recovery.json");
        if !manifest.is_file() {
            continue;
        }
        use std::io::Read;
        let mut bytes = vec![];
        std::fs::File::open(&manifest)?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 1024 * 1024 {
            continue;
        }
        let Ok(recovery) = serde_json::from_slice::<Recovery>(&bytes) else {
            continue;
        };
        if recovery.version != 1 || recovery.source.file != source.file {
            continue;
        }
        if stamp(&source.file)? != recovery.stamp {
            let notice = format!(
                "Archive changed since the saved edits. Recovery files retained at {}",
                folder.display()
            );
            tracing::warn!(path=%manifest.display(), "{notice}");
            recovery_notices()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(source.file.clone(), notice);
            continue;
        }
        let canonical = folder.canonicalize()?;
        for path in recovery
            .replacements
            .values()
            .chain(recovery.additions.values().map(|(_, p, _)| p))
        {
            anyhow::ensure!(
                path.canonicalize()?.starts_with(&canonical),
                "Invalid archive recovery path"
            );
        }
        let session = Session {
            source: recovery.source.clone(),
            stamp: recovery.stamp,
            scratch: Scratch {
                path: folder,
                cleanup: false,
            },
            changes: recovery.changes,
            additions: recovery.additions,
            replacements: recovery.replacements,
            next: recovery.next,
            working: BTreeMap::new(),
        };
        sessions()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(recovery.source)
            .or_insert(session);
    }
    Ok(())
}

fn save_converted(
    source: &ArchiveSource,
    target: &Path,
    progress: &Progress,
) -> anyhow::Result<()> {
    anyhow::ensure!(!target.exists(), "Save As destination already exists");
    let before = stamp(&source.file)?;
    let container = super::browser::container_path(source)?;
    let parent = target.parent().unwrap_or(Path::new("."));
    let stage = tempfile::Builder::new()
        .prefix(".starfold-convert-")
        .tempdir_in(parent)?;
    let output = stage.path().join("converted.zip");
    super::service::request(
        Request::ConvertToZip {
            source: container,
            output: output.clone(),
            password: super::browser::password_for(source),
        },
        progress,
        None,
    )?;
    anyhow::ensure!(!progress.is_cancelled(), "cancelled");
    anyhow::ensure!(
        before == stamp(&source.file)?,
        "Source archive changed during conversion"
    );
    std::fs::hard_link(&output, target)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}
