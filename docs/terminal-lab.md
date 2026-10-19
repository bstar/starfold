# Terminal compatibility lab

The Linux lab provisions 23 terminal emulators through the repository's locked
Nix package set. Hyper, Tabby and Warp remain explicit inventory entries when
normal package/license/security policies prevent provisioning them. Package
versions come from `flake.lock`, rather than an assertion about latest releases.

```sh
# Build the application and matching adjacent preview helpers first.
CARGO_NET_GIT_FETCH_WITH_CLI=true nix develop -c cargo build --release

# Project-local installation; does not change the user's package profile.
nix build .#terminal-lab --out-link result-terminal-lab
./result-terminal-lab/bin/starfold-terminal-lab --list

# Full run. Choose a new or empty output directory.
nix run .#terminal-lab -- --binary ./target/release/starfold \
  --output ./target/terminal-lab/run-001 \
  --profiles cells,auto --themes all --media --idle-seconds 70

# Smaller run during development.
nix develop .#terminal-lab -c python3 scripts/terminal_lab/lab.py \
  --binary ./target/release/starfold --output ./target/terminal-lab/quick-001 \
  --terminals kitty,ghostty,wezterm,alacritty,foot,xterm \
  --profiles auto --themes 2

# Multiplexer, ASCII and monochrome policy.
nix run .#terminal-lab -- --binary ./target/release/starfold \
  --terminals kitty,xterm --profiles tmux,ascii,mono --themes 2 \
  --output ./target/terminal-lab/fallback-001
```

`--binary` can also point to the installed wrapper to retain its media runtime
environment. An adjacent AMP helper takes priority over PATH; use the matching
helper. The lab does not rebuild or replace the application automatically.

## Inventory and isolation

Kitty, Ghostty, WezTerm, Alacritty, foot, Konsole, GNOME Terminal, GNOME Console, Ptyxis, Black Box,
Xfce Terminal, Tilix, Terminator, MATE Terminal, QTerminal, Terminology, Rio,
Contour, COSMIC Terminal, xterm, mlterm, st and cool-retro-term are provisioned.
Unavailable packages are reported, not silently omitted or called FOLD failures.
Hyper's interactive CLI and Warp's proprietary/login workflow also need manual
adapters before they qualify as automated targets.

Each case uses a generated fixture tree, private HOME, FOLD configuration and
D-Bus session. Short temporary paths keep Unix sockets within OS limits. Xvfb,
Openbox and nested Weston provide private displays with Nix Mesa software
rendering. Wayland-only clients run inside Weston. Screenshots capture only
these displays. User terminal profiles, desktop configuration, bookmarks, Trash
and real media/files are not used.

The recorder runs FOLD on an inner PTY and forwards terminal queries and replies
unchanged. Its observation socket is read-only. Keyboard and pointer events
enter through the actual emulator using XTest; startup requires actual keyboard
delivery. Cleanup targets processes with the exact private STARFOLD_DIR marker
and stops the displays and private buses.

## Assertions and evidence

- Startup, ordinary keys, chosen presentation and clean exit.
- Marking, navigation, and an actual queued copy verified against source bytes.
  Commander renders both panes; help opens; filtering selects generated fixtures;
  entering a ZIP lists its member; text preview displays its fixture body.
- Pointer reports arrive and wheel input changes the visible grid. X11 resizing
  changes PTY dimensions while FOLD remains alive. Nested Wayland resizing needs
  a separate compositor-specific check.
- Image transmissions are classified without counting capability queries. The
  four-color image fixture is checked in screenshots. A separate geometric check
  recognizes its quadrants after palette reduction, distinguishing a missing
  image from reduced fidelity.
- F8 changes the persisted theme live. `--themes all` follows the running binary's
  registry until a theme repeats, with a bounded guard. Each theme gets a screenshot,
  dominant colors and emitted/visible RGB measurements. This includes Gruvbox
  light and the DOS/Norton/Windows 3.11 palettes when present in the executable.
- `--media` captures a generated silent movie playing, paused, after an idle wait
  and resumed. A changed screenshot is an observation, not proof of movie motion,
  pacing, pause retention or audio correctness. Disconnects are checked separately.

When native mode is selected, the controller checks switch to cells with F9 and
then return to native mode for image/theme/movie captures. This does not establish
every native pointer target. `cells,auto` adds an explicit cell run. `tmux` uses a
new private server. `ascii` and `mono` set STAR_KIT_ASCII=1 and NO_COLOR=1; their
visual limitations are intentional.

## Reading the report

Open `index.html` in the output directory for the matrix, detailed assertions and
theme gallery. `REPORT.md` is the compact matrix; `results.json` is the structured
record. Every case retains stderr, FOLD logs/config, screen snapshots, a key-step
journal, raw input, and up to 48 MiB of raw output. Byte counters include traffic
beyond that capture cap. Preserve failed-run evidence by choosing a new directory
for each run.

| Status | Meaning |
| --- | --- |
| pass | The specific stated assertion passed. |
| fail | The assertion failed; inspect evidence before assigning its cause. |
| observed | Evidence exists but does not prove the complete feature. |
| unsupported | Feature unavailable in the selected presentation/profile; shown in red. |
| unavailable | Provisioning failed under the locked package policies. |
| manual | A separate interaction or visual review is required. |
| not-run | No claim, including deliberately unselected native mode. |
| not-observed | The protocol was not seen in this scenario; no claim about terminal support. |

The matrix includes drag-and-drop coverage and separate protocol columns for
Kitty graphics, Sixel, iTerm2 inline images, RGB SGR, SGR mouse, Kitty keyboard,
bracketed paste, OSC 52 clipboard and OSC 72 drag/drop. Protocol observations
come from saved image classification, emitted theme colors and actual input
bytes. Graphics alternatives need not all be active. Clipboard and desktop
drag/drop remain manual until dedicated interactions are recorded.

Theme switching and faithful colors are separate. A default session advertising
TERM=xterm without COLORTERM=truecolor can make FOLD reduce colors even if that
emulator supports RGB. The default run does not manufacture a truecolor hint.
Compare the recorded environment, RGB measurements, dominant colors and captures.
Terminal fonts and intentional CRT effects remain visible too.

For a controlled comparison, `--profiles rgb` explicitly sets COLORTERM=truecolor
in cell mode. This is reported as an override, never substituted for the terminal's
default result. Use `--resume` with the same output directory to retry failed or
incomplete cases; earlier case evidence is archived under `history/`. The executable
checksum must match. `--report-only` rebuilds HTML/Markdown/CSV without running any
terminal tests.

The System theme runs inside the private session, so its fallback is captured;
that is not a test of following the real desktop's theme or a user's Stylix file.

## Separate acceptance gates

OSC 52 clipboard prompts/readback, real OSC 72 desktop file drag/drop, audible
output/devices, SSH bandwidth/reattachment, remote original movies, native
fullscreen restoration, hardware GPU performance and HDR need separate tests.
Font coverage/width, legibility, clipping and theme contrast require visual
review. Protocol detection alone never counts as a behavioral pass.

iTerm2 and Apple Terminal require macOS. Windows Terminal, mintty and PuTTY require
Windows. They are external targets, not emulators that can be installed on Linux.
The lab flake outputs are Linux-only; normal Darwin outputs remain intact.

Upstream adapter/protocol references:
[Kitty graphics](https://sw.kovidgoyal.net/kitty/graphics-protocol/),
[Ghostty features](https://ghostty.org/docs/features),
[COSMIC argument handling](https://github.com/pop-os/cosmic-term/blob/master/src/main.rs).
These explain expectations; measured results take precedence.

The report includes terminal menus/window decorations; FOLD themes do not restyle
that outer chrome. `--collect-evidence` rebuilds the aggregate from completed case
records after an interrupted run. `--reassess-evidence` audits stored startup/help
observations with the current evidence parser, preserves replaced assertions and
records the review. These commands do not rerun the application. A controller
locks its output directory to prevent two new runs from sharing it.

The suite exercises representative controller paths, not every command in FOLD.
Move/delete/recovery conflicts, archive editing, PDF/comic/editor providers and
mount actions still need their core regression tests and separate UI acceptance.
The core tests remain `nix develop -c cargo test --all`; emulator results do not
replace them.

```sh
nix develop .#terminal-lab -c python3 -m unittest discover -s scripts/terminal_lab -v
```
