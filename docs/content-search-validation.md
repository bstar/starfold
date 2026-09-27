# Milestone 2b: text-content search

Use an isolated tree and session. Content search reads regular UTF-8 files up
to 1 MiB each and at most 64 MiB across one search. It does not follow file
or directory symlinks. Binary and oversized files are counted as skipped;
an unreadable file or a work limit labels the scan partial or limited.

## Prepare

```sh
lab=$(mktemp -d)
mkdir -p "$lab/files/one/nested" "$lab/files/two" "$lab/files/.private"
printf 'first line\nfind this phrase\n' > "$lab/files/one/nested/note.txt"
printf 'different text\n' > "$lab/files/two/other.txt"
printf 'find this\0binary\n' > "$lab/files/two/blob.bin"
printf 'find this\n' > "$lab/files/.private/secret.txt"
truncate -s 1048577 "$lab/files/two/large.txt"
ln -s "$lab/files" "$lab/files/one/loop"
STARFOLD_DIR="$lab/state" starfold "$lab/files"
```

## Check in STAR/FOLD

1. Press F3 or Ctrl+F, then Tab. The prompt should say it searches file
   contents. Enter `FIND THIS`. The nested note should show its matching line.
   The rule should say `partial` and count one binary and one large file.
   At 60×21, the filename and matching line should remain visible.
2. Press Space to mark the note, inspect Preview, and queue a copy into a
   disposable destination. Run OPERATIONS and verify its contents. Search
   results should use the note's real path, even if the row text is shortened.
3. Close results with `h` and check the previous directory and cursor return.
   Repeat in Commander. Open search again; Tab should switch back to filename
   mode, and the existing filename search should still work.
4. Turn on hidden files with `n` and repeat content search. The hidden text
   file should appear. Turn hidden files off and confirm it disappears from
   the next scan.
5. Search a large disposable tree, press Escape during the scan, and confirm
   it says `cancelled`. Another Escape should close results. A search over
   more than 64 MiB of eligible text should say `limit reached`.
6. Use a separate account that cannot read one file in the disposable tree.
   Confirm an unreadable note and a `partial` status while readable matches
   remain. Replace a found file after queuing a copy; running the queue must
   reject the changed file without copying the replacement.

After checking, quit and remove the disposable tree:

```sh
rm -r -- "$lab"
```
