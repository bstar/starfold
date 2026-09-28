# Status

As of 2026-09-27, STAR/FOLD is a working terminal file manager with Fold and
Commander views. [The file manager roadmap](file-manager-roadmap.md) tracks the
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
| Packaging | Nix, AppImage, and Apple Silicon macOS release routes are configured. Linux release builds are part of milestone acceptance; platform release checks remain separate. |

## Work in progress

Milestone 0's automated Linux checks and release build passed on 2026-09-25;
its [interactive checklist](baseline-validation.md) is ready for desktop
validation. Milestone 1 adds file and directory creation; its
[interactive checklist](create-validation.md) is ready for review. Milestone 2a
adds recursive filename search; its [interactive checklist](search-validation.md)
is ready for review. Milestone 2b adds content search; its
[interactive checklist](content-search-validation.md) is ready for review.
A follow-up yank/paste and current-path clipboard change
has passed automated checks and a Linux release build, with desktop clipboard
and interactive paste validation still pending. Recovery, bulk rename, shell handoff, durable tabs, and other
desktop file actions follow in the roadmap's stated order.

## Known boundaries

- `/` filters one directory; F3/Ctrl+F searches descendants by filename or
  file contents, switched with Tab in the search prompt.
- There is no bulk rename, Trash browser or restore action, shell picker output,
  or multi-tab session yet.
- Copies report byte progress within a file and can be stopped between
  chunks. A stopped copy removes its incomplete destination file.
- Places discovers mounted network locations; it does not establish network
  connections, mount drives, or physically eject them.
- STAR/FOLD's isolated PTY tests do not prove graphics, Trash, or removable
  devices work in every desktop terminal or on both supported platforms.
