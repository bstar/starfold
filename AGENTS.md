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

On the Linux development workstation, the `starfold` command in the user's
shell is `~/.local/bin/starfold`, a symlink to this checkout's
`target/release/starfold`. After changing application code, always complete a
release build before saying the change is ready for desktop testing. Verify
`command -v starfold` and `readlink -f "$(command -v starfold)"` point to the
built executable; a debug/test build does not update the user's command.
Restart running STAR/FOLD instances when desktop testing requires the new
binary, since existing processes keep the old executable loaded.

There are no separately installed system libraries. `unrar_sys` compiles the bundled RARLAB C++ engine using the compiler in the flake; CI supplies the native C++ toolchain. Its notice is installed from `LICENSES/UnRAR.txt`. `libbz2-rs-sys` is pure Rust despite its name. That is worth stating for a program that
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

STAR/KIT's dock layout engine is not used here. `src/ui/layout.rs` lays out
the browser, preview, and operations vertically; Commander divides the
browser rectangle into two equal-width directory panes.

## Commander and Places

Stack 0 is Fold; stacks 1 and 2 are Commander's left and right panes, created
on first use or session restore. The active stack's marks live in
`State::selection`; inactive marks are parked per stack. Commander clears
marks when that pane changes directory. Fold keeps its persistent marks.
UI scroll keys are `(stack index, FrameId)` because frame IDs are local.
Commander moves capture the opposite pane's path when enqueued. Pastes copy
the yanked paths into the active pane's directory.

Places is a modal picker over worker-discovered mounted locations and
`bookmarks.toml` beside configuration. Initial discovery, manual refresh and
bookmark writes run on the IO worker; malformed bookmark files must never be
overwritten. Local-device unmounting and its post-unmount scan run on the
operations worker, which revalidates the device against the current mount table.
Places and OPERATIONS show the in-flight unmount as transient activity, not an
`Op` in the file queue; it must not be selectable or counted as a queued file op.
The mount scan also gathers local-volume capacity and available space, plus
Linux `lsblk` identification when available. Places draws cached facts only;
never query a mounted filesystem or run `lsblk` on the UI thread.
Mounting, physical ejecting, and direct network connections are outside this
feature.
Sessions remember view mode, Fold's directory, both Commander directories,
and the active pane. A CLI directory overrides the active view's location.

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

## Workers and the one function they fold through

`starfold-io` (listings, directory summaries, external opens, the mtime poll),
`starfold-preview` (preview supervision), and `starfold-ops` (the operations
queue, one entry at a time) are the core workers. The IO worker owns and joins
the preview supervisor. They use plain `std::thread` plus
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

Embedded audio is separate from these core workers: `audio_embed` supervises
an optional `staramp embed --stdio` child over versioned JSON lines with its
own bounded communications and latest-frame slot. It never mutates fold
state. The UI captures the active filtered/sorted listing as the playback
queue; it receives styled cells and optionally bounded RGBA transport images,
not terminal escape sequences. STAR/AMP owns the player UI, button artwork,
layout, playback state, and mouse hit-testing, reusing its standalone renderer.
STAR/FOLD only forwards input/presentation and paints the returned content;
do not recreate player widgets or transport geometry here. Closing the
player or exiting must shut down and reap this child. No STAR/AMP crate is
linked into STAR/FOLD.

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

Operations start when requested, in serial queue order. `y` saves paths for
later pastes; `p` and `m` start work, and `dd` confirms once before deletion.
`ctrl+x` stops active work and pauses later requests; `enter` or `X` resumes.
`esc` clears whatever has not started. A delete goes to the trash where the
platform has one, and the actions menu asks before permanent deletion when
configured. `plan` never follows a symlink, and a conflict
is decided by comparing `dev` and `ino`, not by whether a path merely exists
— a case-insensitive filesystem can otherwise see a file collide with itself.

An `Op`'s `progress: Arc<ops::progress::Progress>` (three atomics: done,
total, cancelled) is the one piece of an `Op` shared with the thread actually
running it — the same `Arc` goes out in `Job::Run` and back through
`state.queue`, so the status row reads live numbers without taking the write
lock, and `Command::StopActive` pauses the queue and sets the cancellation
flag without waiting for the ops thread. The ops thread checks it between
items and between bounded file chunks.

Copies stream into private files in the destination directory, with byte
progress and cancellation checks. An incomplete file is removed when work
stops; a completed file is published by rename. Permissions, optional file
times, and extended attributes are copied before publication. This forgoes
the APFS clone optimization of `std::fs::copy` so a large file can be stopped.

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
a `Handle` with no worker threads behind it: same `State::new`, with a
deterministic first listing for frame tests, but the two job queues are
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

2026-09-23: Embedded STAR/AMP passed Linux real-process tests with silent WAV
playback, pause/resume, seek, volume, captured queue navigation, compact/full
cell frames, stop and shutdown. An isolated PTY exercised 60×21 and 100×30,
Fold/Commander browsing while pinned, resize/theme changes, and Preview close
reaping the child. Embedding created no STAR/AMP config/history/cache files.
Audible listening, physical USB/network audio, and macOS embedding remain
unverified. The separately installed executables were not replaced.

2026-09-22: Linux PTY smoke checks passed at 100×30 and 60×21 using an
isolated STARFOLD_DIR: switching views/panes, navigation, bookmark creation,
Places discovery of the mounted GVFS root, clean exit, and session restore.
Snapshots and fake-core tests cover transfers, pane isolation, mouse focus,
unavailable locations, bookmark persistence, and failure reporting. Physical
USB/network transfers and macOS builds were not available on this machine.
The following broader terminal/platform checks are still outstanding:

- Resizing and focus events in an interactive desktop terminal.
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


## Modular previews and archive operations

`fold::preview::providers` is the registry: providers open sessions and sessions
return presentation data from `preview::model`. PDF sessions retain the parsed
document, returning three requested pages. `preview::connection` owns a bounded
cache keyed by path, device/inode, size, mtime and ctime, and a disposable
`starfold --preview-worker` child. Cancellation and deadlines kill/reap the child;
closing Preview releases the session. The UI keeps at most 24 PDF pages and can
request evicted pages again. Backend APIs never reach the renderer.

Large raster images can use `vipsthumbnail` as an optional preview fallback.
It runs with a deadline and its output is decoded under the bounded preview
size. Ordinary images decode in process, so the helper is not needed for
normal camera photos.

`fold::archive` is independent of preview presentation. It owns archive entries,
formats, validation and codec adapters. `archive::operation` plans source trees
and owns private staging/publishing; `archive::connection` supervises codecs in
`starfold --archive-worker`, reports progress, and terminates on cancellation.
Failed or cancelled work cannot publish partial archives/extractions. Codec
processes share a 768 MiB resource ceiling from `fold::process`. RAR uses bundled
native UnRAR: the considered Rust port also had GPL terms, so it is not linked.

Archive destinations must be disjoint from their sources in both directions:
overwriting a directory containing the source archive would delete it during
staging cleanup. Check aliases when planning and again before publishing.
Conflict renaming preserves the full archive suffix (including `.tar.gz`).
Archive regression fixtures are generated in temporary directories, never
checked in as archive files.

The pinned sevenz-rust2 0.20.2 writer inverts empty-entry anti bits. The adapter
compensates when writing directories; its round-trip test and external 7z check
cover this. Revisit that compensation when upgrading the pinned backend.

Context menus capture the clicked path and applicable marked set when opened.
`ui::overlays::context` owns menu/dialog presentation; `ui::app::file_actions`
translates responses to core commands. Compression and extraction use OPERATIONS,
not a second job queue. Shared cheap classification in `fold::file_type` supplies
Unicode icons without doing filesystem IO during drawing.

Directory previews use `preview::directory`: a bounded, cancellable walk returns
a tree of values, and `ui::panels::preview` supplies branches/icons and scrolling.
Counts describe displayed entries; marked-directory sizes remain the independent
`summary` worker job and must not replace the tree when they complete. Directory
trees are rebuilt on request, not stored in the file parser cache.

Fullscreen repaints must not call `Terminal::clear`: Ratatui 0.30 asks the
backend for the cursor position there and a missing reply aborts the UI.
`Terminal::resize` with the current size clears and invalidates the fullscreen
buffers without that query. `tests/terminal.rs` exercises startup, Ctrl+L,
resize and clean exit on a PTY that never answers terminal queries.

## Release targets

Release Linux through Nix, AppImage and native Arch pacman packages, and macOS
through the native Apple Silicon archive (Nix remains available there too).
Do not restore Debian or standalone Linux binary tarball build jobs.
`scripts/build-dist.sh` accepts `nix`, `appimage`, `arch` and `macos`.
Arch packaging builds the current sources with the locked dependencies as an
unprivileged user, runs tests and verifies installation in a fresh container.
The generated PKGBUILD and checksummed source archive accompany the package.
AppImages build natively on x86_64 and ARM64 Linux, with separate checked
runtime hashes and matching smoke/FUSE tests. Do not mix architectures in the
AppImage target cache or artifact names.
Branch release dispatches build artifacts without publishing, while version
tags create a draft release.

## Native terminal graphics experiment

On `experiment/terminal-graphics`, STAR/KIT owns the native Rust pixel renderer
(`tiny-skia` and `cosmic-text`). Do not add browser engines, Electron, webviews
or JavaScript rendering. The persistent FOLD controller and SSH scene relay
remain shared with the cell presentation. Build through `nix develop -c cargo
build --release --features terminal-graphics`; the local `starfold-graphical`
wrapper invokes this checkout with the `graphical` subcommand. Leave the
ordinary `starfold` link intact. Bundled font licensing ships in
`LICENSES/OFL-Liberation.txt`. Renderer tests belong in KIT; FOLD tests cover
application behavior and the relay. Physical macOS Kitty interaction remains
a separate gate from headless CI.

## Native surface experiment

Pixel chrome uses structured panel regions and KIT surfaces; it must not infer
panels by scanning border characters. The cell controller supplies logical hit coordinates; KIT pixel placements
map terminal cell-centre input onto those coordinates.
STAR/AMP owns `native_surface_v1`; this repository only negotiates, validates,
places and forwards input. The graphical Nix package pins its matching helper
on a private runtime PATH, leaving the ordinary STAR/AMP command independent.
Video Preview uses `staramp embed --stdio --transport` for AMP-owned button
artwork, layout and pointer hit testing without opening an audio player.
`video_transport` bounds requests and frames, validates surfaces and reaps the
helper when Preview closes. FOLD maps returned actions to its video controls.

### Native pixel layout and verification

`graphical::pixel_placements` uses KIT column/split helpers and Metrics for
actual pane gaps, margins and header/border insets. `ui/layout.rs` still supplies
logical controller rectangles for compatibility; its cell gaps are not painted
by a `pixel_layout` frontend. KIT uses the same placement row edges for raster
painting and pointer projection. Scrollbar captures stay in their source region,
while file drags resolve the current receiving region. Unoccluded base buffers
are captured before modal painting, and popup spans are separate layers.

Keep ordinary cell clients and older graphical clients on their negotiated
geometry. Terminal pointer precision remains cells. Physical macOS Kitty
interaction remains separately unverified; green headless CI is not that proof.

### Original remote movies

KIT's negotiated `original_media` capability streams opaque, generation-scoped
256 KiB file ranges over the existing relay. KIT owns seekable custom IO, local
decode/audio, bounded cache (64 MiB), read-ahead and cancellation; FOLD owns the
Auto/Original/Preview preference and status presentation. Original must not
silently downgrade on bandwidth stalls. Auto may use Preview after a decode/open
failure. Older clients keep the proxy. Subtitle selections currently require
Preview; switching to Original must not silently discard explicit subtitles.
Original bytes and 4K decode are covered by KIT's movie integration test. Physical
Kitty playback is a separate gate; its SDR presentation does not prove native HDR.
2026-10-06: A muted Mac frontend check read the user's remote 4K80 MKV through
SSH range requests and decoded 124 original 3840×2160 frames across five seconds
of playback with zero decoder drops (74 Mb/s source, approximately 10 MiB cached).
This checked transport and local decoding, not visible Kitty pacing or listening.

### SSH audio output

Preview offers `audio: host` / `audio: local` (`a`) to graphical SSH clients
that negotiate `audio_relay`. AMP owns decoding, queue, volume and seeking;
its `audio_relay_v1` embed capability exports bounded 48 kHz stereo PCM instead
of opening the host device. KIT plays it on the presentation machine. Credits
bound both transport and playback queues. Stream epochs invalidate queued audio
on seeks and reattachments; closing Preview closes the local output and reaps
AMP. Ordinary cell clients and older helpers retain host playback. The toggle
is a session preference, independent of player artwork settings.

`scripts/test-ssh-audio.py` exercises AMP's real child protocol without a sound
device and optionally the FOLD controller with a simulated remote frontend.
Physical macOS Kitty audio over the user's SSH connection still needs a
listening check; generated PCM and headless controller tests do not prove it.
