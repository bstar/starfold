# Configuration

Everything STAR/FOLD keeps lives under one directory. Settings live in
`config.toml`, named Places bookmarks in `bookmarks.toml`, and the current view
and directories in `session.toml`. This page covers those files, the themes and
the directory layout.

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
| `[ui] right_click` | accept physical right-click mouse actions. Default `false`; Ctrl+click works without this setting |
| `[ui] show_hidden` | show dotfiles by default. Default `false`. `.` toggles it for the running session |
| `[ui] sort` | the starting sort key: `name`, `size`, `time` or `ext`. Default `"name"`. `time` sorts newest first; `ext` groups by extension, then by name |
| `[ui] sort_reverse` | reverse the starting sort. Default `false` |
| `[ui] dirs_first` | list directories before files under any sort. Default `true` |
| `[ui] fold_rows` | how many folded parent levels the stack shows before squeezing them into one crumb row. Default `6` |
| `[ui] preview_rows` | rows the preview panel gets when it is open and unfocused, never more than half of what the terminal has to spare above the floor. Default `10` |
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
| `[preview] timeout_ms` | smart-preview request deadline, clamped to 100–30000 ms. Default `2000` |
| `[preview] cache_bytes` | in-memory preview cache budget, clamped to 1–128 MiB. Default `33554432` (32 MiB) |
| `[preview] pdf_page_bytes` | extracted text limit per PDF page, clamped to 1–256 KiB. Default `262144` |
| `[preview] max_bytes` | how much of a text file is read for the preview. Default `262144` (256 KiB) |
| `[preview] max_lines` | how many lines of that text are drawn. Default `400` |
| `[preview] max_image_dimension` | a picture wider or taller than this, in pixels, is refused rather than decoded. Default `4096` |
| `[preview] dir_budget` | the most entries inspected for a directory tree or marked-directory size. Trees also stop at 400 displayed entries, four levels and 200 ms. Default `20000` |
| `[preview] image_scale` | how a picture smaller than the panel is grown: `1x`, `pixels` or `smooth`. Default `1x` |

`image_scale` is the one of those `z` changes while STAR/FOLD is running, and
what `z` chooses is written back here. `1x` draws a small picture at its own
pixel size in the middle of the panel; `pixels` grows it by whole numbers
only, nearest-neighbour, so a screenshot or a piece of pixel art keeps its
hard edges; `smooth` grows it to fill the fitted rectangle at any factor, with
a Catmull-Rom filter. A picture already bigger than the panel is scaled down
to fit in all three, and the meta row under it says which mode is in force and
by how much it grew — `png · 320 × 200 · pixels ×3`. Where the terminal never
said how big a cell is, there is nothing to count pixels with and all three
fall back to fitting the panel.

### Opening a file elsewhere

| Key | Does |
| --- | --- |
| `[open] command` | a whitespace-split argv for opening a file externally, with the path appended as its own argument. Default `""`, which is the desktop's own opener — `open` on macOS, `xdg-open` elsewhere |

`command` is one string, split on whitespace into a program and its flags,
rather than a shell line: a file name with a space or a semicolon in it is
somebody else's file name, and it reaches the program as one argument without
ever passing through a shell to be reinterpreted.

The file menu's Edit action runs a terminal editor inside Preview. It uses
`$VISUAL`, then `$EDITOR`, then `vi`. Quoted arguments in those environment
variables are parsed, and the file path is passed as one separate argument.
`[open] command` still controls external Open only.

## Theming

`theme = "system"` follows the desktop: STAR/FOLD reads Stylix's
`~/.config/stylix/palette.json` (or `palette.yaml`) first, so whatever base16
scheme the rest of the desktop is set to, it matches it, and falls back to
COSMIC's own settings when there is no Stylix file to read.

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
*derived* from that palette — one rule per role, read from a base16 slot where
one applies and held to a contrast floor afterwards — and a theme file that
does state a role has the last word. [Themes](themes.md) has the format for
stating one.

| Role | Is | Derived from |
| --- | --- | --- |
| `dir_fg` | a directory's name, and a symlink to one | base16 `base0D` (blue) |
| `symlink_fg` | a symlink's name | base16 `base0C` (cyan) |
| `exec_fg` | a file with the executable bit set | base16 `base0B` (green) |
| `hidden_fg` | a dotfile, when hidden files are shown | the theme's dim colour |
| `marked_fg` | the mark glyph and a marked row's text | base16 `base0A` (yellow) |
| `marked_bg` | a marked row's background tint | the panel background mixed 20% toward the accent |
| `crumb_fg` | a folded level's line | the theme's dim colour |
| `crumb_active_fg` | the active level's rule | the theme's header colour |
| `size_fg` | the size column | the theme's row-metadata colour |
| `time_fg` | the modified-time column | the theme's row-metadata colour |
| `kind_fg` | the kind/extension column | the theme's dim colour |
| `progress_fg` | the running operation's bar | the theme's accent colour |
| `progress_bg` | the bar's own track | the panel background mixed 15% toward the foreground |
| `conflict_fg` | a queued operation waiting on a conflict answer | base16 `base09` (orange) |
| `error_fg` | a listing or an operation that failed | base16 `base08` (red) |

Every built-in is checked against WCAG AA in the test suite. Anything carrying
words clears 4.5:1 against what it is drawn on; a role that carries no letters
of its own — the progress bar's fill against its track, a marked row's tint
against the plain panel — clears 3:1.

## Where it keeps things

Everything lives under one directory, so a whole STAR/FOLD setup can be backed
up, moved, or deleted by moving one folder:

```
~/.local/starfold/
├── config.toml        your settings (0644)
├── bookmarks.toml     named Places directories (0600)
├── session.toml       Fold and Commander locations, view, hidden and sort state (0600)
├── themes/            your own themes
└── cache/             the log. Safe to delete
    └── starfold.log
```

The directories are mode 0700: bookmarks and session state name locations you
have browsed, and the log can too at debug level.

- `$STARFOLD_DIR` relocates all of it; `$STARFOLD_CONFIG_DIR` moves the config
  and `bookmarks.toml`, which lives beside it.
- `session.toml` and `bookmarks.toml` are 0600 because the directories they name
  are mildly private even though nothing in them is a secret. Both are written
  atomically. An older session with only `last_dir` still opens in Fold.
- The log never carries more than a path at any level above `debug`.

Bookmarks are managed with `B` and the Places picker (`b`). Their file is
also readable TOML:

```toml
[[bookmark]]
name = "Projects"
path = "/home/me/projects"
```

Each bookmark needs a name and an absolute directory path. Unavailable paths
stay saved. Invalid files produce a visible error and are not overwritten;
after repairing the file, use `F5` in Places to retry loading it.

## Audio in Preview

```toml
[preview]
audio_player = "auto" # auto, staramp, or external
audio_buttons = "auto" # auto (pictures when supported), or text
```

`auto` uses a separately installed STAR/AMP with embedding protocol v1 on
`PATH`. A nonempty `[open] command` keeps taking precedence. `staramp` prefers
the embedded player even with a custom opener; `external` always uses the
existing opener. Missing or older STAR/AMP versions fall back to the opener.
Decoder or audio-device failures appear in Preview, without launching another
player automatically; `Shift+O` opens the track externally while Preview has
focus (`o` still opens externally in the file panes).

Enter or double-click starts playback; selecting a file is silent. Once the
embedded player accepts the track, Preview takes focus so Space pauses and
`x` or Esc stops immediately. If you navigate away before it starts, it leaves
your focus alone. The queue
is a snapshot of supported audio files in the active, filtered and sorted
directory listing. Browsing either view does not change that queue. Opening
another track replaces it. The player stays in Preview until stopped, closed
with `i`, or STAR/FOLD exits. It follows STAR/FOLD's theme and uses compact
controls in short terminals.

With Preview focused, `o` toggles graphical/text transport buttons and saves
the preference in STAR/FOLD's configuration. Pictures use the separately
installed STAR/AMP's transport faces and require a graphics-capable terminal
and a STAR/AMP version advertising `transport_images`. Older embedding helpers
and text-only terminals keep the text controls. This setting is independent
of `[ui] graphics` for file previews and does not change STAR/AMP's configuration.

With Preview focused, `w` / `Shift+W` cycles STAR/AMP's visualizers and `d`
cycles seek-bar styles. Clicking a visualizer cycles forward; Ctrl+clicking
the visualizer or seek bar cycles seek styles, and the wheel over a visualizer
cycles forward/back. These controls require a STAR/AMP version advertising
`player_styles`; older helpers show an update hint for the keys and keep their
existing mouse behavior. STAR/AMP saves the choices in its separate
`embed/starfold.toml` profile under its configuration directory (overridable
with `STARAMP_CONFIG_DIR`). This does not alter the standalone player style.
STAR/FOLD shows profile load/save warnings as transient notes while playback
continues.

STAR/AMP owns and renders the embedded player using its existing player UI,
including graphical button artwork and mouse hit-testing. STAR/FOLD is only
the host: it supplies panel size/theme, forwards controls, and displays the
returned cells and images. No STAR/AMP player widgets are duplicated here.

This is an independent player: it does not join another STAR/AMP session or
write its library, history, standalone settings, scrobbles, or Discord
presence. Only an explicit visualizer or seek-style change writes the separate
embedded style profile.
Local files and files on mounted devices/network shares are supported; direct
network URLs, playlists, and CUE expansion are not. Paths that cannot be
represented as UTF-8 are explicitly rejected by embedding, not renamed or
lossily converted; the external opener remains available.
