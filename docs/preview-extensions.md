# Preview extensions

PDF and video previews are bundled executable providers. PDF uses Hayro 0.8.0
in `starfold-preview-pdf`; its rendering dependencies are not linked into the
main executable. `starfold-preview-video` supplies posters and playback actions.
STAR/KIT still owns media decoding, audio output and SSH transport; STAR/AMP
owns transport artwork and hit testing. Ordinary cell clients display video
posters and PDF pages using their existing image/half-block presentation.

Build all providers through the flake:

```sh
CARGO_NET_GIT_FETCH_WITH_CLI=true nix develop -c cargo build --workspace --release --features terminal-graphics
CARGO_NET_GIT_FETCH_WITH_CLI=true nix develop -c cargo test --workspace --features terminal-graphics
```

The bundled helpers live beside the actual FOLD executable, including packaged
installations. Keep the three executables together. Standalone macOS release
binaries additionally carry compressed matching helpers, materialized into a
private versioned cache, because the existing self-updater replaces only FOLD.
This preserves helper versions through updates and rollbacks. Nix/macOS builds
and development builds use adjacent executables. A missing or incompatible
helper shows metadata and an explanatory notice. There is no automatic download.

## PDF controls

Focus Preview to use `n`/`p`, left/right or Page Down/Page Up for pages; `+`/`-`
for zoom; `0` for fit-page; `w` for fit-width; and up/down or `j`/`k` to scroll
a zoomed page; `h`/`l` pan horizontally. The wheel scrolls inside Preview. `t` toggles text mode. Rendering
failures offer text mode; password-protected documents report that they must be
opened externally. Rendering is on demand and the helper's raster cache follows
the preview byte budget, with at most 24 entries. Changing the file discards
its session and cached pages. Hayro has incomplete PDF feature coverage; a
successful render is not a guarantee of full fidelity for every document.

Video retains existing playback, subtitle, fullscreen and Original/Preview SSH
controls. Extensions use FOLD's media services; movies are not encoded into the
general extension message stream. URL/live-stream sources are outside v1.

## Explicit configuration

Add these tables to `config.toml` (do not place them inside another table):

```toml
[preview.extensions]
disabled = [] # e.g. ["video"]

[preview.extensions.overrides]
"application/pdf" = "pdf" # provider id, including configured replacements

[[preview.extensions.providers]]
id = "notes"
command = ["python3", "/absolute/path/to/metadata.py"]
extensions = ["note"]
mime_types = []
```

Provider ids are unique. Explicit suffix overrides (`extension_overrides`) precede
MIME overrides. Configured providers rank by longest suffix, exact MIME, MIME
family (`text/*`), then `*/*`. Higher `priority` wins within the same specificity;
ties preserve configuration order. Bundled PDF/video follow configured matches.
File classification recognizes PDF headers as well as names. An override of a
disabled provider leaves metadata available. Directory, text, image, symlink and
archive previews remain built in. Archive operations use their existing queue.

Commands are argv arrays, executed without a shell. Configured executables are
trusted programs with the user's privileges. Process limits and crash isolation
are not an OS sandbox. FOLD never searches a browsed directory for extensions.

## Protocol v1

`extensions/protocol` is the Rust SDK and authoritative wire definition. The
small Python example in `extensions/examples/metadata.py` demonstrates an
independent implementation with no Rust linkage.

Each frame starts with two big-endian unsigned 32-bit lengths: JSON bytes and
binary attachment bytes. They are followed by UTF-8 JSON and the attachment.
Limits are 1 MiB of JSON and 32 MiB of binary data, checked before allocation.
stdout is exclusively the protocol; diagnostic text belongs on stderr.

Every request/reply has `session`, `generation`, `sequence` and `message`.
Replies echo all three scope fields exactly. The host first sends `hello` with
version 1 and host capabilities; the helper returns its version, provider id,
revision and capabilities. `open` supplies raw Unix path bytes, preview limits
and a viewport/theme. `input` supplies keys, semantic actions, page requests,
pointers or viewport/theme changes. Each request receives one reply. Unsolicited
frames are rejected. `cancel`/`close` end a session; the host also terminates
and reaps stalled or superseded processes and their process groups.

Presentation supports bounded fields/text pages, one raw RGBA8 raster attachment,
validated KIT native surfaces with a cell explanation, and media-service
requests. The image attachment must be exactly width × height × 4 bytes;
dimensions must respect host limits. Controls advertise consumed keys. FOLD
retains global shortcuts, placement, focus and terminal input; pointer precision
remains cells, mapped to the extension's content coordinates. Surface hit actions
are forwarded to their owner. Extensions must not emit terminal escape sequences.
Editor surfaces may set `cell_size: [8, 18]` (source pixels) to request fixed
cell advances, shared pixel edges and connected terminal glyphs. Omit it for
ordinary shaped UI text; older frontends ignore the optional hint.

Only the current file's session remains active. File identity includes path,
device/inode, size, mtime and ctime. Generation checks discard superseded replies.
The preview worker owns IO and supervised sessions; results and durable pending
media actions fold through the same state writer as other core work. The UI
acknowledges actions after consuming them so coalesced preview notifications do
not lose playback commands. Format-specific renderers never reach into the UI.

Linux/macOS release packages ship both providers. Hayro's font, character-map
and color-data notices accompany the helpers in `LICENSES/Hayro-*.txt`.

## NVIM and HTML developer documentation

The optional `starfold-preview-nvim` helper embeds read/write Neovim editing
using an isolated MessagePack-RPC child and its real screen-grid UI. Clean previews follow the cursor and reuse Neovim across files. Unsaved changes
prompt before switching files or quitting: save, discard, or cancel. :w saves, :q closes, and :q! explicitly discards. Configure it explicitly; Neovim is an
external runtime dependency. It is built with the default workspace members.
See [the HTML developer guide](../documentation/index.html), including the
[NVIM setup](../documentation/nvim.html) and
[file-type hierarchy](../documentation/selection.html).

## Graphical and cell presentation

`Viewport.cells` optionally supplies exact `[columns, rows]` for a cell frontend.
`Presentation.cells` optionally returns a bounded `CellGrid` with RGB colors,
modifiers and cursor coordinates. Neovim uses this grid directly, without page
headings or wrapping. Text pages remain the fallback for older providers.
See [presentation and shortcuts](../documentation/presentation.html) for launch,
switching, media and configuration examples.
