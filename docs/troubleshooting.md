# If something is wrong

The usual suspects, in the order people hit them.

## There is no picture in the preview, only blocks

The terminal has no graphics protocol, or the detection could not see through
ssh or a multiplexer. Half-blocks are the fallback and always work; they are
two rows of colour per cell rather than an image.

| Terminal | Pictures |
| --- | --- |
| kitty, Ghostty | kitty protocol |
| WezTerm, iTerm2 | yes |
| foot, xterm with sixel enabled | sixel |
| Alacritty, GNOME Terminal, Konsole | half-blocks |
| inside tmux | half-blocks |

Over ssh, or anywhere else the question goes unanswered, insist:

```toml
[ui]
graphics = "kitty"
```

## It says there is no trash

On a machine with no freedesktop trash implementation available — most often a
minimal Linux install with no `$topdir/.Trash-$uid` reachable on the mount a
file lives on — there is nowhere for a delete to go. STAR/FOLD falls back to
asking whether to delete permanently, and says why.

To choose deliberately:

```toml
[ops]
trash = "never"    # always delete permanently, with the same confirmation
# trash = "always" # refuse to delete at all without a trash available
```

## Deleting a file on another drive is slow, or lands somewhere unexpected

On Linux, the trash is not one place: a file on your home filesystem goes to
`$XDG_DATA_HOME/Trash` (typically `~/.local/share/Trash`), but a file on a
different mount — a second drive, a USB stick, a network share — goes to that
filesystem's own `$topdir/.Trash-$uid` instead, per the freedesktop
specification, so restoring it later does not mean copying it back across
devices. If that top-level directory cannot be created on the other mount —
read-only media, most often — the delete fails there rather than silently
using the home trash, and falls through to the same permanent-delete question
as no trash at all.

## A directory will not list, or a level reads "permission denied"

The level still draws — the crumbs above it, the rest of the column — with
the reason shown in place of a listing. It means the process cannot read that
directory's contents, which is a question for `ls -la` on the same path
outside STAR/FOLD, not something a setting here changes. A directory that
existed a moment ago and has since been removed entirely, rather than merely
walled off, is handled differently: the fold pops back to the nearest level
that still exists, with a note saying so.

## "terminal too small"

Below 60 columns or 21 rows there is not enough room to draw the column, so
the window says so rather than drawing something unreadable. Above the floor,
extra rows go first to the stack's listing, then to the preview panel up to
`[ui] preview_rows`, then to the operations panel while it is focused.

## An operation seems stuck

The status row shows a progress bar while the queue is running
(`COPYING ████████░░ 78%`); if it has not moved in a while the file it is on
is most likely large, or the destination is a slow mount. `ctrl+x` stops the
running operation: what has already completed stays done, the file currently
being copied finishes, and nothing after it is touched.

## Building on a Mac and `cargo` cannot fetch STAR/KIT

STAR/KIT is a git dependency, so a build needs `git` on `PATH`. On a Mac with
Xcode selected, `/usr/bin/git` is a shim that asks `xcrun` where the real
`git` is — and inside `nix develop`, that `xcrun` is Nix's own, which answers
"tool 'git' not found" rather than falling through to Xcode's. The Nix
devshell puts Nix's own `git` first on `PATH` for this reason, so building
through `nix develop -c cargo build` (with `CARGO_NET_GIT_FETCH_WITH_CLI=true`
set, as [Installing](installing.md#from-source) shows) works; a build outside
the devshell needs a `git` that actually resolves, which the Xcode
command-line tools provide once installed (`xcode-select --install`).

## Something is wrong and none of the above

```sh
starfold list ~/some/directory
```

Prints a listing with no window involved, so a defect that reproduces this way
is a defect with a much shorter report — it separates a directory-reading
problem from a drawing one. [The command line](cli.md) has the rest of the
flags.

## Where the logs are

`~/.local/starfold/cache/starfold.log`. Nothing goes to the terminal while the
window is open: it owns the alternate screen.

```sh
starfold -v                                     # debug level
STARFOLD_LOG=debug starfold                     # the same thing, spelled as a filter
STARFOLD_LOG=starfold::fold::ops=trace starfold  # a narrower filter
```

`STARFOLD_LOG` takes `tracing`'s `EnvFilter` syntax and overrides `-v`
entirely.
