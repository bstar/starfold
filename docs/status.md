# Status

As of 2026-10-09, STAR/FOLD is a working file manager with Fold and Commander
views, native Rust graphics in supported Kitty terminals, and a cell presentation
elsewhere. [The file manager roadmap](file-manager-roadmap.md) tracks the
remaining daily-work features and gives each one an acceptance check. The
[milestone 0 baseline](baseline-validation.md) records how to test the current
behavior without touching an ordinary STAR/FOLD session.
The [milestone 1 checklist](create-validation.md) covers file and directory
creation in both views. The [milestone 2a checklist](search-validation.md)
covers recursive filename search. The [milestone 2b checklist](content-search-validation.md)
covers text-content search.

| Area | Current state |
| --- | --- |
| Fold stack, navigation, persistent marks, sort, hidden files, and current-directory filter | Implemented; core tests, UI fixture tests, and isolated PTY checks cover navigation and drawing. |
| Startup and session restore | Saved directories are checked and listed on background readers. The window opens with a `reading…` state while a slow drive responds; another directory can be opened meanwhile. Missing saved directories still fall back to the launch directory. Automated tests cover delayed reads, navigation during a delayed read, and fallback. Desktop timing on a sleeping physical drive needs a hands-on check. |
| File and directory creation | Implemented with immediate worker dispatch, no overwrite, listing refresh, and cursor selection; automated Fold and Commander checks cover the main path. Permission errors need a manual check in an unwritable directory. |
| Recursive filename search | Implemented with bounded worker traversal, cancellation, hidden-path control, result actions, and identity checks before queued actions run. Automated tests cover nested matches, symlink loops, UI actions, and stale replacements. Large-tree cancellation and unreadable paths need hands-on checks. |
| Text-content search | Implemented in the same results view. It scans regular UTF-8 files up to 1 MiB each and 64 MiB total, skips binary and oversized files visibly, shows a matching line, and retains real paths and identity checks for actions. Automated checks cover matches, skips, limits, cancellation, UI frames, and queued copies. Desktop validation remains. |
| Copy, move, trash/delete, rename, conflicts, cancellation, and the inspectable operations queue | Operations start automatically, Stop pauses later work, and file copies can be stopped within a file. Automated tests cover these paths; real Trash, cross-device moves, and macOS copy performance need platform checks. |
| Commander and Places | Implemented. Two panes, mounted-location discovery, searchable bookmarks, drive details, and local-drive unmount have tests. Linux PTY checks covered browsing, Places, and session restore. Physical devices and network mounts need hands-on checks. |
| Previews, file icons, and archive actions | Implemented. Text, images, directories, audio/video tags, PDF pages, archive inspection, compression, and extraction have automated coverage. Terminal graphics and complex real-world files need broader manual checks. |
| Native drag and drop, embedded STAR/AMP | Implemented with PTY or process tests on Linux. Desktop terminal combinations and macOS embedding need hands-on checks. |
| Graphical presentation | The terminal renderer is on main, with shared navigation, pixel layout, pointer feedback, independent window sessions, image zoom, PDF reading, movie playback and AMP-owned player skins. See [terminal graphics](terminal-graphics.md) for transport and platform verification. |
| Archive workspace | Archives open as browsable locations, including nested archives and selective member copies. ZIP chains support staged member editing and guarded publication; other readable formats can be saved as ZIP. The [format matrix](archive-formats.md) distinguishes checked fixtures from unverified formats. |
| Recovery | Worker-backed Trash picker, collision-safe restore, and session-only undo for successful moves/renames. Linux uses freedesktop Trash metadata; macOS records STAR/FOLD deletions in a private journal. Automated tests cover restart, collisions, nested changes, cancellation, staged copying, and queue integration. Real macOS Trash and physical cross-device validation remain. |
| Workspace tabs | Implemented with independent Fold/Commander contexts, sorting, filtering, marks, preview context, and session restoration. Core, snapshots, and PTY checks cover isolation and restart. |
| Packaging | Nix, x86_64/ARM64 AppImages, native Arch packages, and Apple Silicon macOS archives passed the v0.0.2 release workflow, including 15 distribution smoke checks, both FUSE tests, and Arch installation. |

## Work in progress

The [Commander visual target](graphical/treatments/README.md) is an interactive,
buildless design study with bundled screenshots and fonts. Its equal panes,
larger filenames, compact inspection and transfer hierarchy are a review target;
the application does not yet implement that treatment. The older separate GPUI
frontend remains on `experiment/graphical-presentation`; the active renderer uses
STAR/KIT's `tiny-skia` and `cosmic-text` inside the terminal.

Milestone 0's automated Linux checks and release build passed on 2026-09-25;
its [interactive checklist](baseline-validation.md) is ready for desktop
validation. Milestone 1 adds file and directory creation; its
[interactive checklist](create-validation.md) is ready for review. Milestone 2a
adds recursive filename search; its [interactive checklist](search-validation.md)
is ready for review. Milestone 2b adds content search; its
[interactive checklist](content-search-validation.md) is ready for review.
A follow-up yank/paste and current-path clipboard change
has passed automated checks and a Linux release build, with desktop clipboard
and interactive paste validation still pending. Recovery now has an implementation and [manual checklist](recovery-validation.md). Bulk rename, shell handoff, and other desktop file actions remain next.

## Known boundaries

- `/` filters one directory; F3/Ctrl+F searches descendants by filename or
  file contents, switched with Tab in the search prompt.
- Bulk rename, shell picker output, configured custom actions, and permission editing remain unimplemented.
- macOS recovery lists items trashed by STAR/FOLD with this recovery implementation; use Finder for older or other applications' Trash items. Linux lists compatible freedesktop Trash items.
- Undo is limited to the last 100 recorded items in the current session. Overwrites, merges, permanent deletes, and incomplete operations are not undoable. Changed items are refused. Verification is bounded to 50,000 entries and 64 directory levels per item.
- Copies report byte progress within a file and can be stopped between
  chunks. A stopped copy removes its incomplete destination file.
- Places discovers mounted network locations; it does not establish network
  connections, mount drives, or physically eject them.
- STAR/FOLD's isolated PTY tests do not prove graphics, Trash, or removable
  devices work in every desktop terminal or on both supported platforms.
