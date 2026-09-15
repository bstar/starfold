# Working notes

Context that is not derivable from the code or the history, kept here rather
than in any one machine's notes because this is developed on both Linux and
macOS, with more than one assistant, and the repository is the only thing all
of them see.

This file is the only assistant-facing notes file in the repository. Do not add
a tool-specific notes file or directory beside it; every assistant reads
`AGENTS.md`.

## Building

Use the Nix flake for all builds and checks. Do not assume `cargo` is on the
ambient `PATH`.

```sh
CARGO_NET_GIT_FETCH_WITH_CLI=true nix develop -c cargo build --release
CARGO_NET_GIT_FETCH_WITH_CLI=true nix develop -c cargo test --all
```

There are no system libraries. That is worth stating for a program that
deletes to the trash and draws pictures: trash is the freedesktop
specification, in pure Rust, on Linux, and `NSFileManager` through the `objc2`
bindings on macOS — Rust bindings to a system framework, with no `-sys` crate
and no bindgen. A change that adds a `-sys` crate has to add the library to
`flake.nix` and to CI in the same commit.

On a Mac with Xcode selected, `/usr/bin/git` is a shim that asks xcrun where
git is, and inside `nix develop` xcrun is nix's, which answers `tool 'git' not
found`. The devshell therefore carries nix's own git first on `PATH`, which is
what cargo's CLI fetch of STAR/KIT uses.

## STAR/KIT

The shared foundation — paths, logging, private file writes, themes, the dock
layout engine, text entry, wrapping, terminal images — lives in `starkit`, the
crate STAR/AMP and STAR/CORD use too. It is a git dependency pinned to a tag,
and the flake takes it from the revision `Cargo.lock` names through
`cargoLock.allowBuiltinFetchGit`.

It is also the one copy of `ratatui`, `crossterm`, `ratatui-image` and `image`
in the tree. Everything under `src/ui/` reaches them through `starkit::`, and
none of the four is a direct dependency: a widget built against a second copy
of ratatui does not satisfy a signature expecting the first, and the compiler
reports that as two versions carrying the same number.

`src/paths.rs` is a name for `starkit::paths::Paths` and nothing else. Keep the
name: the core takes `crate::paths::Paths` by value throughout, and `PATHS` is
the one place this application's three identifying strings are written down.

A local checkout is used through an uncommitted `.cargo/config.toml`:

```toml
[patch."https://github.com/bstar/starkit"]
starkit = { path = "../starkit" }
```

`.gitignore` already covers it, and it has to be taken away again before
anything is committed: with the patch in place `cargo` rewrites the `starkit`
entry in `Cargo.lock` to the path, and a lock file with no git source in it is
one the flake cannot build. Every public `starkit` item now has three
consumers — check STAR/AMP and STAR/CORD before changing a signature.

`Graphics::probe` has to run before `term::init`: it needs the terminal in its
ordinary mode to ask the question, and `term::init` is what takes the
alternate screen.

## Nothing under src/fold/ draws

No `ratatui`, no `crossterm`, no `crate::ui` anywhere under `src/fold/`. A test
in `fold/mod.rs` greps the module's own sources and fails if any of them
appear.

This is not tidiness. It is why `starfold list` can print a directory with no
terminal attached, why the core is testable from a temporary tree with no
window involved, and why a rendering change cannot break a copy. The core owns
the truth behind an `RwLock` and emits `Event`s that are only *notifications*:
the UI may coalesce or drop them and still render the truth on the next frame.

The dependency runs the other way as well. `src/ui/` never reaches into
`fold::state` internals; it talks to `Handle` and to the read-side query API on
`State`, and `Handle::from_parts` exists so the UI can be driven by a fake core
in tests.

## One writer

`fold::state::apply` is the only thing that mutates `State`. It does no IO —
every job it hands back is run by a worker thread, and a worker takes the
write lock only inside `apply` itself, to fold its own result back in. The UI
never holds a read guard across a draw: it copies its view out when
`State::version` moves, and draws from the copy.

## Operations are transactional

Nothing is copied, moved or deleted until the operations queue is run. `y`,
`m` and `d` add to the queue; `enter` or `X` runs it; `esc` clears it. A
delete goes to the trash where the platform has one, and a permanent delete
asks first. `plan` never follows a symlink, and a conflict is decided by
comparing `dev` and `ino`, not by whether a path merely exists — a
case-insensitive filesystem can otherwise see a file collide with itself.

## Tests

In-module `#[cfg(test)]`, as in STAR/AMP and STAR/CORD. Anything that reads
foreign input — a directory somebody else filled, a symlink, a file name —
gets a proptest as well as table tests. `cargo test` must pass on a machine
with no trash daemon running; anything that needs one is gated behind
`STARFOLD_TEST_TRASH=1`. The UI's frames are `insta` snapshots driven by a
fixture tree built in a tempdir, not by anything checked in.

## Not yet verified

Nothing has shipped yet, so there is nothing here to distrust. Once a
milestone has been checked against a real terminal and a real filesystem,
record here what turned out to be a guess rather than an observation — the
same way STAR/CORD keeps its list of protocol details still waiting on a live
session.
