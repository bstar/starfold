# STAR/FOLD file manager roadmap

This plan records the gap analysis against ranger, lf, vifm, Midnight
Commander, GNOME Files (Nautilus), and Thunar. It is a sequence of independently
testable changes, not a promise to copy every feature of those applications.
The priority is daily file work while preserving STAR/FOLD's Fold stack,
persistent marks, inspectable operations queue, Commander, and previews.

## Delivery rule

Complete one milestone at a time. Each feature milestone ends with:

1. Core tests against temporary file trees, including failures and conflicts.
2. UI fixture or snapshot coverage at 100×30 and 60×21 where the display
   changes. The fake core must exercise the real worker path.
3. The repository's formatting, Clippy, test, and release-build checks through
   `nix develop` (see [AGENTS.md](../AGENTS.md) and CI for exact commands).
4. A release binary verified as the target of the local `starfold` command,
   followed by a short, reproducible manual test in a disposable tree.
5. Updated key, CLI, configuration, and status documentation as applicable;
   one reviewable commit; and a report of results and limitations.

Use `STARFOLD_DIR` for manual tests so STAR/FOLD's session and configuration do
not touch the user's ordinary ones. This does **not** redirect the operating
system's Trash. Trash tests must use disposable files and a separate,
deliberate platform check. Do not use a person's real files to prove recovery.
On macOS, run platform CI and a hands-on Trash check before claiming parity.

Example starting point for ordinary manual tests:

```sh
lab=$(mktemp -d)
mkdir -p "$lab/files/project-a/nested" "$lab/files/project-b"
printf 'find this text\n' > "$lab/files/project-a/nested/note.txt"
STARFOLD_DIR="$lab/state" starfold "$lab/files"
```

After each milestone, give the user the built version and its specific manual
checklist. Incorporate findings before starting the next milestone.

## Milestones

### 0. Establish the baseline

Record the behavior and test results for navigation, marks, queued operations,
Commander, Places, and session restore. Bring [status.md](status.md) up to date;
it currently describes some implemented work as future work. Prepare a small
disposable fixture covering nested directories, hidden files, links, name
collisions, and filenames with spaces. This gives later milestone reports a
known starting point.

**Pass:** Existing tests and release build pass; the fixture and baseline
manual checklist are documented. No feature behavior changes.

### 1. Create files and directories

Add empty-file and directory creation in the active directory. Dispatch
filesystem writes through the existing worker and fold their result through
`state::apply`; the UI must not perform filesystem IO. Create immediately,
because creation has no destructive destination and a new directory should be
enterable at once. Refuse an existing name, never overwrite, and surface
permission and invalid-name errors. Refresh the listing and place the cursor
on the new item. Add discoverable bindings and context actions without
colliding with existing keys.

**Pass:** Create and enter a directory; create a file; try the same name twice;
try an unwritable directory. Automated tests verify no overwrite and correct
refresh in Fold and Commander.

### 2a. Recursive filename search

Keep `/` as the current-directory filter. Add a separate recursive search
rooted at the active directory. Traverse on a worker, never follow directory
symlinks, bound memory/results, show progress and errors, and support cancel.
Results carry full paths and permit opening, previewing, marking, and queuing
the same file actions as ordinary listings. Leaving results returns to the
previous directory and cursor. A stale search result must be revalidated
before an operation runs.

**Pass:** Find and act on a nested file; hide or include hidden files
predictably; cancel a large search; handle a symlink loop and an unreadable
subtree without freezing the UI.

### 2b. Content search

Add a text-content mode to the same results view. Stream files with explicit
per-file and total work limits; skip or clearly label binary and oversized
files. Return a useful matching excerpt or line, and keep actions bound to the
file's real path. Do not silently claim a partial scan is complete.

**Pass:** Find text in a nested file; verify binary/oversized cases, an
unreadable file, cancellation, and actions on a result.

### 3. Recovery after execution

Add a way to inspect and restore items STAR/FOLD put in Trash, including a
clear collision prompt when the original path exists. Investigate the Trash
metadata and restore APIs on Linux and macOS before committing to the backend;
the existing `trash` adapter only deletes. Add scoped undo for completed
renames and moves only when their destination is unchanged and the source is
available. Record enough facts to reject unsafe reversal. A permanent delete
cannot be presented as undoable. Trash restore should work after restarting
STAR/FOLD; session-only rename/move undo may be ephemeral.

**Pass:** Trash and restore a disposable file; restart before restore; provoke
a restore collision; rename or move and undo; modify the destination externally
and see undo safely refused. Automated tests use a fake Trash adapter; a real
platform Trash check is explicit and disposable.

### 4. Bulk selection and rename

Add select-by-pattern and invert-selection actions on top of marks. For batch
rename, show every old and proposed name before execution. Start with
numbering and find/replace, with an option to preserve extensions. Detect
duplicate targets, case-insensitive collisions, cycles, and targets outside
the source directory before touching files. Use a transaction plan with
temporary names and rollback or an explicit recovery record if an IO failure
interrupts the batch. Integrate successful results with scoped undo.

**Pass:** Preview and rename a mixed set; reject duplicate target names;
rename `a` ↔ `b`; inject a mid-batch failure and account for every source;
undo a completed batch while names are unchanged.

### 5. Shell and tool handoff

Add a last-directory output mode for shell wrappers and a machine-readable,
NUL-delimited selected-path output for picker callers. Write output after the
terminal is restored. Provide shell wrapper examples and tests for spaces,
newlines, and non-UTF-8 paths where the platform permits them. Extend the
single configured external opener with an explicit “open with” choice and
configured custom actions. Pass paths as separate arguments: never build a
shell command by interpolating filenames. Show missing-command and nonzero
exit errors.

**Pass:** Exit STAR/FOLD and change a shell to its final directory; return
multiple selected paths; run a harmless test action on files with spaces and
quotes; verify a failing command reports its status.

### 6. Durable project tabs

After the daily-work gaps above, turn the reserved `Tabs` model into several
independent Fold/Commander navigation contexts. Each tab has its own
locations, active pane, cursor/filter/scroll context, and preview context.
Keep the operation queue visible globally so queued work cannot disappear
behind a tab. Save tab locations and the active tab across restart; migrate
old single-tab sessions. Do not persist pending operations merely to restore
tabs: paths may be stale or unsafe after restart. Give async work stable tab
identities and include `TabId` in UI scroll/cache keys. Define a safe policy
for two STAR/FOLD processes sharing one session file so the second process
cannot silently overwrite the first one's saved tabs. Fit the tab indicator
within the existing minimum terminal height.

**Pass:** Open two projects, switch tabs and views, close/reopen STAR/FOLD, and
recover both locations and the active tab. Verify old session migration,
pending-queue visibility, long labels at 60×21, and concurrent-instance
session behavior.

**Why tabs:** Terminal tabs run independent STAR/FOLD instances. In-app tabs
can preserve several project contexts inside one restorable session. This is
worth implementing after the core gaps, not before them.

### 7. Desktop file conveniences

Add duplicate, create symbolic link, and create from a template as separate
small changes. Then add an inspectable properties view and permission editing.
Use the existing queue for actions that can affect existing files, apply
collision checks, and keep metadata reads off the UI thread. Template files
must be copied without running their contents. Permission changes need a
before/after preview and a clear failure report.

**Pass:** Try each action on a file and directory, including an existing
target, a broken link, a read-only source, and a denied permission change.

### 8. Progress and cancellation within a file

Prototype chunked copy against today's `std::fs::copy` path. Measure common
and large files on Linux and macOS and verify xattrs, permissions, timestamps,
and APFS cloning behavior. Ship chunk progress only if the implementation
preserves required metadata and acceptable copy performance; otherwise keep
the fast copy path and accurately show per-file, indeterminate progress.
Define what happens to an incomplete destination after cancellation.

**Pass:** Cancel a large copy and inspect the destination; compare metadata
and elapsed time to the current path. No incomplete file may be presented as
a successful copy.

## Later evaluation

- **Directory comparison:** useful for Commander synchronization, but first
  define comparison criteria and conflict behavior against real workflows.
- **Direct remote browsing:** GNOME Files and Thunar support remote locations;
  STAR/FOLD currently operates on mounted paths. Native SSH/SFTP or other
  virtual filesystems need a separate cross-platform architecture and safety
  design. Mounted network locations remain usable in the meantime.
- **Recent files, icon views, thumbnail controls, and desktop sharing:** assess
  only after the terminal-first workflows above; they do not close the current
  daily-work gaps.

## Comparison references

- [ranger manual](https://github.com/ranger/ranger/blob/master/doc/ranger.pod)
- [lf documentation](https://github.com/gokcehan/lf/blob/master/doc.md)
- [vifm manual](https://vifm.info/manual.shtml)
- [Midnight Commander manual](https://source.midnight-commander.org/man/mc.html)
- [GNOME Files search](https://help.gnome.org/gnome-help/files-search.html),
  [batch rename](https://help.gnome.org/gnome-help/files-rename-multiple.html),
  and [Trash restore](https://help.gnome.org/gnome-help/files-recover.html)
- [Thunar file actions](https://docs.xfce.org/xfce/thunar/working-with-files-and-folders),
  [recursive search](https://docs.xfce.org/xfce/thunar/the-file-manager-window),
  [bulk rename](https://docs.xfce.org/xfce/thunar/bulk-renamer/start), and
  [custom actions](https://docs.xfce.org/xfce/thunar/custom-actions)
