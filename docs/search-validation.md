# Milestone 2a: recursive filename search

Use an isolated tree. The scan never follows directory symlinks, and its
results carry real paths for Preview, marks and file actions.

## Prepare

```sh
lab=$(mktemp -d)
mkdir -p "$lab/files/one/nested" "$lab/files/two" "$lab/files/.private"
printf 'first\n' > "$lab/files/one/nested/needle.txt"
printf 'second\n' > "$lab/files/two/needle.txt"
printf 'hidden\n' > "$lab/files/.private/needle.txt"
ln -s "$lab/files" "$lab/files/one/loop"
STARFOLD_DIR="$lab/state" starfold "$lab/files"
```

## Check in STAR/FOLD

1. Press F3 or Ctrl+F, type `needle`, and press Enter. The result list should
   show both visible relative paths, finish without freezing, and show a
   scanned count. `/` should still filter only the current directory after
   you leave search.
2. Select a result and inspect Preview. Press Space to mark it, then `c` to
   open its file menu. Confirm the clicked path names the nested result, not
   the directory you started in. Queue a copy to the disposable search root
   and run it from OPERATIONS.
3. Press `h` from results. The original directory and cursor should return.
   Repeat in Commander; its pane locations should return when search closes.
4. Search again, click `hidden` or press `n`, and check the hidden result
   appears. Turning hidden off should remove it from a fresh result set.
5. Start a scan over a large disposable tree and press Escape. The scan should
   stop and say `cancelled`; another Escape should restore the original view.
   Pressing `h` during a scan should leave immediately.
6. Rename or replace a found file in another shell after queuing a file action
   but before running it. The operation should fail with a changed-result
   message and leave the replacement untouched. A missing result should not
   be acted on silently.
7. Repeat at 100×30 and 60×21. Long relative paths may be shortened on
   screen; actions and Preview must still use the full path. Search a tree
   with an unreadable directory using an account that cannot read it and
   confirm an error is shown while other matches remain available.

The scanner stops after 100,000 visited entries, 5,000 matches, or 64 levels
and labels the result `limit reached`. A symlink loop must not add levels.

After checking, quit and remove the disposable tree:

```sh
rm -r -- "$lab"
```
