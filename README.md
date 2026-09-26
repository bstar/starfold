# STAR/FOLD

[![ci](https://github.com/bstar/starfold/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/bstar/starfold/actions/workflows/ci.yml)
[![nix](https://github.com/bstar/starfold/actions/workflows/nix.yml/badge.svg?branch=main)](https://github.com/bstar/starfold/actions/workflows/nix.yml)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A terminal file manager in the STAR family. The directories you drill through
stack up behind you, the files you mark stay marked wherever you go, and
keyboard-queued operations wait until you run the queue.

[Status](docs/status.md) says how much of that is built today, area by area.

Unfold your filesystem.

## Get it

Linux releases use Nix and AppImage. Apple Silicon macOS builds are available
on the [releases page](https://github.com/bstar/starfold/releases/latest).

```sh
nix run github:bstar/starfold        # Nix, on Linux or Apple Silicon macOS
```

[Installing](docs/installing.md) covers every route, including building from
source. There are no system libraries to install first.

## Try it

```sh
starfold ~/projects
```

Opens the window there: `l` or `enter` drills into a directory, `h` backs out
one level, `space` marks the entry under the cursor, and `y` queues a copy of
whatever is marked to wherever you land next — nothing touches disk until you
run the queue with `enter` or `X`.

```sh
starfold list .
```

Prints the current directory and exits, with no window involved — the same
reader the column itself uses. [On the command line](docs/cli.md) has the
rest of it.

## What it does

- **The fold stack.** Entering a directory pushes a level onto a column; every
  level you have drilled through is still there, folded to one line, and
  `alt+up` / `alt+down` jump between them without losing anything below.
- **Persistent marks.** `space` marks a file wherever you are; the marks
  follow you to a different directory, so `y` or `m` mean "copy or move what I
  marked, to here".
- **An operations queue.** Copies, moves, deletes, renames, compression and extraction are queued
  rather than run on the spot; nothing touches the filesystem until you run
  the queue, and a delete goes to the trash where the platform has one. A
  dropped file starts its own tracked transfer automatically.
- **Native drag and drop.** In terminals that support the [OSC 72 protocol](https://sw.kovidgoyal.net/kitty/dnd-protocol/),
  drag files into Fold or Commander, between panes, or out to the desktop.
  Remote sessions can transfer files over the terminal connection.
- **Intelligent previews.** Music/video tags, PDF text loaded three pages at
  a time as you scroll, archive contents, images, text and directory summaries.
  Other binary files show useful metadata. Unicode file-type icons appear in
  Fold, Commander and previews; no special font is required.
  Open an audio track to play it here using a separately installed,
  embedding-compatible STAR/AMP; without it, the existing opener still works.
- **File actions menu.** Press `c` with the Stack focused, click `actions` in its heading, Ctrl+click a
  file, or press Menu / Shift+F10 for creation, editing, compression, extraction
  and other file actions. Edit runs the terminal editor in Preview. Physical right-click is optional in configuration.
  Archive jobs appear in OPERATIONS.
  See [previews and archives](docs/previews-and-archives.md).
- **Recursive filename search.** Press F3 or Ctrl+F to search below the active
  directory without changing `/`'s current-directory filter. Results support
  preview, marks and file actions. Escape cancels a scan or closes results;
  closing restores the original directory and cursor.
- **Commander view and Places.** Press `v` for two independently browsable
  panes. `b` opens searchable bookmarks, mounted devices, network locations,
  Home and Root; selected drives show capacity and identifying details.
  `B` bookmarks the current directory.
- **Sixteen themes.** The same chrome and theme engine as the rest of the STAR
  family, so a theme set in one looks the same in another. The mouse works
  everywhere the keyboard does.
- **Linux and macOS.** x86_64 and aarch64 on Linux, Apple Silicon on macOS.

## What it does not do yet

Recursive search, recovery from Trash, bulk rename, shell picker output, and durable tabs are on the
[file manager roadmap](docs/file-manager-roadmap.md). Commander uses two
independent directory panes alongside the Fold stack; Places can unmount a
selected local drive, while mounting, physical ejection and network login
remain jobs for the operating system.

## Read more

The [documentation](docs/README.md) has a page for each of those. The ones
most people want first:

- [The stack](docs/the-stack.md)
- [Keys and mouse](docs/keys-and-mouse.md)
- [Drag and drop](docs/drag-and-drop.md)
- [Configuration](docs/configuration.md)
- [If something is wrong](docs/troubleshooting.md)

[CONTRIBUTING.md](CONTRIBUTING.md) is for building and changing it,
[CHANGELOG.md](CHANGELOG.md) for what each release holds, and
[SECURITY.md](SECURITY.md) for reporting a vulnerability.

## License

MIT. See [LICENSE](LICENSE).
