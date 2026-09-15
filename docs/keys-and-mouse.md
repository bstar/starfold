# Keys and the mouse

Every key STAR/FOLD knows, in the order the `?` overlay prints them. This
file is generated from the table in `src/ui/keymap.rs`, and a test fails if
the two disagree.

A key is offered to the focused module first and to the global table second,
so a binding under a module heading works while that module has focus and
the global ones work from everywhere. While the `/` filter has focus it takes
raw keys, because a letter typed into it is a letter, not a command; every
`alt+…` falls through it, and `esc` and `enter` are always the way out.

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
| `h/left/bs`    | back one level |
| `alt+up`       | jump to the parent |
| `alt+down`     | jump back down |
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
| `enter`        | run it |
| `x/delete`     | drop one |
| `esc`          | clear the queue |

## view

_everywhere_

| key | what it does |
|---|---|
| `i`            | show, fold preview |
| `.`            | hidden files |
| `s`            | next sort key |
| `S`            | reverse the sort |
| `/`            | filter this level |

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

## the mouse

| where | gesture | what it does |
|---|---|---|
| stack    | click                 | move the cursor |
| stack    | double-click          | open it |
| stack    | right-click           | mark it |
| stack    | click a crumb         | jump there |
| stack    | wheel                 | scroll three rows |
| modules  | click a fold          | open it |
| modules  | click a word          | what it says |
| status   | click the bar         | open operations |
