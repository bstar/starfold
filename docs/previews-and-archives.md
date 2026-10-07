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
or `vi` when neither is set, inside the Preview panel. Save and quit with that
editor's own keys. When it exits, STAR/FOLD refreshes the listing and preview.
Copy, Move, Delete,
Compress and Extract apply to the marked set if the clicked entry is marked;
otherwise they apply only to that entry, preserving unrelated marks.

Compression opens a destination archive field. The suffix selects ZIP, tar,
tar.gz, tar.zst or 7z; Tab cycles them. Extraction opens a destination field,
defaulting to a folder named after the archive. For multiple archives, choose a
parent folder and each archive gets its own named folder and operation row.
Enter **queues** the operation. Run it from OPERATIONS using the normal controls.

| Format | Preview / extract | Create |
| --- | --- | --- |
| ZIP | Yes | Yes |
| tar, tar.gz, tar.zst | Yes | Yes |
| tar.xz, tar.bz2 | Yes | No |
| 7z | Yes | Yes |
| RAR | Yes | No |

Extraction currently accepts unencrypted, single-volume archives containing
regular files and directories. Links, device nodes, unsafe member paths and
7z deletion entries are rejected. Limits are 100,000 members and 64 GiB of
extracted data per operation. Compression rejects links/special files too.
ZIP creation uses Deflate, gzip and Zstandard use fast compression, and 7z uses
the backend's default LZMA2 settings. The initial implementation normalizes
permissions; it is not a backup tool for exact permissions, xattrs or timestamps.

Output is staged privately beside the destination. Existing destinations use
the queue's conflict controls; Overwrite replaces the entire destination folder
for extraction rather than merging. Cancellation or failure removes staged output.
The source archive is retained. If publishing and restoring an overwritten
output both fail, the error identifies the recovery directory retaining the old
output. Required RARLAB notices are in `LICENSES/UnRAR.txt`.

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
