# Keys and the mouse

Every key STAR/FOLD knows, in the order the `?` overlay prints them. This
file is generated from the table in `src/ui/keymap.rs`, and a test fails if
the two disagree.

A key reaches its action through five layers, tried in order: an open
overlay takes every key while it is up; the `/` filter's text entry takes
typing next, because a letter typed into it is a letter, not a command;
`g` waiting for a second key (`gg`, `gh`, `gr`) comes next; the focused
module's own bindings are offered the key after that, so a binding under a
module heading works while that module has focus; and the global table
catches whatever nothing above wanted, which is what makes it work from
everywhere. While the filter has focus, every `alt+…` falls through
it and so does `?`, so help and the appearance and panel keys stay reachable
mid-search; `esc` and `enter` are always the way out.

The header highlights each word's keyboard letter. `n` also toggles hidden
files, `f` also opens the filter, and `e` also opens Places. The existing
`.`, `/`, and `b` keys still work. Preview's `c` and Operations' `r`/`c` work only
when that module has focus. The back arrow keeps its `h` navigation key.

In Commander view, `tab` and `shift+tab` switch file panes. `alt+1` focuses
the active pane; `alt+2` and `alt+3` reach preview and operations. `y/p` and
`m` queue files from the active pane into the opposite directory, using
current-directory marks or the highlighted file. `v` switches views, `b`
opens Places, and `B` bookmarks the current directory. In Places, type to
search, use arrows and `enter` to open a location, `F2` to rename a bookmark,
`delete` to remove one after confirmation, and `F5` to rescan mounts.

## navigation

_everywhere_

| key | what it does |
|---|---|
| `tab`          | next module |
| `shift+tab`    | previous module |
| `alt+1`        | the stack |
| `alt+2`        | the preview |
| `alt+3`        | operations |
| `up/k`         | up one |
| `down/j`       | down one |
| `shift+up/K`   | up ten |
| `shift+down/J` | down ten |
| `pgup`         | page up |
| `pgdn`         | page down |
| `home/gg`      | to the top |
| `end/G`        | to the bottom |
| `enter`        | open |
| `esc`          | cancel |

## stack

_in the stack_

| key | what it does |
|---|---|
| `l/right`      | into the directory |
| `h/left/bs`    | parent directory |
| `alt+up`       | jump to the parent |
| `alt+down`     | jump back down |
| `gh`           | the home directory |
| `gr`           | the root |
| `o`            | open externally |
| `r`            | rename |
| `F5/ctrl+r`    | reload |

## selection

_in the stack_

| key | what it does |
|---|---|
| `space`        | mark, move down |
| `a`            | mark all here |
| `A`            | invert the marks |
| `u`            | unmark everything |

## operations

_everywhere_

| key | what it does |
|---|---|
| `y/p`          | copy marked here |
| `m`            | move marked here |
| `d`            | delete marked |
| `X`            | run the queue |
| `ctrl+x`       | stop the running op |

## queue

_in operations_

| key | what it does |
|---|---|
| `enter/r`      | run it |
| `x/delete`     | drop one |
| `esc/c`        | clear the queue |

## preview

_in the preview_

| key | what it does |
|---|---|
| `c`            | close preview |

## view

_everywhere_

| key | what it does |
|---|---|
| `v`            | fold / commander |
| `b/e`          | places |
| `B`            | bookmark directory |
| `i`            | show, fold preview |
| `./n`          | hidden files |
| `s`            | next sort key |
| `S`            | reverse the sort |
| `f, /`         | filter this level |
| `z`            | picture scale |

## appearance

_everywhere_

| key | what it does |
|---|---|
| `t`            | next theme |
| `T`            | previous theme |

## application

_everywhere_

| key | what it does |
|---|---|
| `?/F1`         | this list |
| `ctrl+l`       | redraw the screen |
| `q/ctrl+c`     | quit |

## The mouse

While an embedded STAR/AMP player has Preview focus (`alt+2`), `space` or
`enter` pauses/resumes, `[` / `]` selects the previous/next track, left/right
seeks five seconds, and `+` / `-` changes volume. `x` or `esc` stops the player
and restores ordinary previews; `i` closes Preview and stops playback. `o`
toggles graphical/text transport buttons; `shift+o` opens the playing track
externally. On newer STAR/AMP, `w` / `shift+w` cycles visualizers forward/back
and `d` cycles seek-bar styles; these choices persist separately for STAR/FOLD.
These controls are scoped to Preview;
browser navigation, marking, global focus shortcuts, and quit keep their
normal meanings. The player's transport, seek, and volume controls are also
clickable. Left-click a visualizer to cycle it; right-click a visualizer or
seek bar to cycle the seek style; wheel over a visualizer cycles it.

| where | gesture | what it does |
|---|---|---|
| stack    | click                 | move the cursor |
| stack    | double-click          | open it |
| stack    | right-click           | file actions menu |
| stack    | click a crumb         | jump there |
| stack    | wheel                 | scroll three rows |
| modules  | click a fold          | open it |
| modules  | click a word          | what it says |
| status   | click the bar         | open operations |
