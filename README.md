# STAR/FOLD

[![ci](https://github.com/bstar/starfold/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/bstar/starfold/actions/workflows/ci.yml)
[![nix](https://github.com/bstar/starfold/actions/workflows/nix.yml/badge.svg?branch=main)](https://github.com/bstar/starfold/actions/workflows/nix.yml)
[![debian](https://github.com/bstar/starfold/actions/workflows/debian.yml/badge.svg?branch=main)](https://github.com/bstar/starfold/actions/workflows/debian.yml)
[![arch](https://github.com/bstar/starfold/actions/workflows/arch.yml/badge.svg?branch=main)](https://github.com/bstar/starfold/actions/workflows/arch.yml)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A terminal file manager in the STAR family. The directories you drill through
stack up behind you, the files you mark stay marked wherever you go, and
nothing is copied, moved or deleted until you run the queue.

<!-- The screenshot goes here, as docs/screenshot.png, the way STAR/AMP's
     README carries one. There is no picture yet: the window is still being
     built, and one of a half-drawn window would be retaken every week. -->

[Status](docs/status.md) says how much of that is built today, area by area.

Unfold your filesystem.

## Get it

Packages for Linux are on the
[releases page](https://github.com/bstar/starfold/releases/latest): an AppImage
that needs nothing installed, a `.deb` for Debian and Ubuntu, a portable
tarball, and the source the Arch `PKGBUILD` builds.

```sh
nix run github:bstar/starfold        # Nix, on Linux or Apple Silicon macOS
```

[Installing](docs/installing.md) covers every route, including building from
source. There are no system libraries to install first.

## What it does

- **The fold stack.** Entering a directory pushes a level onto a column; every
  level you have drilled through is still there, folded to one line, and
  `alt+up` / `alt+down` jump between them without losing anything below.
- **Persistent marks.** `space` marks a file wherever you are; the marks
  follow you to a different directory, so `y` or `m` mean "copy or move what I
  marked, to here".
- **An operations queue.** Copies, moves, deletes and renames are queued
  rather than run on the spot; nothing touches the filesystem until you run
  the queue, and a delete goes to the trash where the platform has one.
- **A preview panel.** Text, an image drawn with real pixels where the
  terminal supports it, a directory summary, or a hexdump for anything else.
- **One column, sixteen themes.** The same chrome and the same theme engine
  as the rest of the STAR family, so a theme set in one looks the same in
  another. The mouse works everywhere the keyboard does.
- **Linux and macOS.** x86_64 and aarch64 on Linux, Apple Silicon on macOS.

## What it does not do yet

Tabs, forked stacks, archives and an action palette are on the plan but not in
this milestone. The stack's own data model already leaves room for tabs and
for more than one stack; they are just not wired up to anything yet.

## Read more

The [documentation](docs/README.md) has a page for each of those. The ones
most people want first:

- [The stack](docs/the-stack.md)
- [Keys and mouse](docs/keys-and-mouse.md)
- [Configuration](docs/configuration.md)
- [If something is wrong](docs/troubleshooting.md)

[CONTRIBUTING.md](CONTRIBUTING.md) is for building and changing it,
[CHANGELOG.md](CHANGELOG.md) for what each release holds, and
[SECURITY.md](SECURITY.md) for reporting a vulnerability.

## License

MIT. See [LICENSE](LICENSE).
