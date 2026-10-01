# Graphical interface inside the terminal

This is a fresh experiment from STAR/FOLD main `6cbee38` (v0.0.2), on
`experiment/terminal-graphics`. The separate native GPUI experiment remains on
`experiment/graphical-presentation`.

## Direction

Use Awrit's offscreen-browser-to-terminal rendering approach. The graphical
interface must appear inside the existing terminal and preserve STAR/FOLD's
stack navigation and workflows. Preview and Operations retain their established
placement; Commander splits the browser only.

The reusable renderer, component model, terminal image transport, input routing,
capability negotiation and lifecycle belong in STAR/KIT. STAR/FOLD supplies its
application state and semantic actions. Copy, delete, rename and other filesystem
work continue through the existing controller and workers.

The authoritative foundation design, reference analysis, acceptance gates and
implementation sequence are in
[STAR/KIT's terminal graphics plan](https://github.com/bstar/starkit/blob/experiment/terminal-graphics/docs/terminal-graphics.md).
It incorporates Awrit's rendering path and cmux's surface identities, frame guards,
queue bounds, protocol boundaries and SSH transport lessons.

## Running it

The backend is optional and isolated from the normal launcher. Locally, inside Kitty:

```sh
starfold-graphical ~/Pictures
starfold-graphical --session work ~/projects
starfold-graphical --sessions
starfold-graphical --attach --session work
starfold-graphical --capabilities
```

For SSH, install the feature-enabled Rust host on the remote machine:

```sh
# On the remote machine; no Electron or display server is needed:
nix build .#graphical-host
# On the local machine, inside Kitty:
starfold-graphical --ssh HOST --session work /remote/path
# If the remote host executable is not called starfold:
starfold-graphical --ssh HOST --remote-executable /path/to/result/bin/starfold --session work
starfold-graphical --ssh HOST --attach --session work
starfold-graphical --ssh HOST --sessions
```

`--ssh-config FILE` selects an SSH config without changing user configuration.
Host-key/password/key-passphrase authentication happens before graphical raw mode.
Ctrl+Q detaches; remote filesystem operations keep running. A disconnected view
shows a persistent status and automatically reattaches; Ctrl+R retries immediately.
The normal q action closes the application session. Reconnect does not repeat
copy/delete/rename input. Named sessions retain their own workspace and operations;
only one frontend controls a session at once.

Nix `.#graphical` supplies the local Electron runtime and `starfold-graphical`
wrapper. `.#graphical-host` includes only the Rust controller/relay. To build by
hand, use `nix develop -c cargo build --release --features terminal-graphics`,
then run `target/release/starfold graphical`. Set `STAR_GRAPHICS_ELECTRON` to an
Electron 43.6.0 executable when it is not named `electron` on PATH. Ordinary
`starfold` and the earlier native GPUI launcher remain separate.

Without detected Kitty image support, the launcher automatically presents the
same controller session using terminal cells. No Electron process is needed for
this mode. Detach, reattach, SSH and file operations retain the same semantics.
The capability report distinguishes image transport, measured or estimated pixel
geometry, cell pointer precision, keyboard and paste support. Image support does
not imply pixel-precise mouse input.

## Feature alignment

This adapter calls the established controller and workers, with the existing hit
regions and shortcuts. Rounded pixel panels, filled tabs, SVG file icons, marked
rows, single-line filename truncation, thin storage meters and cached preview
images use shared STAR/KIT components. Existing headers, menus, dialogs, editor
PTY cells and operations content use styled compatibility spans, so actions do
not disappear while their visual primitives evolve.

| Workflow | Graphical path |
| --- | --- |
| Stack and Commander | Same stack model, independent pane sort/filter, captured paths |
| Preview and Operations | Same established module placement and layout controls |
| Tabs, Places, bookmarks, themes | Same actions, menus, sessions and Alt+T behavior |
| Copy/move/delete/rename/create/archive | Existing planning, conflicts, capacity checks, locks and workers |
| Progress, errors and cancellation | Same operations model and persistent failures; report copies to local clipboard |
| Pointer drag and scrollbars | Captured sources; source scroll freeze; target autoscroll; independent scrollbar ownership |
| Kitty desktop drag/drop | Existing OSC 72 protocol through acknowledged SSH effects; interrupted streaming transfers cancel |
| Embedded editor/player | Remote/local application-host processes; styled editor/player cells and cached player transport images |
| Administrator delete retry | Explicit existing failed-operation action; masked password authorization on the host |

Filesystem operations never run in browser JavaScript. A permission failure
remains in Operations; administrator retry remains an explicit action. Passwords
are not logged or included in scenes. Sudo policies requiring a real TTY may
reject graphical authorization; this path is not a general privileged shell.
Remote external applications open on the remote host; audio/video is not forwarded.

Full filenames remain accessible through preview, rename and clipboard actions.
Native terminal text selection and screen-reader text do not operate on PNG
pixels; use the ordinary TUI when those are needed.

The local presenter uploads only changed cell-aligned image regions and keeps
at most 256 placements. Each update is compared with the pixels actually sent to
the terminal, so dropped renderer frames remain safe. Acknowledgement-based scene
pacing keeps one scene in flight and coalesces subsequent application updates;
resize supersedes obsolete geometry without waiting. Filesystem workers continue
independently while presentation waits for a slow connection.

## Verification

- Linux: shared protocol/frame/queue tests, FOLD's existing controller suite and
  real persistent headless-host integration tests pass.
- Real isolated SSH server, with DISPLAY/WAYLAND_DISPLAY removed: 64 MiB marked
  copy plus Unicode/quoted path checksums, disconnect, same-process reattachment
  and duplicate input rejection pass. The fixture changes no user SSH settings.
- `scripts/test-graphical-ssh.py --binary target/debug/starfold --rtt-ms 50 --bandwidth-mbps 10`
  reproduces the transport test. Modeled latency/bandwidth is applied to messages;
  it is not a measurement of an actual WAN. The test now negotiates and
  exercises presentation pacing; observed first listing was 313.4 ms and
  input-to-ack p95 74.7 ms, with copy checksums and reattachment verified.
- 100,000-file synthetic listing: cursor updates reuse row storage and publish
  only visible rows; measured controller+scene p95 1.848 ms and <100 KiB JSON.
- Cell fallback: a real tmux session renders the full interface without Electron;
  a PTY regression test performs a copy and clean shutdown with no display server
  and an unavailable Electron executable. Real tmux checks also verified resize,
  Ctrl+Q detach, reattachment to the same controller process, and path clipboard
  copy with the default `set-clipboard=external` policy.
- Ghostty 1.3.1 on Linux: actual graphical file view, Preview, Operations,
  actions-menu keyboard input, session reattachment, and font-size changes
  rendered successfully. The isolated Nix test used X11/software Mesa after the
  initial driver setup failed; native Wayland and Ghostty SSH remain unverified.
- Changed-region presentation: lossless reconstruction, skipped-frame handling,
  resize/cleanup, decoder limits and bounded placements are tested in STAR/KIT.
  Real Kitty displays the FOLD actions menu and retires its regions when closed.
- Local Kitty: the application pixels render inside the existing terminal with
  no visible native Electron window.
- Live Kitty resize checks produce frames with the new geometry. The SSH view
  reattached to the same controller after a 40-second interruption and repeated
  failed connection attempts; stale frames did not enable input.
- A five-minute full-size Linux idle sample (61 samples) measured 1.14% mean
  aggregate CPU and 469.02–469.07 MiB aggregate PSS, with no measured growth.
  This includes the frontend, controller, relay and Electron processes, and
  excludes Kitty itself. Electron's memory cost remains substantial.
- Linux/macOS feature builds and shared offscreen smoke capture are CI gates.
  Shared Linux Kitty CI also checks actual pixels, keyboard/font resize and
  controlled renderer loss. macOS interactive Kitty and broader terminal
  verification remain promotion gates. This is still an experimental branch.
- The current region presenter passed a fresh Ghostty 1.3.1 Linux check using
  X11/software Mesa: navigation/marks, menus, font zoom with a new geometry
  generation, and clean application/controller exit. This is one tested terminal
  version/platform, not a claim about every graphics-capable terminal.
- A fresh ten-minute local mixed workload completed 949 iterations after a
  checksum-verified 256 MiB copy. It exercised navigation, previews, actions,
  themes and repeated font zoom. All process memory samples were readable;
  aggregate PSS ranged 618.70–1379.81 MiB (median 954.36 MiB), and active CPU
  averaged 96.68% of one core. It presented 3,781 frames and uploaded 255,406,512
  image bytes. This is a bounded observation, not a long-term stability claim.
- `scripts/measure-graphical-latency.py` measures actual Kitty pixel changes on
  a private file/config/session fixture. Thirty navigation/preview/menu actions
  in a 1820×2048 window measured median 531.60 ms and p95 539.14 ms. It includes
  API input and screenshot overhead, and settles asynchronous preview updates
  before each input; it is not renderer-only latency or a sustained input burst.
  It requires Pillow, Kitty >=0.49 and the configured Electron runtime:

  ```sh
  python3 scripts/measure-graphical-latency.py --binary target/release/starfold \
    --kitty /path/to/kitty --kitten /path/to/kitten --output /tmp/fold-pixel-proof
  ```

Sessions and controller logs live under `~/.local/starfold/graphical` (or
`STARFOLD_DIR/graphical`); application debug logs are in the normal log directory.
The shared foundation's [design and measurements](https://github.com/bstar/starkit/blob/experiment/terminal-graphics/docs/terminal-graphics.md)
explain transport limits, image caching and compatibility.
