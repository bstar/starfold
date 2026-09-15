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

## "terminal too small"

Below 60 columns or 21 rows there is not enough room to draw the column, so
the window says so rather than drawing something unreadable. Above the floor,
extra rows go first to the stack's listing, then to the preview panel up to
`[ui] preview_rows`, then to the operations panel while it is focused.

## An operation seems stuck

The status row shows a progress bar while the queue is running
(`COPYING ████████░░ 78%`); if it has not moved in a while the file it is on
is most likely large, or the destination is a slow mount. `ctrl+c` stops the
running operation: what has already completed stays done, and nothing after
it is touched.

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
starfold -v                                    # debug level
STARFOLD_LOG=starfold::fold::ops=trace starfold # a full filter
```

`STARFOLD_LOG` takes `tracing`'s `EnvFilter` syntax and overrides `-v`
entirely.
