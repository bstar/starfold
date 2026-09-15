# Changelog

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [semantic versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **The fold stack.** Entering a directory pushes a level onto the column;
  every level already drilled through stays there, folded to one line.
  `alt+up` jumps to the parent without losing anything, `alt+down` steps back
  into a child a jump left behind, and backing out with `h` discards the
  level for good, the way a fresh navigation discards a browser's forward
  history.
- **Persistent marks.** `space` marks the entry under the cursor from
  anywhere in the stack; the mark follows the fold to a different directory,
  with a running byte total in the status line.
- **The operations queue.** Copies, moves, deletes and renames are queued
  rather than run on the spot, planned before anything is touched, and asked
  about when a name at the destination would collide. A delete goes to the
  trash where the platform has one; a running operation can be cancelled
  without leaving anything half-done.
- **The preview panel.** Text, a decoded image, a budgeted directory summary,
  a hexdump for anything else, and what a symlink points at — built from the
  same limits `config.toml`'s `[preview]` table states.
- **The column and its themes.** One-column chrome shared with the rest of
  the STAR family: the stack, preview and operations modules, a status row,
  every key and mouse gesture, and the `[fold]` colour roles derived from
  STAR/KIT's sixteen built-in themes.
- **`starfold list`.** A headless listing for a script or a bug report, with
  the same reader the window itself uses underneath.
- **Fixture-driven tests.** A tempdir fixture tree (`src/fold/testing.rs`)
  and a threadless fake core (`src/ui/fake.rs`) that runs the real listing,
  planning and execution code inline, so the core and the drawn frames are
  both tested against a real filesystem with no terminal and no worker
  thread involved.
