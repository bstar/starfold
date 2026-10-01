# Recovery validation

Open file actions with `c`, Menu / Shift+F10, or right-click. Choose **Recovery →
Trash…** to restore a trashed item, or **Recovery → Undo…** to inspect reversible
moves and renames. `Ctrl+Z` opens Undo directly. Use arrows or j/k to select,
Enter to recover, r to refresh, and Escape to close. Recoveries appear immediately
in OPERATIONS; failures remain available there after notifications expire.

## Behavior

- Linux lists compatible freedesktop Trash entries, including across restarts.
- macOS records STAR/FOLD's Trash operations in a private journal under the app's
  base directory. Recovery works after restart for these recorded items. Use
  Finder for items trashed by other applications or before this feature existed.
  A missing or corrupt journal reports an error; it is not silently replaced.
- A Trash restore collision offers an editable `name (1).ext` suggestion.
  Existing files, directories and dangling symlinks are never overwritten.
- Undo keeps up to 100 eligible items globally across tabs for this session.
  Completed moves/renames without overwrite or directory merge are eligible.
  Copies, permanent deletes, incomplete jobs and merged/overwritten destinations
  are excluded. Undo refuses a changed item, descendant, replaced parent
  directory, or occupied original name.
- Verification is bounded to 50,000 entries and 64 directory levels per item.
  Directory symlinks are not followed. Missing original directories must be
  recreated before recovery. Metadata is checked again when queued work runs.
- Across devices, recovery stages a complete copy with metadata, checks the
  source again, and publishes without overwrite before removing the old copy.
  Cancellation removes staging; a cleanup failure reports the recovered location
  and retains its error in OPERATIONS.

## Disposable manual checklist

Use a new temporary tree and `STARFOLD_DIR` for an isolated session. This does
not redirect system Trash: use only files created for this check. Real macOS
Trash and physical cross-device checks require separate hands-on validation.

1. Create `original.txt`, rename it, press Ctrl+Z, select it and Enter.
   Confirm the original name and content return and the undo entry disappears.
2. Move a directory containing a file between Commander panes. Undo and check
   that both panes refresh and the directory and content return.
3. Rename another file, modify it outside STAR/FOLD, then try Undo. Confirm the
   operation fails persistently and leaves the modified file untouched.
4. Trash a disposable file. Restart STAR/FOLD, open Recovery → Trash, and restore
   it. Confirm content and metadata return to the original directory.
5. Trash another file, create a new file at its original name, and restore.
   Confirm the numbered suggestion is editable and both files survive.
6. Try a restore whose original directory is missing, then recreate the directory
   and retry from a refreshed Trash picker. Confirm the first attempt loses nothing.
7. For a physical cross-device move, undo a large file and cancel during copying.
   Confirm the moved file remains intact and no partial original is published.
8. Repeat the real Trash checks on macOS. Verify a journaled item returns after
   restart, and that errors stay visible without overwriting either file.

Automated coverage includes temporary-tree safety/property tests, private Linux
Trash recovery in separate processes, a macOS fake Trash adapter for persisted
intent, queue integration, and light/dark snapshots at 100×30 and 60×21.
