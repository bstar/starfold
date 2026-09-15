//! Running a planned operation: the bytes actually moving.
//!
//! `run` never re-reads a directory -- everything it touches is already
//! named in [`super::Plan::items`], in the order `plan` found it. It copies
//! file by file with `std::fs::copy` (which keeps xattrs and, on APFS,
//! clones rather than duplicates the data), recreates a symlink rather than
//! following it, renames a top-level source where it can and falls back to
//! copy-then-delete across a device boundary, and checks
//! `progress.is_cancelled()` between every item so a running op notices a
//! `Command::Cancel` promptly rather than only between top-level sources.

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use super::progress::Progress;
use super::{Conflict, ConflictPolicy, DeleteHow, Item, ItemKind, OpKind, Outcome, Plan};

/// Knobs `run` takes from the configuration, and one for the tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct RunOptions {
    pub preserve_times: bool,
    /// Copy-then-delete even where a rename would do, so the cross-device
    /// fallback path is exercised on one filesystem in tests.
    pub force_copy: bool,
}

/// Run `plan`. Never panics on a bad file; what failed is in the `Outcome`.
pub fn run(
    kind: OpKind,
    plan: &Plan,
    policy: ConflictPolicy,
    options: &RunOptions,
    progress: &Progress,
) -> Outcome {
    match kind {
        OpKind::Copy | OpKind::Move => run_copy_move(kind, plan, policy, options, progress),
        OpKind::Delete(DeleteHow::Trash) => run_trash_delete(plan, progress),
        OpKind::Delete(DeleteHow::Permanent) => run_permanent_delete(plan, progress),
        OpKind::Rename => run_rename(plan, policy, progress),
    }
}

fn ino_of(meta: &fs::Metadata) -> (u64, u64) {
    (meta.dev(), meta.ino())
}

/// The half-open range of `plan.items` that is `sources[i]`'s own subtree:
/// from its root up to (but not including) the next present source's root,
/// or the end of `items` for the last one. Relies on `plan`'s own invariant
/// that each source's walk is written out as one contiguous run.
fn subtree_range(plan: &Plan, i: usize) -> Option<(usize, usize)> {
    let start = plan.roots.get(i).copied().flatten()?;
    let end = plan.roots[i + 1..]
        .iter()
        .find_map(|r| *r)
        .unwrap_or(plan.items.len());
    Some((start, end))
}

/// Remove whatever is at `target` so a fresh copy, move or rename can take
/// its place -- `ConflictPolicy::Overwrite` on a top-level conflict that is
/// not two directories merging into each other.
fn replace_existing(target: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(target).map_err(|e| format!("{}: {e}", target.display()))?;
    let result = if meta.is_dir() {
        fs::remove_dir_all(target)
    } else {
        fs::remove_file(target)
    };
    result.map_err(|e| format!("{}: {e}", target.display()))
}

/// `stem (n).ext` for `n = 1, 2, ...`, pure over a set of names already
/// taken -- the core `RenameNew` uses, shared with the proptest below so
/// both exercise the same logic.
fn candidate_name(stem: &str, ext: Option<&str>, n: u64) -> String {
    match ext {
        Some(ext) if !ext.is_empty() => format!("{stem} ({n}).{ext}"),
        _ => format!("{stem} ({n})"),
    }
}

fn first_free_in_set(
    existing: &std::collections::HashSet<String>,
    stem: &str,
    ext: Option<&str>,
) -> String {
    for n in 1u64.. {
        let candidate = candidate_name(stem, ext, n);
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!("u64 does not run out of candidates")
}

/// The disk-backed form of [`first_free_in_set`]: the first `stem (n).ext`
/// that does not already exist in `dir`.
fn first_free_on_disk(dir: &Path, stem: &str, ext: Option<&str>) -> String {
    for n in 1u64.. {
        let candidate = candidate_name(stem, ext, n);
        if fs::symlink_metadata(dir.join(&candidate)).is_err() {
            return candidate;
        }
    }
    unreachable!("u64 does not run out of candidates")
}

fn stem_and_ext(name: &std::ffi::OsStr) -> (String, Option<String>) {
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .unwrap_or(name)
        .to_string_lossy()
        .into_owned();
    let ext = path.extension().map(|e| e.to_string_lossy().into_owned());
    (stem, ext)
}

/// Rewrite every `to` in `items` from under `from_prefix` to under
/// `to_prefix` -- what `RenameNew` needs after `plan` already computed every
/// destination path against the original, now-conflicting name.
fn remap_to(items: &[Item], from_prefix: &Path, to_prefix: &Path) -> Vec<Item> {
    items
        .iter()
        .map(|item| {
            let to = item.to.as_ref().map(|t| {
                let rel = t.strip_prefix(from_prefix).unwrap_or(t);
                if rel.as_os_str().is_empty() {
                    to_prefix.to_path_buf()
                } else {
                    to_prefix.join(rel)
                }
            });
            Item {
                from: item.from.clone(),
                to,
                kind: item.kind,
            }
        })
        .collect()
}

/// Copy every item in `items` (already in parent-before-child order) to its
/// `to`. Stops at the first failure -- the caller attributes it to the
/// top-level source and moves on to the next one, rather than this function
/// trying to keep copying a subtree whose parent directory may itself be
/// the reason a child could not be reached.
fn copy_subtree(items: &[Item], options: &RunOptions, progress: &Progress) -> Result<(), String> {
    for item in items {
        if progress.is_cancelled() {
            return Err("cancelled".into());
        }
        let to = item
            .to
            .as_ref()
            .ok_or_else(|| format!("{}: no destination", item.from.display()))?;
        match item.kind {
            ItemKind::Dir => {
                fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
                if let Ok(meta) = fs::symlink_metadata(&item.from) {
                    let _ = fs::set_permissions(to, meta.permissions());
                }
            }
            ItemKind::File(len) => {
                fs::copy(&item.from, to).map_err(|e| format!("{}: {e}", item.from.display()))?;
                if options.preserve_times {
                    if let Ok(src_meta) = fs::metadata(&item.from) {
                        if let Ok(modified) = src_meta.modified() {
                            let accessed = src_meta.accessed().unwrap_or(modified);
                            let times = fs::FileTimes::new()
                                .set_modified(modified)
                                .set_accessed(accessed);
                            if let Ok(f) = fs::OpenOptions::new().write(true).open(to) {
                                let _ = f.set_times(times);
                            }
                        }
                    }
                }
                progress.add(len);
            }
            ItemKind::Symlink => {
                let target = fs::read_link(&item.from)
                    .map_err(|e| format!("{}: {e}", item.from.display()))?;
                if fs::symlink_metadata(to).is_ok() {
                    let _ = fs::remove_file(to);
                }
                std::os::unix::fs::symlink(&target, to)
                    .map_err(|e| format!("{}: {e}", to.display()))?;
            }
        }
    }
    Ok(())
}

fn remove_source(source: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(source).map_err(|e| format!("{}: {e}", source.display()))?;
    let result = if meta.is_dir() {
        fs::remove_dir_all(source)
    } else {
        // A file, or a symlink -- `remove_file` removes the link itself and
        // never the target, exactly like a plain `unlink(2)`.
        fs::remove_file(source)
    };
    result.map_err(|e| format!("{}: {e}", source.display()))
}

/// `rename`, falling back to copy-then-delete on `ErrorKind::CrossesDevices`
/// (`EXDEV`) or when `options.force_copy` asks for that path unconditionally
/// -- the source is only ever removed once every item under it has copied
/// cleanly.
fn move_subtree(
    source: &Path,
    items: &[Item],
    options: &RunOptions,
    progress: &Progress,
) -> Result<(), String> {
    let target = items
        .first()
        .and_then(|it| it.to.clone())
        .ok_or_else(|| format!("{}: no destination", source.display()))?;

    if !options.force_copy {
        match fs::rename(source, &target) {
            Ok(()) => {
                let bytes: u64 = items
                    .iter()
                    .map(|it| match it.kind {
                        ItemKind::File(n) => n,
                        _ => 0,
                    })
                    .sum();
                progress.add(bytes);
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {}
            Err(e) if e.raw_os_error() == Some(18) => {
                // EXDEV is 18 on both Linux and Darwin (POSIX does not fix
                // the value, but these two agree); kept as a fallback in
                // case `ErrorKind::CrossesDevices` is ever not what the
                // platform reports for it.
            }
            Err(e) => return Err(format!("{}: {e}", source.display())),
        }
    }

    copy_subtree(items, options, progress)?;
    remove_source(source)
}

fn run_copy_move(
    kind: OpKind,
    plan: &Plan,
    policy: ConflictPolicy,
    options: &RunOptions,
    progress: &Progress,
) -> Outcome {
    progress.set_total(plan.total_bytes);
    let mut outcome = Outcome::default();
    let conflict_by_source: HashMap<&Path, &Conflict> = plan
        .conflicts
        .iter()
        .map(|c| (c.source.as_path(), c))
        .collect();

    for i in 0..plan.sources.len() {
        if progress.is_cancelled() {
            outcome.cancelled = true;
            break;
        }
        let source = &plan.sources[i];
        let Some((start, end)) = subtree_range(plan, i) else {
            outcome.failed.push((source.clone(), "not planned".into()));
            continue;
        };
        let items = &plan.items[start..end];
        let Some(mut target) = items.first().and_then(|it| it.to.clone()) else {
            outcome
                .failed
                .push((source.clone(), "no destination".into()));
            continue;
        };

        let mut owned_items: Option<Vec<Item>> = None;

        if let Some(conflict) = conflict_by_source.get(source.as_path()) {
            match policy {
                ConflictPolicy::Ask => {
                    // A conflict is never supposed to reach `exec` under
                    // `Ask` -- the queue stops at `OpStatus::NeedsPolicy`
                    // and waits for `Command::SetPolicy` -- so arriving here
                    // is a logic error upstream, worth a failure that says
                    // so rather than a silent skip.
                    outcome
                        .failed
                        .push((source.clone(), "unanswered conflict".into()));
                    continue;
                }
                ConflictPolicy::Skip => {
                    outcome.skipped += 1;
                    continue;
                }
                ConflictPolicy::Overwrite => {
                    if !conflict.both_dirs {
                        if let Err(e) = replace_existing(&target) {
                            outcome.failed.push((source.clone(), e));
                            continue;
                        }
                    }
                }
                ConflictPolicy::RenameNew => {
                    let file_name = target.file_name().unwrap_or_default().to_os_string();
                    let (stem, ext) = stem_and_ext(&file_name);
                    let new_name = first_free_on_disk(&plan.dest, &stem, ext.as_deref());
                    let new_target = plan.dest.join(&new_name);
                    owned_items = Some(remap_to(items, &target, &new_target));
                    target = new_target;
                }
            }
        }
        let _ = &target;

        let items_ref: &[Item] = owned_items.as_deref().unwrap_or(items);
        let result = match kind {
            OpKind::Copy => copy_subtree(items_ref, options, progress),
            OpKind::Move => move_subtree(source, items_ref, options, progress),
            _ => unreachable!("run_copy_move only handles Copy and Move"),
        };
        match result {
            Ok(()) => outcome.done += 1,
            // A cancellation mid-subtree surfaces from `copy_subtree` or
            // `move_subtree` as an ordinary `Err`, the same as a real
            // failure -- distinguished here, after the fact, by asking
            // `progress` itself rather than by matching the error's text,
            // so the run stops for good (like the top-of-loop check above)
            // instead of recording a spurious per-source failure and moving
            // on to the next one.
            Err(_) if progress.is_cancelled() => {
                outcome.cancelled = true;
                break;
            }
            Err(e) => outcome.failed.push((source.clone(), e)),
        }
    }

    outcome
}

fn run_trash_delete(plan: &Plan, progress: &Progress) -> Outcome {
    progress.set_total(plan.total_items as u64);
    let mut outcome = Outcome::default();
    outcome.skipped += plan.missing.len();

    let present: Vec<PathBuf> = plan
        .sources
        .iter()
        .filter(|s| !plan.missing.contains(s))
        .cloned()
        .collect();
    if present.is_empty() {
        return outcome;
    }
    if progress.is_cancelled() {
        outcome.cancelled = true;
        return outcome;
    }

    match super::trash::delete(&present) {
        Ok(()) => {
            outcome.done += present.len();
            progress.add(plan.total_items as u64);
        }
        Err((path, msg)) => outcome.failed.push((path, msg)),
    }
    outcome
}

fn run_permanent_delete(plan: &Plan, progress: &Progress) -> Outcome {
    progress.set_total(plan.total_items as u64);
    let mut outcome = Outcome::default();
    outcome.skipped += plan.missing.len();

    for i in 0..plan.sources.len() {
        let source = &plan.sources[i];
        if plan.missing.contains(source) {
            continue;
        }
        if progress.is_cancelled() {
            outcome.cancelled = true;
            break;
        }
        let count = subtree_range(plan, i)
            .map(|(start, end)| (end - start) as u64)
            .unwrap_or(0);
        match remove_source(source) {
            Ok(()) => {
                outcome.done += 1;
                progress.add(count);
            }
            Err(e) => outcome.failed.push((source.clone(), e)),
        }
    }
    outcome
}

/// Rename `from` to `to` on a case-insensitive filesystem where the two
/// names resolve to the same entry (`Foo` -> `foo`): a direct `rename`
/// would be a no-op there (the kernel sees no work to do, or in the worst
/// case treats it as deleting what it was asked to create), so the change is
/// made in two hops through a temporary sibling name that cannot collide
/// with either.
fn rename_case_only(from: &Path, to: &Path) -> Result<(), String> {
    let parent = from.parent().unwrap_or_else(|| Path::new("."));
    let tmp = parent.join(format!(".starfold-rename-{}", std::process::id()));
    fs::rename(from, &tmp).map_err(|e| format!("{}: {e}", from.display()))?;
    fs::rename(&tmp, to).map_err(|e| {
        // Put it back under the original name rather than leaving it
        // stranded under the temporary one.
        let _ = fs::rename(&tmp, from);
        format!("{}: {e}", to.display())
    })
}

fn run_rename(plan: &Plan, policy: ConflictPolicy, progress: &Progress) -> Outcome {
    progress.set_total(plan.total_items.max(1) as u64);
    let mut outcome = Outcome::default();

    let Some(item) = plan.items.first() else {
        outcome
            .failed
            .push((PathBuf::new(), "nothing to rename".into()));
        return outcome;
    };
    let from = item.from.clone();
    let Some(mut to) = item.to.clone() else {
        outcome.failed.push((from, "no destination".into()));
        return outcome;
    };

    if progress.is_cancelled() {
        outcome.cancelled = true;
        return outcome;
    }

    if let Some(conflict) = plan.conflicts.first() {
        match policy {
            ConflictPolicy::Ask => {
                outcome.failed.push((from, "unanswered conflict".into()));
                return outcome;
            }
            ConflictPolicy::Skip => {
                outcome.skipped += 1;
                return outcome;
            }
            ConflictPolicy::Overwrite => {
                if !conflict.both_dirs {
                    if let Err(e) = replace_existing(&to) {
                        outcome.failed.push((from, e));
                        return outcome;
                    }
                }
            }
            ConflictPolicy::RenameNew => {
                let file_name = to.file_name().unwrap_or_default().to_os_string();
                let (stem, ext) = stem_and_ext(&file_name);
                let dir = to.parent().unwrap_or_else(|| Path::new("."));
                let new_name = first_free_on_disk(dir, &stem, ext.as_deref());
                to = dir.join(new_name);
            }
        }
    }

    let case_only_same_inode = fs::symlink_metadata(&from)
        .ok()
        .zip(fs::symlink_metadata(&to).ok())
        .map(|(a, b)| ino_of(&a) == ino_of(&b))
        .unwrap_or(false);

    let result = if case_only_same_inode && from != to {
        rename_case_only(&from, &to)
    } else {
        fs::rename(&from, &to).map_err(|e| format!("{}: {e}", to.display()))
    };

    match result {
        Ok(()) => {
            outcome.done += 1;
            progress.add(plan.total_items as u64);
        }
        Err(e) => outcome.failed.push((from, e)),
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fold::ops::plan::plan as make_plan;
    use crate::fold::ops::DeleteHow;
    use std::collections::HashSet as StdHashSet;
    use std::os::unix::fs::PermissionsExt as _;

    fn progress() -> Progress {
        Progress::new(0)
    }

    fn options() -> RunOptions {
        RunOptions {
            preserve_times: true,
            force_copy: false,
        }
    }

    /// Every regular file's bytes under `dir`, and every mode bit, compared
    /// recursively -- the general-purpose check most of the copy tests below
    /// build on.
    fn assert_trees_equal(a: &Path, b: &Path, check_times: bool) {
        let mut a_entries: Vec<_> = fs::read_dir(a).unwrap().flatten().collect();
        let mut b_entries: Vec<_> = fs::read_dir(b).unwrap().flatten().collect();
        a_entries.sort_by_key(|e| e.file_name());
        b_entries.sort_by_key(|e| e.file_name());
        assert_eq!(
            a_entries.iter().map(|e| e.file_name()).collect::<Vec<_>>(),
            b_entries.iter().map(|e| e.file_name()).collect::<Vec<_>>(),
            "{} and {} do not have the same names",
            a.display(),
            b.display()
        );
        for (ea, eb) in a_entries.iter().zip(b_entries.iter()) {
            let pa = ea.path();
            let pb = eb.path();
            let ma = fs::symlink_metadata(&pa).unwrap();
            let mb = fs::symlink_metadata(&pb).unwrap();
            if ma.file_type().is_symlink() {
                assert!(mb.file_type().is_symlink());
                assert_eq!(fs::read_link(&pa).unwrap(), fs::read_link(&pb).unwrap());
                continue;
            }
            if ma.is_dir() {
                assert!(mb.is_dir());
                assert_trees_equal(&pa, &pb, check_times);
                continue;
            }
            assert_eq!(
                fs::read(&pa).unwrap(),
                fs::read(&pb).unwrap(),
                "{}",
                pa.display()
            );
            assert_eq!(
                ma.permissions().mode() & 0o777,
                mb.permissions().mode() & 0o777,
                "{}",
                pa.display()
            );
            if check_times {
                assert_eq!(
                    ma.modified().unwrap(),
                    mb.modified().unwrap(),
                    "{}",
                    pa.display()
                );
            }
        }
    }

    fn small_tree(root: &Path) {
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("a.txt"), b"hello a").unwrap();
        fs::write(root.join("sub/b.txt"), b"hello b").unwrap();
        std::os::unix::fs::symlink("a.txt", root.join("link")).unwrap();
    }

    #[test]
    fn a_copied_tree_matches_the_source_byte_for_byte_and_mode_for_mode() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir(&src).unwrap();
        small_tree(&src);
        fs::set_permissions(src.join("a.txt"), fs::Permissions::from_mode(0o640)).unwrap();
        let dest = dir.path().join("dest");
        fs::create_dir(&dest).unwrap();

        let p = make_plan(OpKind::Copy, std::slice::from_ref(&src), Some(&dest)).unwrap();
        let outcome = run(
            OpKind::Copy,
            &p,
            ConflictPolicy::Ask,
            &options(),
            &progress(),
        );

        assert_eq!(outcome.done, 1);
        assert!(outcome.failed.is_empty());
        assert_trees_equal(&src, &dest.join("src"), true);
    }

    #[test]
    fn symlinks_are_recreated_as_symlinks_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir(&src).unwrap();
        small_tree(&src);
        let dest = dir.path().join("dest");
        fs::create_dir(&dest).unwrap();

        let p = make_plan(OpKind::Copy, std::slice::from_ref(&src), Some(&dest)).unwrap();
        run(
            OpKind::Copy,
            &p,
            ConflictPolicy::Ask,
            &options(),
            &progress(),
        );

        let copied_link = dest.join("src/link");
        assert!(fs::symlink_metadata(&copied_link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_link(&copied_link).unwrap(), Path::new("a.txt"));
    }

    #[test]
    fn a_same_device_move_keeps_the_inode() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("file.txt");
        fs::write(&src, b"move me").unwrap();
        let src_ino = fs::metadata(&src).unwrap().ino();
        let dest = dir.path().join("dest");
        fs::create_dir(&dest).unwrap();

        let p = make_plan(OpKind::Move, std::slice::from_ref(&src), Some(&dest)).unwrap();
        let outcome = run(
            OpKind::Move,
            &p,
            ConflictPolicy::Ask,
            &options(),
            &progress(),
        );

        assert_eq!(outcome.done, 1);
        assert!(!src.exists());
        let moved = dest.join("file.txt");
        assert_eq!(fs::metadata(&moved).unwrap().ino(), src_ino);
    }

    #[test]
    fn a_forced_copy_move_produces_an_identical_tree_and_removes_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir(&src).unwrap();
        small_tree(&src);
        let dest = dir.path().join("dest");
        fs::create_dir(&dest).unwrap();
        // A copy of the tree taken before the move, to compare the moved
        // copy against once `src` is gone.
        let reference = dir.path().join("reference");
        fs::create_dir(&reference).unwrap();
        small_tree(&reference);

        let p = make_plan(OpKind::Move, std::slice::from_ref(&src), Some(&dest)).unwrap();
        let opts = RunOptions {
            preserve_times: false,
            force_copy: true,
        };
        let outcome = run(OpKind::Move, &p, ConflictPolicy::Ask, &opts, &progress());

        assert_eq!(outcome.done, 1);
        assert!(!src.exists(), "the source is removed once copied");
        assert_trees_equal(&reference, &dest.join("src"), false);
    }

    #[test]
    fn a_top_level_conflict_can_be_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("dest");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("a.txt"), b"already here").unwrap();
        let src = dir.path().join("a.txt");
        fs::write(&src, b"incoming").unwrap();

        let p = make_plan(OpKind::Copy, &[src], Some(&dest)).unwrap();
        assert_eq!(p.conflicts.len(), 1);
        let outcome = run(
            OpKind::Copy,
            &p,
            ConflictPolicy::Skip,
            &options(),
            &progress(),
        );

        assert_eq!(outcome.skipped, 1);
        assert_eq!(outcome.done, 0);
        assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"already here");
    }

    #[test]
    fn a_top_level_conflict_can_be_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("dest");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("a.txt"), b"already here").unwrap();
        let src = dir.path().join("a.txt");
        fs::write(&src, b"incoming").unwrap();

        let p = make_plan(OpKind::Copy, &[src], Some(&dest)).unwrap();
        let outcome = run(
            OpKind::Copy,
            &p,
            ConflictPolicy::Overwrite,
            &options(),
            &progress(),
        );

        assert_eq!(outcome.done, 1);
        assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"incoming");
    }

    #[test]
    fn a_top_level_conflict_can_be_resolved_by_picking_a_new_name() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("dest");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("a.txt"), b"already here").unwrap();
        let src = dir.path().join("a.txt");
        fs::write(&src, b"incoming").unwrap();

        let p = make_plan(OpKind::Copy, &[src], Some(&dest)).unwrap();
        let outcome = run(
            OpKind::Copy,
            &p,
            ConflictPolicy::RenameNew,
            &options(),
            &progress(),
        );

        assert_eq!(outcome.done, 1);
        assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"already here");
        assert_eq!(fs::read(dest.join("a (1).txt")).unwrap(), b"incoming");
    }

    #[test]
    fn rename_new_picks_the_first_free_numbered_name_then_the_next() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), b"0").unwrap();
        fs::write(dir.path().join("a (1).txt"), b"1").unwrap();
        assert_eq!(
            first_free_on_disk(dir.path(), "a", Some("txt")),
            "a (2).txt"
        );
    }

    #[test]
    fn permanent_delete_removes_a_tree_and_a_symlink_without_following_it() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.txt");
        fs::write(&target, b"do not delete me").unwrap();
        let tree = dir.path().join("tree");
        fs::create_dir(&tree).unwrap();
        small_tree(&tree);
        let link = dir.path().join("link-to-target");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let p = make_plan(
            OpKind::Delete(DeleteHow::Permanent),
            &[tree.clone(), link.clone()],
            None,
        )
        .unwrap();
        let outcome = run(
            OpKind::Delete(DeleteHow::Permanent),
            &p,
            ConflictPolicy::Ask,
            &options(),
            &progress(),
        );

        assert_eq!(outcome.done, 2);
        assert!(!tree.exists());
        assert!(fs::symlink_metadata(&link).is_err(), "the link is gone");
        assert!(target.exists(), "the link's target is untouched");
    }

    #[test]
    fn cancelling_mid_run_stops_before_every_file_is_copied() {
        // 200 *top-level* sources rather than one directory holding 200
        // files: `Outcome.done` counts completed top-level sources, so this
        // is what makes "cancelled and done < 200" a meaningful assertion
        // about how far the run got, not just about a single source's
        // subtree. Each file is large enough that copying all 200 takes
        // measurably longer than spinning up the watcher thread below and
        // noticing the first file's bytes land -- with tiny files the whole
        // run can finish before a freshly spawned thread is ever scheduled,
        // which is what made an earlier version of this test flaky.
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir(&src).unwrap();
        let file_bytes = vec![b'x'; 200_000];
        let mut sources = Vec::with_capacity(200);
        for i in 0..200 {
            let path = src.join(format!("f{i}.txt"));
            fs::write(&path, &file_bytes).unwrap();
            sources.push(path);
        }
        let dest = dir.path().join("dest");
        fs::create_dir(&dest).unwrap();

        let p = make_plan(OpKind::Copy, &sources, Some(&dest)).unwrap();
        let prog = Progress::new(0);

        let outcome = std::thread::scope(|scope| {
            scope.spawn(|| {
                while prog.done() == 0 {
                    std::hint::spin_loop();
                }
                prog.cancel();
            });
            run(OpKind::Copy, &p, ConflictPolicy::Ask, &options(), &prog)
        });

        assert!(outcome.cancelled);
        assert!(outcome.done < 200, "done was {}", outcome.done);
    }

    #[test]
    fn an_unreadable_source_directory_fails_its_own_entry_and_the_rest_still_copies() {
        if std::env::var("USER").as_deref() == Ok("root") {
            println!("skipping: permission checks do not apply as root");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad");
        fs::create_dir(&bad).unwrap();
        fs::write(bad.join("inner.txt"), b"unreachable").unwrap();
        let good = dir.path().join("good.txt");
        fs::write(&good, b"reachable").unwrap();
        let dest = dir.path().join("dest");
        fs::create_dir(&dest).unwrap();

        let p = make_plan(OpKind::Copy, &[bad.clone(), good.clone()], Some(&dest)).unwrap();

        fs::set_permissions(&bad, fs::Permissions::from_mode(0o000)).unwrap();
        // Detect whether removing every permission bit actually blocks
        // access here (it does not under some sandboxes or as an
        // effectively privileged user); skip cleanly rather than asserting
        // a failure that cannot happen.
        let actually_blocked = fs::metadata(bad.join("inner.txt")).is_err();
        if !actually_blocked {
            fs::set_permissions(&bad, fs::Permissions::from_mode(0o755)).unwrap();
            println!("skipping: removing all permission bits did not block access here");
            return;
        }

        let outcome = run(
            OpKind::Copy,
            &p,
            ConflictPolicy::Ask,
            &options(),
            &progress(),
        );
        fs::set_permissions(&bad, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(!outcome.failed.is_empty(), "the unreadable source failed");
        assert_eq!(outcome.done, 1, "the good source still copied");
        assert!(dest.join("good.txt").exists());
    }

    #[test]
    fn a_case_only_rename_goes_through_the_temporary_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("Foo");
        fs::write(&from, b"x").unwrap();
        let to = dir.path().join("foo");
        let case_insensitive = fs::symlink_metadata(&to).is_ok();
        if !case_insensitive {
            println!("skipping: this filesystem is case-sensitive");
            return;
        }
        let ino_before = fs::metadata(&from).unwrap().ino();

        let p = make_plan(OpKind::Rename, &[from], Some(&to)).unwrap();
        let outcome = run(
            OpKind::Rename,
            &p,
            ConflictPolicy::Ask,
            &options(),
            &progress(),
        );

        assert_eq!(outcome.done, 1);
        assert_eq!(fs::metadata(&to).unwrap().ino(), ino_before);
        assert_eq!(fs::read(&to).unwrap(), b"x");
    }

    proptest::proptest! {
        #[test]
        fn rename_new_never_collides_and_keeps_the_extension(
            stem in "[a-zA-Z0-9_]{1,10}",
            ext in proptest::option::of("[a-z]{1,4}"),
            existing_count in 0usize..15,
        ) {
            let mut existing: StdHashSet<String> = StdHashSet::new();
            for n in 1..=(existing_count as u64) {
                existing.insert(candidate_name(&stem, ext.as_deref(), n));
            }
            let picked = first_free_in_set(&existing, &stem, ext.as_deref());
            proptest::prop_assert!(!existing.contains(&picked));
            if let Some(e) = &ext {
                let suffix = std::format!(".{}", e);
                proptest::prop_assert!(picked.ends_with(&suffix));
            }
        }
    }
}
