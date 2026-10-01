# Native graphical feature alignment

The experimental native frontend shares `App` key dispatch, captured action
requests, worker commands and validation with the terminal frontend. This is a
coverage checklist for the presentation, not a claim that every physical device
or terminal emulator has been tested.

| Area | Native presentation | Verification |
| --- | --- | --- |
| Fold navigation | Retained folded levels, expanded active frame, back/forward, home/root, cursor/scroll restoration | Shared stack/session tests; actual large-directory navigation |
| Commander | Independent stacks, marks, filters and sort; only browser splits | Shared per-pane regression tests; actual native copy between panes, including two marked paths and a long Unicode filename |
| File rows | Fixed height, ellipsis, full-name tooltip, cached paths and shared colors | Actual 100,000-entry window and scrolling; row-cache tests |
| Tabs | Padded filled tabs, close/new, context actions, rename, duplicate, reorder, reopen | Shared tab tests; actual native menu, rename and session persistence |
| File actions | Open, Preview, Edit, Mark, Copy, Move, Rename, Copy path, New, Archive, Delete, Tabs, Recovery | Shared captured-target/controller tests; native menu renders same action tree |
| Menus | Nested submenus, keyboard/mouse selection, hover, separators, disabled/dangerous actions | Shared popup tests; native menu screenshot |
| Typed dialogs | Destination, Help, Failure, Confirm, TrashWarning, Create, Recovery, Rename, Search, Sort, Conflict, ConflictRename | Exhaustive overlay match; shared dialog validation tests; actual custom conflict rename |
| Places | Browse/search, bookmark form, rename, details, remove, refresh, unmount, empty trash | Exhaustive native views and shared PlaceAction dispatch; existing regression tests; actual bookmark/details navigation |
| Preview | Directory, text, document/PDF pages, image scale modes, symlink, empty/error; below browser | Existing preview-worker tests; native image fixture; conversion off UI thread |
| Editor | Existing PTY, resize, key/paste input, terminal colors/styles, lifecycle/error handling | Shared editor tests; KIT wide/combining-character surface tests; actual native PTY input/lifecycle |
| Audio player | Existing process control, styled terminal surface, key/pointer input and update polling | Existing player/process tests; same application action dispatch |
| Operations | Immediate queue, planning/space checks, captured paths, locks, full progress, rate/ETA, cancel, conditional resume/remove | Existing operation/lock tests; actual 256 MiB SHA-256 copy |
| Failures | Persistent details, complete native clipboard report, retry and administrator authorization | Shared failure/retry tests; authorization subprocess is cancellable and off UI thread |
| Recovery | Trash/Undo browse, refresh, restore destination and editable collision name | Shared recovery/restore tests and native controlled input |
| Clipboard and drops | Native clipboard; captured local pane/folder/external paths; explicit source scroll freeze and target scroll | Native clipboard/drop controller tests; existing local/SSH drop PTY tests |
| Themes/input | Alt+T/shared theme colors; Unicode selection/graphemes, paste, IME composition, password masking | KIT input tests; default STAR/AMP, STAR/CORD and STAR/WIRE build checks |
| Terminal backend | Existing plain/enhanced terminal workflow, graphics fallback and remote transport | Default regression suite, terminal PTY tests and earlier authenticated Kitty/SSH check |

## Performance checks

The release fixture used 30 tabs and 100,000 actual files on warm tmpfs. It passed
100 cursor moves, viewport scrolling, native tab actions, rename, persistence and
clean exit. Input p95 was 0.738 ms and element construction p95 0.295 ms; settled
idle CPU was 0.30% of one core. These exclude GPU presentation latency. Row/path
caches invalidate on listing generation, filter, marks and relative-time updates.

## Remaining validation

- Physical portable-drive trash/unmount/failure cases and successful native
  administrator authorization with real credentials.
- Hands-on macOS display/input, assistive technology and proprietary GPU drivers.
- Distant SSH and tmux graphics performance; native GPUI windows do not travel
  through ordinary SSH. The terminal frontend is the remote path.
- End-to-end GPU frame timing and slow-mount first paint.

The regular launcher, session and stable app dependencies remain separate. Use
`starfold-visual --backend desktop` and restart existing experimental windows to
load a rebuilt executable.
