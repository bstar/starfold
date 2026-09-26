# Milestone 1: create files and directories

Use a disposable tree and isolated application state. This checklist follows
[milestone 1 of the roadmap](file-manager-roadmap.md).

## Prepare

From the STAR/FOLD repository:

```sh
lab=$(mktemp -d)
mkdir -p "$lab/files/empty" "$lab/files/locked"
printf 'keep this content\n' > "$lab/files/keep.txt"
chmod u-w "$lab/files/locked"
STARFOLD_DIR="$lab/state" starfold "$lab/files"
```

## Check in STAR/FOLD

1. Click `actions` in the stack heading, choose New file, type `new note.txt`,
   and press Enter. The file should appear and
   be selected. In another shell, check that it is empty:
   `test ! -s "$lab/files/new note.txt"`.
2. Click `actions`, choose New directory, type `new folder`, and press Enter. The directory should appear
   selected. Press Enter again to enter it; `h` returns to its parent.
3. Choose New file from `actions` and enter `keep.txt`. STAR/FOLD should show a creation error.
   Check `cat "$lab/files/keep.txt"` still says `keep this content`.
4. Enter `empty`. Click `actions` or open the menu with Menu or Shift+F10. It should offer
   New file and New directory. Create `inside.txt` from the menu. Ctrl+click
   blank listing space and check the same two actions are available.
5. Go back and enter `locked`. Choose New file with `denied.txt`. A permission error
   should appear and no file should be created. If your account can still
   write here because of an elevated shell or ACL, use a location that really
   rejects creation for this check.
6. Return to the top, press `v` for Commander, and click `actions` in one pane
   to create a directory. It should be selected there and enterable. Switch
   panes with Tab, create a file from that pane's menu, and verify the other pane does not
   jump or change directories.
7. Repeat the dialogs at 100×30 and 60×21. Check that the name, inline name
   error, and Enter/Escape instructions are readable. Escape should dismiss
   either dialog without creating anything.
8. By default a physical right-click should leave the file menu closed. In
   `"$lab/state/config.toml"`, set `right_click = true` under `[ui]`, restart
   STAR/FOLD with the same launch command, and confirm right-click opens it.
9. Select `keep.txt`, open its file menu, and choose Edit. The editor should
   fill the Preview panel while the listing stays visible. Change the text,
   save and quit using the editor's own keys, then check the updated text in
   Preview and with `cat "$lab/files/keep.txt"`. Repeat at 60×21. A binary
   file such as `blob.bin` should have no Edit action.

After review, restore write permission and remove the disposable tree:

```sh
chmod u+w "$lab/files/locked"
rm -r -- "$lab"
```

## Automated checks

The release gate runs `cargo test --all`, `cargo clippy --all-targets -- -D
warnings -A dead_code`, and `cargo build --release` in `nix develop`. Creation
tests run the real IO worker functions synchronously against a temporary tree,
including existing files, directories, and broken symlinks.
