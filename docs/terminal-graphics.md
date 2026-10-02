# Native graphical STAR/FOLD inside Kitty

This experiment uses the established STAR/FOLD controller with STAR/KIT's native
Rust renderer. There is no Electron, Chromium, webview, HTML, CSS or JavaScript
rendering. The earlier separate GPUI desktop experiment remains on
`experiment/graphical-presentation`; this mode draws inside the existing terminal.

## Run

```sh
nix build .#graphical
./result/bin/starfold-graphical ~/Pictures
starfold-graphical --session work ~/projects
starfold-graphical --sessions
starfold-graphical --attach --session work
starfold-graphical --capabilities
```

For a local release build:

```sh
nix develop -c cargo build --release --features terminal-graphics
target/release/starfold graphical ~/Pictures
```

No separate graphical runtime or display server is required by the renderer.
Kitty supplies the final image display. Normal `starfold` continues to use the
standard CLI/TUI. The graphical layout defaults to the terminal’s measured size (100% scale):

```sh
STAR_GRAPHICS_SCALE=150 starfold-graphical
```

Accepted scale values are 100–200. File labels use shaped sans-serif text,
terminal-compatible chrome uses monospace, and labels are clipped to one line.
The same theme colors, hit targets, stack/Commander layout and 60×21 floor apply.
Bundled fonts provide the baseline; installed fonts provide Unicode fallback.

## SSH

```sh
nix build .#graphical-host
starfold-graphical --ssh HOST --session work /remote/path
starfold-graphical --ssh HOST --remote-executable /path/to/starfold --session work
starfold-graphical --ssh HOST --attach --session work
```

The local machine runs the Rust raster renderer. The remote executable runs the
persistent application controller and filesystem workers. Both are native Rust.
SSH authentication finishes before terminal raw mode. A renderer/frontend failure
leaves remote operations and persistent session state available for reattachment.
The remote executable must include the `terminal-graphics` feature.

## Feature alignment

This presentation retains Fold's stacked navigation, Commander panes, independent
sorting/filtering, tabs, Places/bookmarks, previews, operations, marks, file locks,
copy conflicts, persistent errors, archive actions and authorization prompts.
Keyboard and pointer input use the existing controller. Desktop drag/drop is a
separately detected OSC 72 extension; image support alone does not supply it.
Clipboard output is local OSC 52, with tmux buffer support where needed.

Raster previews are decoded once, resized once per geometry and reused during
ordinary repaints. The duplicate half-block preview is omitted in pixel mode.
A completed graphical thumbnail wakes the scene immediately instead of waiting
for the idle refresh. CLI and cell-fallback previews retain their existing paths.

## Verification

```sh
nix develop -c cargo test --all
nix develop -c cargo test --features terminal-graphics
nix develop -c cargo clippy --all-targets --all-features -- -D warnings -A dead_code
```

The shared KIT tests verify native pixels, Unicode shaping/clipping, image cache
reuse, resize, password masking, invalid input, dirty-region reconstruction,
pointer input and actual Kitty screenshots. Shared headless tests need no window
system. Linux native Kitty screenshots run under Xvfb. Interactive macOS Kitty
still requires a real logged-in Mac and remains a separate verification gate.

`scripts/test-graphical-ssh.py` exercises the real SSH relay and filesystem
operations. `scripts/measure-graphical-latency.py` uses a private fixture and Kitty
>=0.49's screenshot API to measure startup and input through observed pixels:

```sh
python3 scripts/measure-graphical-latency.py \
  --binary target/release/starfold --kitty /path/to/kitty --kitten /path/to/kitten \
  --output /tmp/fold-latency --samples 9 --navigation-only
```

Screenshot timings include remote-control/capture overhead and must not be
reported as pure rendering latency. Browser-era performance and resource numbers
do not describe the native replacement.

Sessions and controller logs live under `~/.local/starfold/graphical` (or
`STARFOLD_DIR/graphical`). Application debug logs remain in the usual log directory.
The source of the shared renderer is in STAR/KIT's `src/terminal_graphics/native.rs`.

### Native chrome and embedded player ownership

The graphical presentation uses KIT native surfaces for compact pane toolbars,
continuous storage meters, and the padded status bar. Tabs keep their close
buttons and grow with the label within a bounded width. Operations collapses to
an idle/history strip and expands for active work, failures, or explicit focus.
These surfaces are negotiated using the `native_surfaces` frontend capability.

STAR/AMP's native player lives in STAR/AMP (`src/embed/native.rs`). The optional
`native_surface_v1` embed extension returns KIT primitives and owns transport,
seek, volume and visualizer hit geometry. FOLD places the returned surface and
forwards input through the existing supervised child protocol. Older helpers
continue to provide cells. Local animation is capped at 30 FPS and SSH at 15 FPS;
hidden native players stop publishing frames while audio continues.

The graphical Nix package includes a matching helper. For a manual build, put the
matching STAR/AMP experimental build on the graphical launcher's private PATH.
This does not require replacing the normal `staramp` command.
