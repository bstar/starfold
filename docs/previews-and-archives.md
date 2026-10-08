# Previews, file icons and archive actions

Fold and Commander show Unicode icons for folders, links, executables, audio,
video, images, PDFs, archives, source code, fonts and other files. Icons use
filename/MIME hints without opening every directory entry. Preview dispatch also
checks signatures. A Nerd Font is not required.

## Intelligent previews

- Directories: scrollable trees with folders first, branch guides and file-type
  icons. Symlinks are shown without expanding them. Discovery stops at four
  levels, 400 entries (or the smaller `max_lines` / `dir_budget`), or 200 ms
  (or the smaller `timeout_ms`), with checks between filesystem calls.
  Counts describe the entries shown; depth limits, unreadable folders and partial
  listings are indicated. Hidden entries are included.
- Audio: common tags plus duration, bitrate, sample rate and channel count through
  Lofty. Extra textual tags are included; cover artwork is not decoded here.
- Video: the bundled video extension supplies posters and playback actions,
  using the existing KIT media services and AMP transport controls. Local and
  SSH playback retain their audio, subtitle and fullscreen controls. Container
  metadata remains available when the extension cannot open a file.
- PDF: the bundled Hayro extension renders pages on demand and retains the parsed
  document. Focus Preview for page navigation, zoom, fit and pan controls;
  `t` switches to extracted text. Its raster cache respects the preview byte
  budget and retains at most 24 entries. See [extension controls and configuration](preview-extensions.md).
- Archives: bounded member lists with sizes; no extraction just to preview.
- Other binaries: type, size, modification time and permissions instead of hex.
  Existing text, image, directory and symlink previews remain available.

PDF text extraction does not perform OCR or request passwords. Image-only pages
can be rendered; protected PDFs require an external viewer. Corrupt inputs and
unsupported features have explicit messages, and render failures switch to text.
Complex PDFs may have imperfect rendering, text ordering or font decoding.
A request that exceeds its time or memory limit leaves an explanation; browsing
stays responsive.

The PDF and video parsers ship as bundled executable extensions. Other preview
parsers remain part of Starfold. No `pdftotext`, `ffprobe`, `7z`, or `unrar`
installation is required. Preview children are isolated from the UI, bounded by a
request deadline (two seconds by default) and a 768 MiB resource ceiling. These
are process/resource boundaries, not an operating-system filesystem sandbox.
The cache is memory-only and defaults to 32 MiB. See [configuration](configuration.md)
and the [public extension protocol](preview-extensions.md).

## Right-click actions

Press `c` with the Stack focused or click `actions` in its heading for a centered modal. Ctrl+click a listing
entry, or use Menu / Shift+F10. Physical right-click also opens the menu when
`[ui] right_click = true`. Up/Down or j/k selects an
action; Enter activates it. Escape and clicking outside dismiss the menu. Opening
the menu does not toggle marks. Mark/unmark remains in the menu and on Space.
Ctrl+click empty listing space, or open the menu in an empty directory, for
New file and New directory. Both create in the active directory immediately;
they never overwrite an existing name.

Open, Preview, Edit and Rename apply to the clicked entry. Edit appears for
regular text-like files and links to them; it runs `$VISUAL`, then `$EDITOR`,
or `vi` when neither is set, inside the Preview panel. Configuration files such
as `.cfg`, `.conf` and `.ini` are editable text. Open/Enter uses the desktop
association; choose Edit to use the terminal editor. Save and quit with that
editor's own keys. When it exits, STAR/FOLD refreshes the listing and preview.
Copy, Move, Delete,
Compress and Extract apply to the marked set if the clicked entry is marked;
otherwise they apply only to that entry, preserving unrelated marks.

## Archive workspace extension

Archive support is included in the ordinary `starfold` build and installation.
Its extension provider runs in a supervised child of the same executable. FOLD owns navigation, selection,
previews, staging and OPERATIONS. See [the archive extension contract](archive-extension.md).

Enter an archive to browse it as a folder in either pane.
While browsing inside an archive, select a member to use the ordinary text,
image, document or media preview. Only that member is materialized into private
scratch storage; the whole archive is not extracted. Unrecognized binary members
show a hex and readable-byte view of up to 4 KiB, bounded further by the preview
limits. This does not decode proprietary game resource formats.

Archive Preview also offers **Browse**; click it or press Enter while Preview
has focus to open the archive in the active pane. Space marks members there.
Enter directories and nested archives normally; Back returns through their boundaries. Space marks
individual members; yank/paste or Copy extracts the marked members into a
filesystem pane. Directory copies preserve their structure, including empty
archive directories. Selecting a member uses the ordinary preview providers;
only that member is materialized in private scratch storage. Filename and
bounded text search also work within the archive. Remote sessions run the
extension on the host; selected files can be streamed out through Kitty drag.

### ZIP changes

In a writable ZIP, including ZIPs nested inside ZIPs, copy/paste or drop into the archive stages additions or replacements.
SSH imports receive into private staging first. Move into a ZIP is disabled:
save the copied members before removing their originals.
Rename and Delete stage changes too. Existing members use the normal conflict
choices. The original ZIP remains unchanged until **Save ZIP changes** in the
Archive menu, or **Ctrl+S**. The footer reports pending changes. **Discard ZIP
changes** restores the original view. Quit, tab close and Back out of the archive
ask before abandoning pending changes; saving must finish before continuing.
Save failures keep both the original and the pending edits.

Select a text/configuration member to load the configured Preview editor.
Editor saves update a private working copy and stage an exact header replacement;
they do not modify the original ZIP. The Preview Neovim provider retains its
normal configuration and dirty-buffer Save/Discard/Cancel controls. Other terminal
editors use `$VISUAL`, then `$EDITOR`, then `vi`. An untouched editor follows
selection; after input it stays pinned until you exit it. Tabs retain these live
sessions. Exit generic editors before saving/discarding their archive.

Nested ZIP editing requires every containing archive to be writable ZIP. **Save
archive** rebuilds the affected chain and publishes it atomically at the outermost
ZIP, including staged parent and sibling changes. Cancellation, codec failure,
insufficient space and a changed source leave the original and staged edits intact.
**Save archive as…** publishes the chosen ZIP subtree to a separate ZIP. Read-only
formats can be converted to ZIP with this action; their original remains unchanged.
Members in a read-only chain show bounded text without starting an editor. Extract
those members before editing them.

Saved staging survives application restart in private `archive-edits` cache
folders. Reopening an unchanged archive restores its pending edits. If the source
changed, the pane retains a warning with the recovery folder rather than applying
stale header identities. Unsaved editor buffers cannot be recovered after a crash.

Encrypted ZIP replacements preserve AES strength. Legacy ZipCrypto replacements
are upgraded to AES-256; added members in an encrypted ZIP require the session
password and remain encrypted. Unchanged members retain their compressed bytes.
**Test archive** checks member decoding; **Unlock archive…** supplies a password
for this session. Passwords are masked and sent through private stdin, never
command arguments or operation logs. Password support depends on the codec;
encrypted legacy XAD archives currently report a clear unsupported error.

### Compression

Compress opens a destination and options dialog. The suffix selects ZIP, tar,
tar.gz, tar.zst or 7z; Tab cycles them. Use Up/Down to choose fields, Space to
change an option, and Enter or the Create button to start. Presets are Store,
Fast, Balanced and Maximum. ZIP/7z additionally support passwords and split
volumes. The Mac metadata option excludes `.DS_Store`, `._*` and `__MACOSX`.
Operations start immediately, report progress, and support cancellation.

| Formats | Browse / selective copy | Create | Staged editing |
| --- | --- | --- | --- |
| ZIP / CBZ | Yes | ZIP | Writable ZIP chains |
| tar, tar.gz, tar.zst | Yes | Yes | — |
| tar.xz, tar.bz2, tar.lz4 | Yes | — | — |
| 7z, RAR / CBR | Yes | 7z | — |
| CAB, ISO, DMG, XAR, AR, DEB, RPM, disk/container formats | Native codec fallback | — | — |
| SIT, SITX, SEA, other legacy formats | XAD/unar fallback | — | — |
| gzip, bzip2, XZ, Zstandard and supported numbered volumes | Native codec fallback | — | — |

See the [format verification matrix](archive-formats.md) for tested fixtures and
remaining coverage gaps.

Native coverage depends on the archive's actual structure and available engines;
recognizing a suffix does not guarantee every format variant. Codec failures
remain visible in the pane or OPERATIONS. Missing volumes must be supplied beside
the first volume. Duplicate ZIP and 7z names have distinct member identities for
selective reads; duplicate destination names require separate copies/renaming.
ZIP's writer rejects duplicate names when saving a rebuild and retains the original.

Limits are 100,000 indexed members, eight nested archive levels, and 64 GiB of
expanded data per operation. Links, special files, hostile member paths and 7z
anti-items are rejected. Indexing has a 60-second deadline; cancellation kills
and reaps the extension and its native child processes. Preview scratch files
are cached under a byte/count budget and removed when the process exits.

Output is staged privately beside its destination. Existing destinations use
OPERATIONS conflict controls; whole-archive extraction replaces the destination
folder when Overwrite is chosen. ZIP Save preserves unmodified compressed
members and the archive comment, checks source identity again before publication,
and publishes the result atomically. Compression normalizes permissions and is
not a backup tool for preserving xattrs, ownership or every timestamp.

## Performance checks

After a release build, run:

```sh
python3 scripts/bench-previews.py target/release/starfold
```

The script generates simple 10-, 200- and 1000-page PDFs, verifies extracted text,
and measures helper startup plus first-page rendering, then retained-session
page loading. With FFmpeg installed it also generates tagged audio/video fixtures.
Installed `pdftotext` and `ffprobe` are comparison tools only. Measurements use warm
filesystem caches and synthetic documents; they are not a guarantee for arbitrary
fonts, scans or complex layouts. The parent supervisor additionally polls responses
at up to 10 ms intervals; UI frame scheduling contributes to visible latency.
