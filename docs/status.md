# Status

As of 2026-09-25, STAR/FOLD is a working terminal file manager with Fold and
Commander views. [The file manager roadmap](file-manager-roadmap.md) tracks the
remaining daily-work features and gives each one an acceptance check. The
[milestone 0 baseline](baseline-validation.md) records how to test the current
behavior without touching an ordinary STAR/FOLD session.
The [milestone 1 checklist](create-validation.md) covers file and directory
creation in both views. The [milestone 2a checklist](search-validation.md)
covers recursive filename search.

| Area | Current state |
| --- | --- |
| Fold stack, navigation, persistent marks, sort, hidden files, and current-directory filter | Implemented; core tests, UI fixture tests, and isolated PTY checks cover navigation and drawing. |
| File and directory creation | Implemented with immediate worker dispatch, no overwrite, listing refresh, and cursor selection; automated Fold and Commander checks cover the main path. Permission errors need a manual check in an unwritable directory. |
| Recursive filename search | Implemented with bounded worker traversal, cancellation, hidden-path control, result actions, and identity checks before queued actions run. Automated tests cover nested matches, symlink loops, UI actions, and stale replacements. Large-tree cancellation and unreadable paths need hands-on checks. |
| Copy, move, trash/delete, rename, conflicts, cancellation, and the inspectable operations queue | Implemented and tested with temporary trees. Real Trash and cross-device behavior need separate platform checks. |
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
is ready for review. Content search, recovery, bulk rename, shell handoff, durable tabs, and other
desktop file actions follow in the roadmap's stated order.

## Known boundaries

- `/` filters one directory; F3/Ctrl+F searches descendant filenames. File
  contents are not searchable yet.
- There is no bulk rename, Trash browser or restore action, shell picker output,
  or multi-tab session yet.
- `std::fs::copy` reports progress between files and checks cancellation
  between files, not partway through one large file.
- Places discovers mounted network locations; it does not establish network
  connections, mount drives, or physically eject them.
- STAR/FOLD's isolated PTY tests do not prove graphics, Trash, or removable
  devices work in every desktop terminal or on both supported platforms.
