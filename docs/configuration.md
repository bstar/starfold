# Configuration

Everything STAR/FOLD keeps lives under one directory, and everything it can be
told lives in one file there. This page covers the file, the themes and the
directory layout.

## The config file

`~/.local/starfold/config.toml`, written with comments on first run.

Every table has defaults, and a file that omits one gets the whole table's
defaults. Editing one key never means writing the other six.

### The window

| Key | Does |
| --- | --- |
| `[ui] theme` | a theme id, or `"system"` to follow the desktop. Default `"catppuccin-mocha"`. See [Theming](#theming) |
| `[ui] graphics` | how the preview draws a picture: `auto` asks the terminal, `kitty` insists on the kitty protocol, `blocks` (also spelled `halfblocks`) draws two pixels to a cell in any terminal at all, `off` draws no picture and leaves a name instead. Default `auto` |
| `[ui] padding_x` / `padding_y` | blank columns and rows around the whole layout, for a terminal whose window has none. Default `0` |
| `[ui] show_hidden` | show dotfiles by default. Default `false`. `.` toggles it for the running session |
| `[ui] sort` | the starting sort key: `name`, `size`, `modified` or `kind`. Default `"name"` |
| `[ui] sort_reverse` | reverse the starting sort. Default `false` |
| `[ui] dirs_first` | list directories before files under any sort. Default `true` |
| `[ui] fold_rows` | how many folded parent levels the stack shows before squeezing them into one crumb row. Default `6` |
| `[ui] preview_rows` | rows the preview panel gets when it is open and unfocused. Default `10` |
| `[ui] ops_rows` | rows the operations panel may grow to while it is focused. Default `6` |
| `[ui] max_entries` | the most entries a single listing will hold before it reports itself truncated. Default `50000` |

The window needs 60 columns by 21 rows. Below that STAR/FOLD says so rather
than drawing something it cannot draw honestly.

### Operations

| Key | Does |
| --- | --- |
| `[ops] trash` | where a delete goes: `"auto"` (default) uses the platform's trash if one is available and asks before a permanent delete if not; `"always"` insists on the trash and refuses to delete without one; `"never"` always deletes permanently, with the same confirmation |
| `[ops] confirm_delete` | ask before a permanent delete. Default `true` |
| `[ops] conflicts` | the default answer when a copy or move would overwrite something: `"ask"` (default) stops the queue and asks, `"skip"` leaves the existing file alone, `"overwrite"` replaces it, `"rename"` keeps both under a new name |
| `[ops] preserve_times` | keep a copied file's modification time rather than stamping it with the time of the copy. Default `true` |

### Preview

| Key | Does |
| --- | --- |
| `[preview] max_bytes` | how much of a text file is read for the preview. Default `262144` (256 KiB) |
| `[preview] max_lines` | how many lines of that text are drawn. Default `400` |
| `[preview] max_image_dimension` | a picture wider or taller than this, in pixels, is refused rather than decoded. Default `4096` |
| `[preview] dir_budget` | the most entries a directory summary will walk before it stops counting and says so. Default `20000` |

### Opening a file elsewhere

| Key | Does |
| --- | --- |
| `[open] command` | argv for opening a file externally, never a shell line. The path is appended. Default `[]`, which is the desktop's own opener — `open` on macOS, `xdg-open` elsewhere |

`command` is a list because a file name with a space or a semicolon in it is
somebody else's file name, and it reaches a program without ever reaching a
shell.

## Theming

`theme = "system"` follows the desktop. STAR/FOLD reads Stylix's
`~/.config/stylix/palette.json`, so whatever base16 scheme the rest of the
desktop is set to, it matches it.

Sixteen themes ship built in, and `t` and `T` cycle them live:

`winamp-classic` · `cosmic` · `catppuccin-mocha` · `catppuccin-latte` ·
`gruvbox-dark` · `nord` · `tokyo-night` · `dracula` · `rose-pine` ·
`everforest` · `solarized-dark` · `one-dark` · `kanagawa` · `ayu-dark` ·
`matte-black` · `terminal`

They are the same sixteen files STAR/AMP and STAR/CORD use, because all three
programs take their theme engine from
[STAR/KIT](https://github.com/bstar/starkit). A theme set in one looks like
the theme set in another.

Your own themes go in `~/.local/starfold/themes/` as `<id>.toml`, and a file
there wins over a built-in of the same name. [Themes](themes.md) has the
format.

### The `[fold]` roles

None of the sixteen shared files says anything about a file listing, and none
of them should have to: a theme is a palette. So the `[fold]` table is
*derived* from that palette — one rule per role, run over sixteen schemes and
held to a contrast floor — and a theme file that does state a role has the
last word.

| Role | Is |
| --- | --- |
| `dir_fg` | a directory's name |
| `symlink_fg` | a symlink's name |
| `exec_fg` | a file with an executable bit set |
| `hidden_fg` | a dotfile, when shown |
| `marked_fg` | a marked entry, and the mark itself |
| `crumb_fg` | a folded level's name in the breadcrumb trail |
| `size_fg` | the size column |
| `time_fg` | the modified-time column |
| `progress_fg` | the operations progress bar |
| `conflict_fg` | a row the queue found a conflict on |
| `error_fg` | a listing or an operation that failed |

Every built-in is checked against WCAG AA in the test suite. Anything carrying
words clears 4.5:1 against what it is drawn on; a mark that carries no letters
clears 3:1.

## Where it keeps things

Everything lives under one directory, so a whole STAR/FOLD setup can be backed
up, moved, or deleted by moving one folder:

```
~/.local/starfold/
├── config.toml        your settings (0644)
├── session.toml       last directory, hidden and sort state (0600)
├── themes/            your own themes
└── cache/             the log. Safe to delete
    └── starfold.log
```

The directories are mode 0700, and not out of taste: `session.toml` names
every directory you have been browsing, and the log can too at debug level.

- `$STARFOLD_DIR` relocates all of it; `$STARFOLD_CONFIG_DIR` moves just the
  config.
- `session.toml` is 0600 because the directories it names are mildly private
  even though nothing in it is a secret.
- The log never carries more than a path at any level above `debug`.
