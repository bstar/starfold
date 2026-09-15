//! What an operation would do, worked out before anything is touched.
//!
//! `plan` never writes to disk -- it only reads. It expands every directory
//! source recursively, **never following a symlink** (a symlink is one item,
//! recreated as a link by `exec`, whatever it points at); totals the bytes of
//! every regular file found; records a [`super::Conflict`] for every name
//! already present at the destination; and refuses a copy or move that would
//! land inside itself. All of it is read once, in one pass, into
//! [`super::Plan::items`], so `exec` never has to read a directory itself.

use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use super::{Conflict, Item, ItemKind, OpKind, Plan};

/// How deep a directory tree is walked before `plan` stops descending.
///
/// Not an error: a tree this deep is vanishingly unlikely to be anything
/// other than a symlink-free cycle a hostile or broken filesystem produced
/// (bind mounts nested into themselves, for instance), and the `(dev, ino)`
/// check below already terminates the ordinary case of a real loop. The cap
/// is depth-in-directories, not depth-in-symlinks -- nothing here ever
/// follows a symlink to begin with.
const MAX_DEPTH: u32 = 64;

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("{0} is inside itself")]
    IntoItself(PathBuf),
    /// A move whose source is already a direct child of `dest`: there is
    /// nothing to do, and doing it anyway (reading it out, writing it back)
    /// would be a needless -- and on a source made unreadable in between,
    /// destructive -- round trip. Distinguished from [`Self::IntoItself`]
    /// because the fix reads differently: "it's already there" rather than
    /// "that would put it inside itself".
    #[error("{0} is already at the destination")]
    AlreadyAtDestination(PathBuf),
    #[error("{0} is gone")]
    Missing(PathBuf),
    #[error("a copy or move needs somewhere to go")]
    NoDestination,
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Work out `Plan` for `kind` over `sources` into `dest`.
pub fn plan(kind: OpKind, sources: &[PathBuf], dest: Option<&Path>) -> Result<Plan, PlanError> {
    match kind {
        OpKind::Copy | OpKind::Move => plan_copy_move(kind, sources, dest),
        OpKind::Delete(_) => Ok(plan_delete(sources)),
        OpKind::Rename => plan_rename(sources, dest),
    }
}

fn io_err(path: &Path, source: std::io::Error) -> PlanError {
    PlanError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// `(dev, ino)`, the identity a case-insensitive filesystem cannot fool: two
/// paths naming the same inode are the same file however differently they
/// are spelled.
fn ino_of(meta: &fs::Metadata) -> (u64, u64) {
    (meta.dev(), meta.ino())
}

/// A path resolved once, for comparison: its canonical form (symlinks and
/// `.`/`..` gone) alongside the inode it names. Used only to compare *whole
/// directories* the way `IntoItself` needs to, never to decide what `exec`
/// walks -- that walk reads `symlink_metadata` and stops at every symlink.
fn identity(path: &Path) -> Result<(PathBuf, (u64, u64)), PlanError> {
    let canon = fs::canonicalize(path).map_err(|e| io_err(path, e))?;
    let meta = fs::metadata(&canon).map_err(|e| io_err(path, e))?;
    Ok((canon, ino_of(&meta)))
}

/// The closest ancestor of `path` (possibly `path` itself) that exists, so a
/// not-yet-created rename target can still be checked against `from` without
/// requiring it to exist first.
fn nearest_existing(path: &Path) -> Option<PathBuf> {
    let mut cur = Some(path);
    while let Some(p) = cur {
        if fs::symlink_metadata(p).is_ok() {
            return Some(p.to_path_buf());
        }
        cur = p.parent();
    }
    None
}

/// Whether `container` names `path` itself or an ancestor of it.
///
/// Checked two ways because either alone can be fooled: the canonical-path
/// prefix is wrong on a case-insensitive filesystem, where `canonicalize`
/// does not normalise the case of a component it did not have to resolve
/// through a symlink (`/Foo` and `/foo` canonicalise to two different
/// strings naming the same directory); the `(dev, ino)` ancestor walk alone
/// would miss a `path` that does not exist yet (a rename target), which has
/// no metadata of its own to compare.
fn contains(container: &(PathBuf, (u64, u64)), path: &(PathBuf, (u64, u64))) -> bool {
    if container.1 == path.1 {
        return true;
    }
    if path.0.starts_with(&container.0) {
        return true;
    }
    let mut cur = path.0.parent();
    while let Some(p) = cur {
        if p == container.0 {
            return true;
        }
        if let Ok(m) = fs::metadata(p) {
            if ino_of(&m) == container.1 {
                return true;
            }
        }
        cur = p.parent();
    }
    false
}

fn kind_of(meta: &fs::Metadata) -> ItemKind {
    let ft = meta.file_type();
    if ft.is_dir() {
        ItemKind::Dir
    } else if ft.is_symlink() {
        ItemKind::Symlink
    } else {
        ItemKind::File(meta.len())
    }
}

fn file_name_of(path: &Path) -> std::ffi::OsString {
    path.file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| path.as_os_str().to_os_string())
}

/// Walk one path -- a whole subtree if it is a directory, one item
/// otherwise -- appending every [`Item`] found to `items` in parent-before-
/// child order and adding every regular file's length to `total_bytes`.
///
/// Best-effort past the first `symlink_metadata`: a directory that becomes
/// unreadable partway through planning (a permission change racing the
/// plan, most plausibly) stops that branch of the walk rather than failing
/// the whole plan -- the items already found are still exec'd, and whatever
/// could not be seen here will surface as a per-item failure when `exec`
/// tries to reach it instead. Nothing here follows a symlink: a directory
/// entry is only ever descended into after its own `symlink_metadata` says
/// `Dir`.
fn walk(
    path: &Path,
    to: Option<PathBuf>,
    depth: u32,
    seen: &mut HashSet<(u64, u64)>,
    items: &mut Vec<Item>,
    total_bytes: &mut u64,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    let kind = kind_of(&meta);
    match kind {
        ItemKind::Symlink => {
            items.push(Item {
                from: path.to_path_buf(),
                to,
                kind,
            });
        }
        ItemKind::File(len) => {
            *total_bytes += len;
            items.push(Item {
                from: path.to_path_buf(),
                to,
                kind,
            });
        }
        ItemKind::Dir => {
            if !seen.insert(ino_of(&meta)) {
                // Already walked this exact directory by another path --
                // a loop (a bind mount nested into itself, say), not a
                // symlink, since a symlink is never descended into.
                return;
            }
            items.push(Item {
                from: path.to_path_buf(),
                to: to.clone(),
                kind,
            });
            let Ok(entries) = fs::read_dir(path) else {
                return;
            };
            for entry in entries.flatten() {
                let child = entry.path();
                let child_to = to.as_ref().map(|t| t.join(entry.file_name()));
                walk(&child, child_to, depth + 1, seen, items, total_bytes);
            }
        }
    }
}

fn plan_copy_move(
    kind: OpKind,
    sources: &[PathBuf],
    dest: Option<&Path>,
) -> Result<Plan, PlanError> {
    let dest = dest.ok_or(PlanError::NoDestination)?;
    let mut plan = Plan {
        dest: dest.to_path_buf(),
        ..Plan::default()
    };
    if sources.is_empty() {
        return Ok(plan);
    }

    let dest_id = identity(dest)?;
    let mut seen = HashSet::new();

    for src in sources {
        let meta = fs::symlink_metadata(src).map_err(|_| PlanError::Missing(src.clone()))?;
        let src_kind = kind_of(&meta);

        if src_kind == ItemKind::Dir {
            let src_id = identity(src)?;
            if contains(&src_id, &dest_id) {
                return Err(PlanError::IntoItself(src.clone()));
            }
        }

        if matches!(kind, OpKind::Move) {
            if let Some(parent) = src.parent() {
                if let Ok(parent_id) = identity(parent) {
                    if parent_id.1 == dest_id.1 {
                        return Err(PlanError::AlreadyAtDestination(src.clone()));
                    }
                }
            }
        }

        let file_name = file_name_of(src);
        let target = dest.join(&file_name);

        if let Ok(target_meta) = fs::symlink_metadata(&target) {
            plan.conflicts.push(Conflict {
                source: src.clone(),
                dest: target.clone(),
                both_dirs: target_meta.is_dir() && meta.is_dir(),
            });
        }

        plan.sources.push(src.clone());
        plan.roots.push(Some(plan.items.len()));
        walk(
            src,
            Some(target),
            0,
            &mut seen,
            &mut plan.items,
            &mut plan.total_bytes,
        );
    }

    plan.total_items = plan.items.len();
    Ok(plan)
}

fn plan_delete(sources: &[PathBuf]) -> Plan {
    let mut plan = Plan::default();
    let mut seen = HashSet::new();

    for src in sources {
        plan.sources.push(src.clone());
        if fs::symlink_metadata(src).is_err() {
            plan.missing.push(src.clone());
            plan.roots.push(None);
            continue;
        }
        plan.roots.push(Some(plan.items.len()));
        walk(
            src,
            None,
            0,
            &mut seen,
            &mut plan.items,
            &mut plan.total_bytes,
        );
    }

    plan.total_items = plan.items.len();
    plan
}

fn plan_rename(sources: &[PathBuf], dest: Option<&Path>) -> Result<Plan, PlanError> {
    let from = sources.first().ok_or(PlanError::NoDestination)?;
    let to = dest.ok_or(PlanError::NoDestination)?;

    let meta = fs::symlink_metadata(from).map_err(|_| PlanError::Missing(from.clone()))?;
    let kind = kind_of(&meta);

    if kind == ItemKind::Dir {
        let from_id = identity(from)?;
        if let Some(anchor) = nearest_existing(to) {
            if let Ok(anchor_id) = identity(&anchor) {
                if contains(&from_id, &anchor_id) {
                    return Err(PlanError::IntoItself(from.clone()));
                }
            }
        }
    }

    let mut conflicts = Vec::new();
    if let Ok(to_meta) = fs::symlink_metadata(to) {
        let same_inode = ino_of(&meta) == ino_of(&to_meta);
        if !same_inode {
            conflicts.push(Conflict {
                source: from.clone(),
                dest: to.to_path_buf(),
                both_dirs: to_meta.is_dir() && meta.is_dir(),
            });
        }
    }

    Ok(Plan {
        sources: vec![from.clone()],
        dest: to.to_path_buf(),
        total_bytes: 0,
        total_items: 1,
        conflicts,
        items: vec![Item {
            from: from.clone(),
            to: Some(to.to_path_buf()),
            kind,
        }],
        roots: vec![Some(0)],
        missing: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fold::testing::Fixture;

    fn kind_dir_file_symlink_counts(items: &[Item]) -> (usize, usize, usize) {
        let mut dirs = 0;
        let mut files = 0;
        let mut symlinks = 0;
        for item in items {
            match item.kind {
                ItemKind::Dir => dirs += 1,
                ItemKind::File(_) => files += 1,
                ItemKind::Symlink => symlinks += 1,
            }
        }
        (dirs, files, symlinks)
    }

    #[test]
    fn planning_a_copy_totals_bytes_and_items_over_a_nested_tree() {
        let fx = Fixture::tree();
        let dest = fx.path("pictures");
        let src = fx.path("projects");

        let result = plan(OpKind::Copy, std::slice::from_ref(&src), Some(&dest)).unwrap();

        let expected_bytes = fs::metadata(fx.path("projects/starwire/src/main.rs"))
            .unwrap()
            .len()
            + fs::metadata(fx.path("projects/starwire/Cargo.toml"))
                .unwrap()
                .len()
            + fs::metadata(fx.path("projects/starwire/README.md"))
                .unwrap()
                .len()
            + fs::metadata(fx.path("projects/starwire/.gitignore"))
                .unwrap()
                .len();
        assert_eq!(result.total_bytes, expected_bytes);

        // projects, starwire, src, target = 4 dirs; main.rs, Cargo.toml,
        // README.md, .gitignore = 4 files; nothing under `projects` is a
        // symlink.
        let (dirs, files, symlinks) = kind_dir_file_symlink_counts(&result.items);
        assert_eq!(dirs, 4);
        assert_eq!(files, 4);
        assert_eq!(symlinks, 0);
        assert_eq!(result.total_items, result.items.len());
        assert_eq!(result.roots, vec![Some(0)]);
    }

    #[test]
    fn a_symlink_is_one_item_and_its_target_is_not_walked() {
        let fx = Fixture::tree();
        let dest = fx.path("empty");
        // `notes.txt` is a symlink to `projects/starwire/README.md`.
        let src = fx.path("notes.txt");

        let result = plan(OpKind::Copy, &[src], Some(&dest)).unwrap();

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].kind, ItemKind::Symlink);
        assert_eq!(result.total_bytes, 0, "the target's bytes are not counted");
    }

    #[test]
    fn a_symlink_loop_terminates() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::os::unix::fs::symlink(&b, &a).unwrap();
        std::os::unix::fs::symlink(&a, &b).unwrap();
        let dest = dir.path().join("dest");
        fs::create_dir(&dest).unwrap();

        // Each of `a` and `b` is a symlink -- never followed, so the "loop"
        // is never actually entered -- and this returns promptly rather than
        // hanging.
        let result = plan(OpKind::Copy, &[a, b], Some(&dest)).unwrap();
        assert_eq!(result.items.len(), 2);
        assert!(result.items.iter().all(|i| i.kind == ItemKind::Symlink));
    }

    #[test]
    fn copying_a_directory_into_itself_is_refused() {
        let fx = Fixture::tree();
        let src = fx.path("projects");
        let err = plan(OpKind::Copy, std::slice::from_ref(&src), Some(&src)).unwrap_err();
        assert!(matches!(err, PlanError::IntoItself(p) if p == src));
    }

    #[test]
    fn copying_a_directory_into_its_own_descendant_is_refused() {
        let fx = Fixture::tree();
        let src = fx.path("projects");
        let descendant = fx.path("projects/starwire");
        let err = plan(OpKind::Copy, std::slice::from_ref(&src), Some(&descendant)).unwrap_err();
        assert!(matches!(err, PlanError::IntoItself(p) if p == src));
    }

    #[test]
    fn moving_a_source_already_inside_the_destination_is_refused_as_nothing_to_do() {
        let fx = Fixture::tree();
        let dest = fx.path("projects");
        let src = fx.path("projects/starwire");
        let err = plan(OpKind::Move, std::slice::from_ref(&src), Some(&dest)).unwrap_err();
        assert!(matches!(err, PlanError::AlreadyAtDestination(p) if p == src));
    }

    #[test]
    fn a_conflict_is_found_for_a_name_that_already_exists_at_the_destination() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("dest");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("blob.bin"), b"already here").unwrap();
        let colliding = dir.path().join("blob.bin");
        fs::write(&colliding, b"incoming").unwrap();

        let result = plan(OpKind::Copy, &[colliding], Some(&dest)).unwrap();
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].dest, dest.join("blob.bin"));
        assert!(!result.conflicts[0].both_dirs);
    }

    #[test]
    fn a_case_only_rename_is_not_a_conflict_on_a_case_insensitive_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        let lower = dir.path().join("a");
        std::fs::write(&lower, b"x").unwrap();
        let upper = dir.path().join("A");
        let case_insensitive = fs::symlink_metadata(&upper).is_ok();
        if !case_insensitive {
            println!("skipping: this filesystem is case-sensitive");
            return;
        }

        let result = plan(OpKind::Rename, &[lower], Some(&upper)).unwrap();
        assert!(
            result.conflicts.is_empty(),
            "the same inode under a different case is not a conflict"
        );
    }

    #[test]
    fn a_missing_source_is_reported_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nowhere");
        let dest = dir.path().join("dest");
        std::fs::create_dir(&dest).unwrap();
        let err = plan(OpKind::Copy, std::slice::from_ref(&missing), Some(&dest)).unwrap_err();
        assert!(matches!(err, PlanError::Missing(p) if p == missing));
    }

    #[test]
    fn a_missing_delete_source_is_recorded_but_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nowhere");
        let present = dir.path().join("here.txt");
        std::fs::write(&present, b"x").unwrap();

        let result = plan(
            OpKind::Delete(super::super::DeleteHow::Trash),
            &[missing.clone(), present.clone()],
            None,
        )
        .unwrap();

        assert_eq!(result.missing, vec![missing]);
        assert_eq!(result.roots, vec![None, Some(0)]);
        assert_eq!(result.items.len(), 1);
    }

    #[test]
    fn renaming_refuses_a_target_inside_the_source_directory() {
        let fx = Fixture::tree();
        let from = fx.path("projects");
        let to = fx.path("projects/starwire/renamed");
        let err = plan(OpKind::Rename, std::slice::from_ref(&from), Some(&to)).unwrap_err();
        assert!(matches!(err, PlanError::IntoItself(p) if p == from));
    }

    #[test]
    fn a_deep_tree_stops_descending_at_the_depth_cap() {
        let dir = tempfile::tempdir().unwrap();
        let mut cur = dir.path().to_path_buf();
        for i in 0..(MAX_DEPTH as usize + 10) {
            cur = cur.join(format!("d{i}"));
        }
        fs::create_dir_all(&cur).unwrap();
        let dest = dir.path().join("dest");
        fs::create_dir(&dest).unwrap();

        let top = dir.path().join("d0");
        let result = plan(OpKind::Copy, &[top], Some(&dest)).unwrap();
        // One item per level up to the cap, not the full 74.
        assert!(result.items.len() <= MAX_DEPTH as usize + 1);
    }
}
