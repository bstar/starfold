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

The shared foundation — paths, logging, private file writes, themes,
keymap/help, chrome, text entry, wrapping, terminal graphics — lives in
`starkit`, the crate STAR/AMP and STAR/CORD use too. It is a git dependency
pinned to a tag, and the flake takes it from the revision `Cargo.lock` names
through `cargoLock.allowBuiltinFetchGit`.

STAR/KIT's dock layout engine is not used here: the one-column layout
(`src/ui/layout.rs`) is hand-rolled the same way STAR/CORD's is, because
STAR/FOLD only ever has one column of modules to place, never more than one
arrangement to choose between.

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
one the flake cannot build. **Never commit `.cargo/config.toml`, and never
commit a `Cargo.lock` written while it was in place** — one agent did exactly
this during this milestone, and the fix was reverting `Cargo.lock` back to
the git-sourced entry and deleting the file. If `git status` ever shows
`Cargo.lock` changed with no dependency version bump to explain it, check for
this before anything else. Every public `starkit` item now has three
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

## Two threads, and the one function both fold through

`starfold-io` (listings, directory summaries, previews, external opens, the
mtime poll) and `starfold-ops` (the operations queue, one entry at a time)
are the only two OS threads. Both are plain `std::thread` plus
`crossbeam-channel`; there is no async runtime, because nothing here waits on
a socket. Neither thread ever writes to `State` directly: `worker::finish`
(`src/fold/worker.rs`) is the one function that takes the write lock, folds a
`Done` into `State` through `state::apply`, dispatches whatever `Job`s that
implied, and sends whatever `Event`s it implied — and it is the *same*
function both real worker loops call and `ui/fake.rs`'s threadless fixture
calls to fold a result in on the test thread. If you add a third place that
calls `state::apply` directly instead of going through `finish` (or through
`Handle::send` for a `Command`), you have created a second writer.

## One writer

`fold::state::apply` is the only thing that mutates `State`. It does no IO —
every job it hands back is run by a worker thread, and a worker takes the
write lock only inside `apply` itself (via `finish`, above), to fold its own
result back in. The UI never holds a read guard across a draw: it copies its
view out when `State::version` moves, and draws from the copy. `state.rs` has
its own grep test, separate from the one described above, for `std::fs::`
inside `apply`'s own functions (each exempted line marked `NO-IO-HERE`) —
`apply` must never touch a filesystem directly, only ever hand back a `Job`
for a worker to do it.

## Operations are transactional

Nothing is copied, moved or deleted until the operations queue is run. `y`,
`m` and `d` add to the queue; `enter` or `X` runs it; `esc` clears whatever
has not started (a running op is left to finish, or is stopped with
`ctrl+x`). A delete goes to the trash where the platform has one, and a
permanent delete asks first. `plan` never follows a symlink, and a conflict
is decided by comparing `dev` and `ino`, not by whether a path merely exists
— a case-insensitive filesystem can otherwise see a file collide with itself.

An `Op`'s `progress: Arc<ops::progress::Progress>` (three atomics: done,
total, cancelled) is the one piece of an `Op` shared with the thread actually
running it — the same `Arc` goes out in `Job::Run` and back through
`state.queue`, so the status row reads live numbers without taking the write
lock, and `Command::Cancel` sets the flag without waiting for the ops thread
to notice. The ops thread checks it between items, not mid-file: a large file
already being copied by `std::fs::copy` finishes before a cancel takes
effect.

`std::fs::copy` is what actually moves bytes, once per file — it keeps
xattrs, and on APFS it clones rather than duplicates the data. There is no
intra-file progress: the status bar's number moves file by file, not byte by
byte within one very large file, because `std::fs::copy` gives nothing to
poll partway through. Chunked copying for progress inside a single huge file
is a later milestone's question, not this one's.

A rename to a name that differs only in case, on a filesystem where that is
not a different name at all, cannot go through a direct `rename(2)` — the
kernel either treats it as a no-op or, in the worst case, deletes what it was
asked to create, because it sees no work to do. `exec::rename_case_only`
works around this by renaming through a temporary sibling name first
(`.starfold-rename-<pid>`) and from there to the real target, restoring the
original name if the second hop fails rather than leaving the file stranded
under the temporary one.

`ops::trash::available()` always returns `true`. This is deliberate, not a
stub: a correct probe would have to know, at startup, which filesystem every
*future* delete will target — the home trash
(`$XDG_DATA_HOME/Trash`/`~/.local/share/Trash`) is only the common case, and a
path on a different mount uses that mount's own `$topdir/.Trash-$uid`
instead, which nothing available at startup can name. Reporting `true`
unconditionally and letting a real failure surface from `trash::delete` (which
`state::apply` turns into a note plus the permanent-delete confirmation) is
simpler and cannot be wrong the way a home-only probe would be.

The preview panel is intended to follow the cursor: every `Command::Preview`
bumps `State::preview_generation`, and a `Done::Previewed` carrying an older
generation than the state currently holds is dropped rather than shown — the
cursor moved again before the earlier build finished. `worker::perform_io`
checks the generation again before it even starts building, so a stale
request does not cost a decode it will only throw away.

## Testing: `Fixture::tree()` and `fake::handle()`

`src/fold/testing.rs`'s `Fixture::tree()` is the one fixture tree every core
test and every drawn frame is built on — a tempdir, not anything checked in
under `testdata/` (see `testdata/README.md` for exactly what it contains).
Build one, read `f.home()` or `f.path("relative/thing")`, and let the
`TempDir` drop clean it up.

`src/ui/fake.rs`'s `fake::handle(cfg) -> (Handle, Fake)` is how a UI test gets
a `Handle` with no worker threads behind it: same `State::new`, same
synchronous first listing as `Handle::spawn`, but the two job queues are
handed to a `Fake` instead of two spawned threads. Send a `Command` through
the `Handle` exactly as the real UI would, then call `Fake::pump()` to drain
both queues through `worker::perform_io`, `worker::perform_ops` and
`worker::finish` — the very functions the real threads call — synchronously,
on the test thread, looping until nothing is left to run (a `Done::Planned`
can itself produce the `Job::Run` that actually moves bytes, so one drain is
not always enough). This is not a second implementation of the core the way
some fakes are: `state::apply`, `listing::read`, `preview::build`,
`ops::plan::plan` and `ops::exec::run` all run for real, against the real
fixture tree. Only the threads are missing.

## Tests

In-module `#[cfg(test)]`, as in STAR/AMP and STAR/CORD. Anything that reads
foreign input — a directory somebody else filled, a symlink, a file name —
gets a proptest as well as table tests. `cargo test` must pass on a machine
with no trash daemon running; anything that needs one is gated behind
`STARFOLD_TEST_TRASH=1`. The UI's frames are `insta` snapshots driven by
`fake::handle()` over `Fixture::tree()`, not by anything checked in.

## Not yet verified

Nothing in this milestone has been run in a real terminal against a real
filesystem yet — everything above is built and passes its own headless
tests, which is a different claim. Once it has been checked against a live
session, this is where a claim that turned out to be a guess rather than an
observation gets recorded, the same way STAR/CORD keeps its own list of
protocol details still waiting on one. Specifically, still unverified as of
this milestone:

- The window itself: `ui/app.rs`'s event loop, drawing, resizing, focus.
- A picture actually appearing in kitty, iTerm2, WezTerm or Ghostty, and as
  half-blocks where none of those apply.
- The mtime poll (`watch.rs`) noticing a real external change while the
  window is open.
- Deleting to the trash on a real macOS machine and a real Linux one.
- A move across a real device boundary (EXDEV, the copy-then-delete
  fallback).
- The 60×21 floor, and the row just below it, on a real terminal rather than
  a fixture-sized buffer.
- The picture scales (`z`) under sixel and iTerm2. `1x` and `pixels` place a
  pre-scaled picture over a rectangle that is a whole number of cells and so
  is up to one cell wider and taller than the pixels themselves -- kitty
  tolerates that slack, and whether sixel and iTerm2 do has only been
  reasoned about. Checked by eye in kitty; the other two are guesses. What
  `smooth` and `pixels` look like where the terminal reports no cell size is
  reasoned about too -- all three fall back to the old fit, which cannot be
  seen without a terminal that does it.
