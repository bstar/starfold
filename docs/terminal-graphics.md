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
standard CLI/TUI. The graphical layout defaults to 100% of the terminal’s
measured size. Increase it explicitly if desired:

```sh
STAR_GRAPHICS_SCALE=150 starfold-graphical
```

Accepted scale values are 100–200. Labels and chrome use the terminal’s font
and size when Kitty can report them; file labels are clipped to one line.
The same theme colors, hit targets, stack/Commander layout and 60×21 floor apply.
STAR/KIT resolves Kitty’s reported font locally, including for SSH attachments.
Override it with `STAR_GRAPHICS_FONT="JetBrainsMono Nerd Font"`; optionally use
`STAR_GRAPHICS_FONT_SIZE=16` for a pixel size override. If the terminal font
cannot be found, an installed monospace font or bundled Liberation Mono is used.
Swash rasterization can differ slightly from Kitty’s antialiasing and hinting.
Reconnect after changing the terminal font family. Installed fonts supply Unicode
fallback.
BMP image previews are enabled in both terminal modes.
In graphical image previews, click **scale** in the Preview header or press `z`
with Preview focused to cycle **1x**, **pixels**, and **smooth**. Pixels enlarges
at whole-number factors with nearest-neighbor sampling; 1x never enlarges.
Oversized images fit within the preview; large sources still use bounded
thumbnails. The selected mode is saved in `preview.image_scale`.
Use **− / +** beside the scale control to zoom, or press `-` / `+` with Preview
focused. `Ctrl` + mouse wheel zooms over the image. Click the percentage or press
`0` to reset. Percentages are relative to the selected scale mode; pixel mode
rounds enlargement to whole pixel blocks. Images stay clipped inside Preview.

Drag the top border of Preview up or down to resize it. The mouse changes to a
vertical resize cursor over the divider. Focusing Preview does not change its
height; the chosen size belongs to the current tab for the running session.
The handle is unavailable while a modal is open and cannot start a file drag.
Directory previews use compact rows with continuous branch lines. The footer
uses the ASCII footer’s state and priority: `? help`, recent notifications,
then operation progress (including rate and ETA), then contextual hints when idle.
Marked totals, location, and graphics status remain on the right. Hints use
compact inline text rather than graphical keycap buttons.

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

The native path row is a clickable breadcrumb trail (`~ / Pictures / Pixel Art`).
Each segment navigates its pane to that ancestor; the current segment is bold.
Long paths collapse on the left, with an ellipsis that opens the hidden parent.
Filter and truncated-list indicators remain on the right.
Clickable breadcrumbs, tabs, header actions, menu rows and dialog buttons advertise
hand-pointer regions to KIT. Hover feedback uses local OSC 22 output without an SSH
round trip; the preview resize cursor retains priority during divider dragging.

Native file drags capture the pressed file or marked set before selection changes.
Kitty OSC 72 coordinates are projected through the same pixel placements used
for painting. Kitty owns its gesture through the terminal drop event; ordinary
mouse release cannot queue a second operation. Both terminal and fallback drags
open the existing Copy/Move chooser. Source scrolling is held, destination edge
and wheel scrolling remain available, and self-drops are rejected. Unchanged
hover acceptance is not echoed repeatedly through the graphical relay.

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

`scripts/test-graphical-drag.py` injects real X11 mouse events into a private
Kitty, verifies OSC 72 was used, clicks Copy and Move, checks exact file bytes,
and bounds hover traffic. Run it only in an isolated Xvfb display; it requires
libX11/libXtst and JetBrainsMono Nerd Font for its fixed 9pt geometry:

```sh
xvfb-run -a -s '-screen 0 1920x1080x24' python3 scripts/test-graphical-drag.py \
  --private-xvfb --binary target/release/starfold \
  --kitty /path/to/kitty --kitten /path/to/kitten --output /tmp/fold-drag-proof
```

`scripts/test-graphical-external-drag.py` adds a separate GTK3 drag source and
checks that visible Copy/Move rows select the correct action in a narrow tabbed
window. It requires PyGObject/GTK3 and ImageMagick, uses the same arguments above,
and needs a private `2400x1080x24` Xvfb screen. It checks delivered bytes and the
successful Move removes the source only after delivering its bytes. On Linux,
ordinary local URI drops end the desktop gesture before waiting for the action
menu; FOLD then owns both copying and source removal. This avoids Hyprland's
pointer grab making the menu unclickable. SSH handles, Kitty's temporary drag
files, and macOS promises retain their terminal offer until consumption is done.
Mouse choices for these retained offers on Hyprland remain unverified; the local
URI fix must not be described as resolving that separate case.
Popup layers align to the cell grid so cell-precision mouse events cannot select
the adjacent action when panel padding puts a popup between rows.

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
The additive `pixel_layout` capability supplies shared KIT pixel gaps and panel
insets, independent of terminal cell proportions. Rendering and mouse projection
use the same placements, including modal layers and captured scrollbars. Fonts
retain terminal-derived sizes; text bitmaps are not resized. Older clients keep
their original cell geometry.

STAR/AMP's native player lives in STAR/AMP (`src/embed/native.rs`). The optional
`native_surface_v1` embed extension returns KIT primitives and owns transport,
seek, volume and visualizer hit geometry. FOLD places the returned surface and
forwards input through the existing supervised child protocol. Older helpers
continue to provide cells. Local animation is capped at 30 FPS and SSH at 15 FPS;
hidden native players stop publishing frames while audio continues.

The graphical Nix package includes a matching helper. For a manual build, put the
matching STAR/AMP experimental build on the graphical launcher's private PATH.
This does not require replacing the normal `staramp` command. On Linux, a raw
Nix-built helper also needs its dynamic ALSA plugins. In the STAR/AMP checkout,
build `nix build .#alsa-plugins --out-link target/alsa-plugins`, then point the
private helper link at STAR/AMP's `scripts/run-local.sh`. The packaged graphical
app includes this runtime setup automatically. `scripts/test-native-playback.py`
in STAR/AMP verifies playback and native pointer controls with silent audio on
a real output device.

Capture the native layout with an isolated fixture (Kitty 0.49 or newer):

```sh
python3 scripts/test-native-layout.py \
  --binary target/release/starfold --staramp /path/to/staramp \
  --kitty /path/to/kitty --kitten /path/to/kitten --output /tmp/fold-layout
```

Use `--theme catppuccin-latte` for the light-theme fixture and `--x11` under
Linux Xvfb. The script captures Fold, Commander/tabs, a popup, the embedded
player, pause and enlarged text. Its silent WAV checks presentation, not audible
playback. It does not modify the user's application configuration or files.

Graphical tabs use numbered sessions: a small one-based position, a soft filled
active rectangle, bold active name and muted inactive names. Numbers follow the
current session order, not the visible scroll offset. Close buttons and rail
controls retain separate click targets; tabs have no bottom underline.
