# Native graphical STAR/FOLD inside Kitty

This presentation uses the established STAR/FOLD controller with STAR/KIT's native
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
Kitty supplies the final image display. Normal `starfold` chooses graphical presentation in supported Kitty terminals
and cells elsewhere. F9 switches live; `--cells` and `--graphical` override the
saved preference for one launch. F8 / Shift+F8 change themes, with Alt+T aliases. The graphical layout defaults to 100% of the terminal’s
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

## Animated image previews

Animated GIFs play automatically in Preview, retaining transparency, frame
intervals and loop counts. Image zoom and the pixel/smooth sampling toggle work
on every frame. The native frontend receives a lossless APNG asset once and
plays it locally; animation frames are not repeatedly transferred over SSH.
The ordinary cell view advances composited frames through its image backend.
Older graphical frontends receive individual frames until they support negotiated
local animation playback.

Decoding runs off the UI thread, with cancellation, a deadline, a 64 MB lossless compressed frame
budget and a 4,096-frame cap. Extremely large animations fall back to a still
preview; sequences exceeding the frame budget play a bounded portion and log
that truncation. Native transport thumbnails never enlarge small pixel art.

## SSH

Local launches default to session `local`; SSH launches (including launches
inside an SSH shell) default to `default`. This keeps a local window and an SSH
window from repeatedly replacing each other's frontend and cancelling playback.
Each session has one attached frontend. Use distinct `--session` names for
additional simultaneous windows; explicit names and `--attach` retain their
normal behavior. To open a previously saved local `default` workspace, use
`starfold-graphical --session default` after detaching its SSH view.


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

### Build the macOS frontend

Check out `experiment/terminal-graphics` and build the native frontend on the
Mac running Kitty:

```sh
git switch experiment/terminal-graphics
git pull --ff-only
nix develop -c cargo build --locked --release --features terminal-graphics
target/release/starfold graphical --ssh HOST --remote-executable /path/to/remote/starfold
```

The remote executable must be the feature-enabled binary, rather than a wrapper
that always launches the ordinary CLI. Video audio plays through the Mac's
output device and starts at 80%; click the audio percentage or press **M** to mute. Launching a renderer on the
SSH host without the local Kitty integration sends audio to that host instead.

### Remote movie quality

Current graphical clients negotiate original-file streaming. With Preview focused,
press **O** or click **mode** to cycle **Auto → Original → Preview**. Auto starts
with Original and falls back to Preview if opening or decoding fails. Original
keeps the source quality and buffers instead of reducing resolution when the
connection falls behind. Preview uses the adaptive, lower-bandwidth proxy.
Older clients retain that proxy without requiring an upgrade.

Original streams seekable file ranges over SSH and decodes video and the selected
audio track on the presentation machine. It avoids server-side video/audio
re-encoding and retains the source resolution before fitting it into the window.
The playback status shows **SSH stream**, the mode, source bitrate, buffered bytes
and dropped frames. Both machines need matching feature-enabled builds.

The compressed-data cache is bounded to 64 MiB, with an 8 MiB read-ahead target
and a 4 MiB initial buffer. Seeking starts a new stream generation; stale replies
cannot enter the new decoder. This does not eliminate network or rendering
limits: sustained playback needs sufficient bandwidth and a fast local decoder.

Kitty presentation remains SDR RGBA, with HDR tone mapping where needed; original
transport does not provide native HDR output or surround-audio passthrough.
Embedded and external subtitles currently require Preview mode. Turn subtitles
off before returning to Original; automatic forced subtitles are not rendered
in Original mode.

### Launch after an ordinary SSH login

With the experimental STAR/KIT Kitty integration installed on the **client
machine**, use the normal workflow:

```sh
ssh HOST
starfold-graphical
```

The remote launcher detects the integration and starts the local native Rust
frontend inside the same Kitty window. It reuses the existing SSH TTY: no second
connection, additional login, host alias guessing or `--ssh` flag. Rendering,
font resolution and desktop drag handling stay local; the remote session owns
files, operations, navigation and previews. Copy/Move menus can release the
desktop pointer immediately, with Move cleanup deferred until remote delivery.

Install the Kitty watcher and fixed local frontend once per client machine as
described in [STAR/KIT's integration instructions](https://github.com/bstar/starkit/tree/experiment/terminal-graphics/integrations/kitty).
Open a new Kitty window after installation. The remote STAR/FOLD build must
also be current and feature-enabled. Without the integration, an ordinary SSH
launch retains the original remote frontend and its desktop drop limitation.
General Kitty remote control does not need to be enabled.

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
pointer grab making the menu unclickable. Kitty's temporary drag files and macOS
promises retain their terminal offer until consumption is done.
For graphical SSH on Linux, launch the client locally with
`starfold-graphical --ssh HOST`. STAR/KIT captures local URI capabilities and
releases the desktop drag without waiting for disk IO, then streams file data over SSH when
Copy/Move is chosen. Move removes local sources only after successful remote
publication; changed sources and cleanup errors remain in OPERATIONS. Both ends
need a current feature-enabled build. Ordinary SSH shells use this same local
frontend when the STAR/KIT Kitty integration above is installed. Without it,
they retain the Kitty transfer path and its Hyprland mouse limitation.

`scripts/test-graphical-ssh-drag.py` tests real Nautilus drops through a private
loopback SSH connection on Hyprland. It requires Kitty, Nautilus, OpenSSH, grim,
Hyprland's Lua dispatch API and writable `/dev/uinput`. It explicitly injects
mouse input on the desktop and uses temporary files, keys, configs and sessions.
The fixed fixture needs a `1897x1040` desktop area at the given origin. Run:

```sh
python3 scripts/test-graphical-ssh-drag.py --inject-host-mouse --origin 2574,1106 \
  --binary target/release/starfold --kitty /usr/bin/kitty --kitten /usr/bin/kitten \
  --output /tmp/fold-ssh-drag-proof --tree
```

It verifies mouse Copy/Move actions, transferred bytes, symlink preservation and
source removal after Move. Omit `--tree` for the regular-file case. This is a
loopback integration gate, not a WAN measurement.
Add `--plain-ssh --watcher /path/to/starkit/integrations/kitty/star_kit.py` to
exercise an ordinary `ssh -tt` login followed by the remote graphical launch,
including terminal negotiation, local frontend startup and the existing TTY
message relay. The installed `~/.local/bin/star-kit-terminal` must invoke the
tested build's `--graphical-terminal-client` entrypoint.
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


## Native video Preview

Preview shows an animated loading indicator while opening a movie and starting playback. Buffering is shown beside the timeline until playback resumes. These states appear in both graphical and cell modes; cell-mode letterboxing matches the surrounding preview pane.

Select a video to see its local movie details, including embedded IMDb tags and an IMDb title link when an ID is available. Details work offline; ratings and synopsis appear only when present in the file tags. Press **Enter** on
the file or click **Play** in the Preview header to start playback. Audio starts at 80%. Header controls
show the volume percentage and provide Play/Pause, mute, volume, Expand/Restore
and Close. Muting restores the previous volume when toggled again. Click the full
width timeline to seek. Dragging pauses playback and previews the target time;
release seeks once and restores the previous play/pause state. The graphical transport row reuses STAR/AMP's native
play, pause, stop, backward/forward buttons and volume slider. Backward/forward
seek five seconds; Stop returns to the movie details. The matching AMP helper supplies
the artwork, layout and hit testing without opening another audio player.
Focusing Preview keeps its height unchanged; only Expand
or dragging the divider changes it.

Enter on a video file starts its video Preview directly, including MKV files,
without sending it to the music player. Conversion starts from the first decoded
frame, so an incomplete stream header cannot abort FFmpeg during startup or seek.

With Preview focused: **P** or **Space** plays/pauses, **Left/Right** seek five seconds,
**M** toggles mute, **+/-** adjusts volume, and **E** expands/restores Preview.
**F** enters desktop fullscreen; **Shift+F** fills the terminal without changing
its desktop window. **Esc** restores the file manager while playback continues.
**A** opens audio tracks and **S** opens subtitles. Both menus support mouse
selection and keyboard navigation; subtitles include Off and Load subtitle file. Position, buffering, mute and quality appear in
the status footer. Streamed playback is labeled **SSH stream** in the playback
details and status footer. The unplayed timeline track is slightly lighter than
the panel in dark themes.
Audio-device failures allow silent video and show a notice
inside Preview. Ordinary cell mode shows the poster and metadata only.

STAR/KIT owns the native FFmpeg decoder, H.264/AAC encoder, bounded scheduling,
local CPAL audio, timeline surface and media transport. STAR/FOLD selects files
and supplies controls. This does not invoke a browser, Electron, MPV, or a new
window. STAR/AMP remains a separate player owned by its own repository.

For SSH, the host creates a small MPEG-TS/H.264/AAC preview while the Kitty
frontend decodes and plays audio locally. It uses the existing scene connection;
there is no whole-file download, extra login or open media port. Start at 480p /
24 fps / up to 1.5 Mb/s, reduce to 360p / 15 fps / 750 kb/s after repeated stalls,
and try 720p / 30 fps / 3 Mb/s after 20 seconds of smooth playback with measured
transport headroom. AAC is 96 kb/s.
Encoding never upscales and is bounded by the visible Preview. Quality changes
and seeks start a fresh generation at the current position; stale chunks cannot
enter the new decoder. Media uses 32 KiB chunks and a 256 KiB credit window;
control and scene messages have priority. Decoded video queues hold one frame
and audio rings hold half a second of samples. Closing, changing files, quitting
or disconnecting cancels playback and streaming.

Both the host and local Kitty frontend need builds with `terminal-graphics`.
Native build dependencies: FFmpeg with libx264, libavfilter, libswscale and
libswresample, clang/libclang and pkg-config; Linux additionally needs ALSA.
Nix supplies these and matching PipeWire/PulseAudio plugins. macOS archives
require Homebrew FFmpeg with libass, zscale and tonemap filters. Physical macOS
audio/Kitty interaction still needs testing.


### Local fullscreen movies and 4K

```sh
starfold-graphical --play "/path/to/movie.mkv"
starfold-graphical --ssh user@host --play "/remote/path/to/movie.mkv"
```

Local playback decodes the original video dimensions and follows its timestamps;
it does not use the SSH preview proxy. FFmpeg attempts VideoToolbox on macOS and
VAAPI on Linux, falling back to software when device initialization is unavailable.
Nix Linux launchers provide Mesa's VAAPI driver path unless overridden. Intel or
other drivers can be selected through `LIBVA_DRIVERS_PATH`; `STAR_VIDEO_SOFTWARE=1`
disables hardware decoding for diagnosis. Logs report the initialized decoder.

Frames pass to Kitty as raw RGBA, separate from application chrome. Local Kitty
uses temporary-file transfer, avoiding PNG compression and base64 frame expansion.
Kitty performs display scaling. Hardware decode still downloads frames for CPU
color conversion and subtitle composition; this is not an all-GPU pipeline.

Embedded audio tracks, text/ASS subtitles, PGS/DVD bitmaps and matching SRT/ASS/VTT
sidecars are selectable. Track changes restart at the current position and retain
pause/volume. Local audio uses decoded PCM, prefers the source sample rate and
channel count when supported by the output device, and reports necessary conversion.
It does not promise encoded HDMI bitstream passthrough. HDR is tone-mapped to SDR;
Kitty's RGBA presentation is not HDR output. The SSH path burns the chosen subtitles
and sends the selected audio through its existing adaptive H.264/AAC proxy.

Desktop fullscreen uses a fixed local Kitty kitten and restores the prior window
state and tab layout. During playback it hides Kitty's tab bar, removes window
padding and margins, and uses a black background. Exit restores the previous tabs,
spacing and background. Movies retain their aspect ratio, with black letterboxing
where needed. Kitty remote control must allow the operation; if unavailable,
terminal fullscreen still works and you can use Kitty's fullscreen key.
Controls sent through the terminal request no reply, so desktop fullscreen cannot
inject control-response bytes into STAR/FOLD's keyboard input. Socket controls use
their separate reply channel.
Controls hide after three seconds while playing and reappear on pointer/key input.
On macOS, the mouse cursor also hides after three idle seconds during fullscreen
playback. Movement reveals it immediately; input, pause, focus loss and leaving
the player restore it. Browsing does not trigger this hide policy. Linux retains
Kitty's own cursor-hiding preference.

Bounded six-second decoder/conversion measurements on an AMD Radeon 8060S with
VAAPI: two actual 3840×2160 movies sustained approximately 24 fps. Generated 4K
24/30 fps sources kept pace; a 60 fps source reached approximately 46 fps.
A 3840×1600 HDR movie also kept pace at approximately 24 fps with tone mapping.
These measurements exclude Kitty upload/display and do not guarantee smooth 4K60.
Run KIT's `movie-bench` example with `--features media` to measure another machine.
