# Milestone 0: baseline validation

This is the manual check for the first milestone of the
[file manager roadmap](file-manager-roadmap.md). It uses disposable files and
an isolated STAR/FOLD state directory. It does not test the operating system's
Trash, removable drives, or network mounts.

## Prepare

From the STAR/FOLD repository:

```sh
lab=$(./scripts/prepare-baseline.sh)
printf 'Fixture: %s\n' "$lab"
STARFOLD_DIR="$lab/state" starfold "$lab/files"
```

The fixture contains two project directories; a nested text file; a name with
spaces; a hidden file; a working and a broken symbolic link; and two
`shared.txt` files with different contents for a later collision check. The
script creates a fresh tree each time and prints its path. It does not remove
the tree automatically, so it stays available while checking results.

## Check in the application

1. **Navigation:** Enter `project-a`, then `nested`. `h` goes to the parent.
   `Alt+Up` and `Alt+Down` jump between Fold levels without losing the cursor.
   `.` reveals `.hidden`; `/` filters only the current directory. Clear the
   filter with Escape.
2. **Marks and queue:** In `project-a/nested`, mark `note.txt` with Space.
   Navigate to `project-b` through the parent directories. The mark count
   remains visible. Press `y`: OPERATIONS gains a queued copy to `project-b`.
   Before running it, verify in another shell that
   `"$lab/files/project-b/note.txt"` does not exist. Press `X` to run the
   queue; the copied file should then exist with the same contents.
3. **Collision:** Mark `project-a/shared.txt` and queue a copy into
   `project-b`, where `shared.txt` already exists. Run the queue and check
   that STAR/FOLD asks how to resolve the conflict rather than silently
   overwriting the destination. Choose Skip for this baseline; the
   destination must still contain `destination version`.
4. **Commander:** Press `v`, switch panes with Tab and Shift+Tab, and browse
   different directories in each pane. Returning to Fold with `v` should
   retain its location. The OPERATIONS queue is shared across views.
5. **Places:** Press `B` to bookmark the current directory, then `b` to open
   Places. Find the bookmark by typing part of its name and open it. This
   bookmark lives under `"$lab/state"`, not your ordinary STAR/FOLD state.
6. **Session restore:** Navigate to `project-b`, quit with `q`, and relaunch
   without a directory argument:

   ```sh
   STARFOLD_DIR="$lab/state" starfold
   ```

   The last view and its saved directories should return. An explicit
   directory argument overrides the saved location, so omit it here.
7. **Layout:** Repeat the navigation and Places checks with terminal windows
   at 100×30 and 60×21. Check that the title, rows, and status remain usable.

If a step fails, record the step, terminal size, expected result, and what
appeared instead. The fixture can be removed after review with
`rm -r -- "$lab"` once its path has been checked.

## Automated baseline

Run these from the repository through the Nix development shell:

```sh
CARGO_NET_GIT_FETCH_WITH_CLI=true nix develop -c ./scripts/check-version.sh
CARGO_NET_GIT_FETCH_WITH_CLI=true nix develop -c cargo fmt --check
CARGO_NET_GIT_FETCH_WITH_CLI=true nix develop -c cargo clippy --all-targets -- -D warnings -A dead_code
CARGO_NET_GIT_FETCH_WITH_CLI=true nix develop -c cargo test --all
CARGO_NET_GIT_FETCH_WITH_CLI=true nix develop -c cargo build --release
```

After the release build, verify that `readlink -f "$(command -v starfold)"`
points to this checkout's `target/release/starfold` on the Linux development
workstation. Restart any running STAR/FOLD process before testing the new
binary.

### Recorded Linux result, 2026-09-25

- Version check, `cargo fmt --check`, and strict Clippy passed. Clippy initially
  found redundant `statvfs` casts; the capacity calculation now accepts both
  Linux's 64-bit and macOS's 32-bit block-count types.
- `cargo test --all` passed: 507 unit tests, 2 preview integration tests,
  3 CLI smoke tests, and 4 terminal tests (516 total).
- `cargo build --release` passed. The installed `starfold` command resolves to
  this checkout's `target/release/starfold` and reports version `0.0.1`.
- `starfold list` read the disposable `project-a` fixture. It showed the
  directory, both links, and the regular files, while hiding `.hidden` by
  default.
- The interactive checklist above remains for desktop validation. This Linux
  run does not establish macOS behavior or real Trash/device behavior.
