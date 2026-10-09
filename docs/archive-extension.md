# Archive extension protocol

Archive support ships inside `starfold`, using the extension in `extensions/archive`.
`extensions/archive-protocol` is its independent Rust contract. It has no UI,
terminal or FOLD dependencies, so another application or codec provider can use
this boundary. FOLD remains responsible for panes, previews and transactional
publication. ZIP edit staging also remains in FOLD; the extension rebuilds only
a private output selected by the host.

## Build and discovery

```sh
nix develop -c cargo build --release -p starfold
```

The default provider runs from the same executable in the private
`--archive-extension-stdio` child mode. No adjacent archive helper or separate
installation is required, including after a single-executable update. The codec
implementation remains an independent extension crate with a versioned wire
contract and isolated process lifetime. The optional `starfold-archive --stdio`
executable exposes the same provider for other consumers.
An explicit executable override is available:

```sh
STARFOLD_ARCHIVE_EXTENSION=/absolute/path/to/provider starfold
```

Codec executables are discovered beside the extension, through its package's
runtime configuration, or on PATH. Overrides are `STARFOLD_ARCHIVE_7ZZ`,
`STARFOLD_ARCHIVE_LSAR`, and `STARFOLD_ARCHIVE_UNAR`. Nix supplies pinned store
paths; Arch depends on `7zip` and `unar`; AppImage carries its native tools and
libraries. macOS packages require `brew install ffmpeg sevenzip unar`.

## Wire format, version 1

Launch the executable with `--stdio`. Every JSON message is prefixed by its
length as a little-endian `u32`. Frames must not exceed 32 MiB. The extension
sends `Reply::Hello` with a protocol version and capability list. The host checks
both the version and the capability needed for its request.

One process handles one request and then exits. Requests are:

| Request | Capability | Result |
| --- | --- | --- |
| List | index | Entries, with an explicit partial flag |
| Read | member_read | Exact member ordinal, streamed Data chunks |
| Create | create | New private archive, using Options |
| Extract | extract | New private output directory |
| Rebuild | zip_edit | ZIP header changes and staged additions |
| Inspect | inspect | Bounded JSON Inspection in Data: detected format, editable, encrypted |
| RebuildEdited | zip_replace | ZIP changes, additions and exact-ordinal file replacements |
| ConvertToZip | zip_convert | Convert a readable archive into a private ZIP |
| Test | test | Decode and validate every regular member |

Replies may include Progress(byte count), Done, or Error(message). Data(length)
is immediately followed by exactly that many raw bytes, at most 256 KiB; no
other frame may interleave those bytes. Stdout carries protocol only. A provider
must not log passwords. Options' Rust Debug implementation redacts passwords;
requests deliberately do not implement Debug.

`Entry` positions are stable header identities for that source version. Paths
are relative member names, validated independently by FOLD. Source changes
invalidate cached identities and materialized files. Never normalize traversal
names into different names, follow member links, or write directly to a user's
final destination. FOLD kills the process group on cancellation or deadlines;
providers must keep native children in that group and bounded output buffers.

## Engines and licensing

ZIP/TAR/7z/RAR readers and ZIP/TAR/gzip/Zstandard/7z writers live in the Rust
extension. Native [7-Zip](https://7-zip.org/) expands format, encryption and
volume support; [XAD/unar](https://theunarchiver.com/command-line) supplies legacy
formats. Those executables remain replaceable and run out of process. Their
licenses and source links accompany distributions in NOTICE and LICENSES.

Linux protocol and controller tests exercise round trips, passwords, volumes,
header identity, nested browsing, selective copies, traversal rejection,
source changes and ZIP save/discard. macOS packaging and physical SSH drag
interaction require testing on their respective machines.

## Editing and publication

These requests are additive version-1 capabilities. Older providers can still
browse; missing inspection/edit capabilities leave the chain read-only. The host
never assumes editability from the filename. `Replacement` identifies an exact
source header ordinal and a private regular-file snapshot. Editing is supported
only when every container is ZIP; the host rebuilds nested containers from the
leaves and atomically publishes the outermost output after checking fingerprints.
Save As converts other readable formats into ZIP. Expansion, count, free-space,
cancellation and destination checks apply to conversion too.

The host journals completed editor saves and structural edits in private cache
storage without recording passwords. Recovery validates source fingerprints and
snapshot containment; conflicting sources retain a visible recovery warning.
The extension never launches editors or draws UI. Native codec fallback discards
failed extraction attempts before exposing bytes, and resolves the same unique
member identity between engines. AppleDouble entries retain their original names.
