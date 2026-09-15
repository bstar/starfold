# Status

This page is read against what is actually in the tree, area by area, rather
than against what the README says the project is for. "Built and tested
headlessly" means the code exists and its own tests pass with no terminal
attached — `fold::state::apply`'s own tests, the fixture-driven tests under
`src/fold/`, the UI's frame and layout tests driven by the fixture core in
`src/ui/fake.rs` — not that anyone has watched it running in a real terminal
yet. See [Not yet verified](#not-yet-verified) for the difference that still
matters.

## Milestone 1

| Area | State |
| --- | --- |
| The fold stack: push, pop, jump, forward | built, tested headlessly |
| Persistent selection (marks, byte totals) | built, tested headlessly |
| The operations queue: copy, move, delete, rename, conflicts, cancellation | built, tested headlessly |
| Trash integration | built (wraps the `trash` crate); the move-to-the-real-trash round trip is gated behind `STARFOLD_TEST_TRASH=1` and has not been watched happening on a real Linux or macOS trash |
| The preview panel: text, image, directory summary, hexdump, symlink | built, tested headlessly, including decoding a real PNG; nobody has yet seen an image drawn in a terminal |
| `starfold list` | built, tested, including a headless smoke test |
| The column: layout, panels, status row, keymap, overlays, theme | built, unit tested (layout tiling by proptest, keymap invariants, theme legibility against all sixteen built-ins); the event loop that wires them into a running window (`src/ui/app.rs`) is being finished as this page is written, so the window itself has not run yet |
| Themes, via STAR/KIT: the `[fold]` roles | built, contrast-checked against all sixteen built-in themes |
| Packaging: Nix flake, PKGBUILD, `.deb`, AppImage, portable tarball, CI | scaffolded; not exercised as part of this milestone's own work |

## Not planned for milestone 1

Tabs, forked stacks, the action palette, archives, and git status in the
listing. Chunked copy progress for a single very large file — today's copy
reports progress file by file, with nothing between "started" and "done" for
one file's own bytes. Watching a directory through the operating system
(`notify` or equivalent) rather than the once-a-second mtime poll `watch.rs`
uses today. The data model leaves room for tabs and for more than one stack
from day one, but none of it is wired up to anything yet.

## Not yet verified

Nothing in this milestone has been run in a real terminal against a real
filesystem yet. Once it has, this is where a claim that turned out to be
wrong, or a rough edge nobody smoothed over, gets recorded honestly rather
than left for someone to discover on their own. What is still only inferred
from the code and its headless tests, not observed:

- The window itself: drawing, the event loop, resizing, focus.
- A picture actually appearing in kitty, iTerm2, WezTerm, Ghostty or as
  half-blocks — the preview's image decoding is tested, drawing it to a real
  terminal is not.
- The mtime poll noticing a change made outside STAR/FOLD while it is open.
- Deleting to the trash on a real macOS and a real Linux machine.
- A move across a real device boundary (a second drive, a USB stick, a
  network mount).
- The 60×21 floor, and the row just below it, on a real terminal rather than
  a fixture-sized buffer.
