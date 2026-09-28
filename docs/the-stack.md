# The stack

STAR/FOLD is built around one idea: the directories you drill through do not
replace each other, they stack up.

## Commander and Places

Press `v` to switch between Fold and Commander. Commander puts two directory
listings side by side, with one active pane. `Tab` or `Shift+Tab` changes the
active pane; clicking a pane focuses it. Each pane remembers its own directory,
cursor and filter. The preview follows the active pane, and the operations
queue is shared between views.

In Commander, `space` marks entries in the active directory. `y` (or `yy`)
yanks those entries, or the highlighted entry when nothing is marked. Switch
to the destination pane and press `p` to queue a copy there. `m` still queues
a move to the opposite pane. The paste destination is captured when `p` is
pressed, so later navigation cannot change it. Operations start immediately.
Marks in Fold keep their existing across-directory
behavior; Commander marks belong to each pane and clear when that pane changes
directory.

Press `c` with the Stack focused or click `actions` in its heading to open the file actions menu.
Choose Copy current path to put the open directory's full path on the system clipboard. Over SSH,
STAR/FOLD sends it to the terminal's clipboard with OSC 52, so the local computer receives it
when the terminal allows clipboard writes. This works with a file highlighted and in an empty
directory. Choose New file or New directory, type one name,
and press Enter. Creation runs immediately through the
IO worker; it does not enter the operations queue. An existing file, directory,
or symlink is never overwritten. A successful creation refreshes the listing
and selects the new item, so Enter can open a new directory at once. In
Commander, creation belongs to the active pane, even if you switch panes while
the worker runs. The menu offers both creation actions in an empty directory.

F3 or Ctrl+F opens recursive filename search from the active directory. It
does not follow directory symlinks and shows relative paths so duplicate names
remain distinct. Search results support preview, marks and the same file
actions as ordinary rows. Escape cancels a running scan; press it again to
close the results, or press `h` to leave immediately. Closing restores the
directory and cursor you had before searching. Hidden paths follow the
`hidden` setting, which reruns an open search when changed. A capped scan is
labelled as limited rather than complete.

Press `b` for Places and search by name or path. It groups your bookmarks,
mounted devices, mounted network locations, Home and Root. `B` saves the
current directory as a bookmark with an editable name; bookmarks can be
renamed or removed in Places. A saved bookmark stays in the list when its
device or network share is disconnected, and navigation reports the failure.
Mounted local drives show capacity and device in the list. On wider terminals,
the selected place has a detail pane with its mount path, filesystem and
available space. Linux also shows label, UUID, model, serial and connection
type when the device reports them. Drive information is gathered during the
mount scan, not while drawing the window. Press `F3` for a full-width detail
view, including on smaller terminals.
Select a local drive in Places and press `F6`, or click its `[unmount]` button,
to unmount it after confirmation.
While it runs, Places shows an animated status and OPERATIONS shows a temporary
unmount activity row. That row is separate from queued file operations.
STAR/FOLD moves panes that were viewing the drive to its mount-point parent.
Unmounting can fail while another app is using the drive; the error appears in
Places. Places does not mount drives, eject physical media or log in to network
shares.

STAR/FOLD remembers the last view, both Commander directories and the Fold
directory. An explicit directory on the command line opens in the active view.
If a saved location is gone on the next launch, that location opens at the
launch working directory and a warning appears.

## Levels fold behind you

Entering a directory pushes a level onto the column. Every level you have
already drilled through is still there, folded to a single line:

```
▸ ~                                                                 12 items
▸ projects/                                                          9 items
─ starfold/ ── 14 files · 3 dirs · 84.2 MB ─────────────────────────────────
  src/                              dir         -    Sep 12 14:02
  Cargo.toml                        toml    1.2 KB   Sep 14 09:27
  …
```

Only the level you are actually in is expanded into a listing. The ones above
it are a breadcrumb trail with a count, not a memory you have to reconstruct —
you can see, at a glance, that you got here through `~` and `projects/`.

Moving into a directory (`l`, `enter`, a double-click) pushes a new level and
folds the one you were on. Moving up (`h`, `left`, `backspace`) returns to the
filesystem parent in both views. When that parent is already the previous Fold
level, its cursor is restored. From a launch directory or a Places jump, `h`
keeps going through actual parents to `/`; the new parent highlights the
directory you just left.

## Jumping without losing anything

`alt+up` jumps straight to the parent level without popping anything: the
level you were in stays on the stack, folded, ready to jump straight back into
with `alt+down`. Clicking a folded crumb does the same thing for any level,
not just the parent.

This is different from popping. Popping throws away everything below the
level you return to — if you back out of `starfold/src/` with `h`, `src/` is
gone from the stack and going back into it starts fresh. Jumping with `alt+up`
keeps `src/` exactly as you left it, cursor and all, because you told the
stack where you wanted to look, not that you were done with where you were.

## Marks follow you

`space` marks the entry under the cursor; the mark is not local to a
directory. Mark a file three levels down, jump back to the top of the stack,
mark another file somewhere else entirely, and both are still marked. The
status line says how many: `2 marked · 14.2 MB`.
The focused row has a `›` pointer. A marked row keeps its filled `●` beside
that pointer when the cursor lands on it; an unmarked row shows `○`.

This is what makes `y` and `p` work across directories: gather marked files,
press `y` to save their paths, then land where you want them and press `p`.
The saved paths can be pasted more than once. `m` queues a move of the marked
files to the current directory. Once an operation has run, the marks it
consumed are gone: the copies are not marked, and neither are the originals,
so `dd` pressed next targets what is under the cursor and not what you just
copied.

## The operations queue

Pressing `p` or `m` adds an entry to OPERATIONS and starts it when the worker is free:

```
COPY 2 items → ~/Archive                                 copying 42.0%
```

OPERATIONS expands while work is active without changing keyboard focus.
`ctrl+x` stops the active operation and pauses later work, including new
requests. `X` from anywhere or `enter`/`r` in OPERATIONS resumes it. `esc`
there clears work that has not started. A running copy shows live byte progress
to a tenth of a percent, with partial bar cells, even while the file view is
idle (`COPYING ███████▊░░ 78.0%`). Stopping it removes any unfinished
destination file. Files already completed stay completed.
For a remote drag and drop, the same operation shows a receiving bar and live
byte count while the sender's total is unknown. It then continues into the
measured placement phase without starting a second operation or resetting the
progress bar.

`dd` asks for confirmation, then starts deletion of the marked entries or the
entry under the cursor when nothing is marked. A single `d` only waits for the
second key. The confirmation names the captured target; accepting it does not
starts the deletion without a second Run step.

## What `esc` does

`esc` never quits. Tried in order: it closes an open overlay (help, a
confirmation, a rename, a conflict prompt) first; then it clears the `/`
filter if one is active; then, on the OPERATIONS module, it clears whatever
in the queue has not started running yet.

## Sorting, hiding and filtering

`s` or the `sort` heading opens a picker for the sort key, direction, and
directories first. The direction row toggles between Low → High and High → Low
for size, A → Z and Z → A for names and extensions, or newest and oldest for
dates. Press `S` to toggle direction directly, including while the picker is
open. With directories first enabled, direction changes the order within each
group while directories continue to lead.

- **name** compares case-insensitively and treats a run of digits as a
  number, so `file2.txt` sorts before `file10.txt` rather than after it.
- **size** is smallest first; reversed, largest first.
- **time / modified**, **created**, and **accessed** are newest first by
  default and oldest first when reversed. Unavailable dates stay last.
- **ext** groups by extension (lowercased, without the dot), then falls back
  to the name within a group.
- **type** groups directories, files, symlinks, and other entries, then sorts
  names within each group.

`.` shows or hides dotfiles; hidden by default. `/` filters the active
level's rows through a fuzzy, ranked match (best match first, case
insensitive) over what is currently visible — narrowing an already-sorted
view rather than searching the whole directory. An empty query is the
identity: every row, in the order the sort already gave them.

## When a level cannot be shown

If a directory cannot be read at all — most often a permission wall — the
active level's body shows the reason (`permission denied`) instead of a
listing, and the rest of the stack still draws normally: you can still see
how you got there and back out. If the directory a frame was open on has
been removed entirely, the fold pops back to the nearest level that still
exists and says so. A listing bigger than `[ui] max_entries` (50000 by
default) is read up to that limit and the level's rule line says
`(truncated)`.

## The preview

`i` shows or folds the PREVIEW panel; it is intended to follow the cursor as
it moves, building a fresh preview for whatever row the cursor lands on and
discarding one that arrives for a file the cursor has already left. What it
shows depends on what the cursor is on:

- **A directory** previews as a summary — how many files, how many
  directories, and their total size — from a budgeted walk that stops at
  `[preview] dir_budget` entries (20000 by default) or 64 levels deep and
  says so if it had to give up early. It never follows a symlink out of the
  directory.
- **Text** shows up to `[preview] max_bytes` (256 KiB by default) and
  `[preview] max_lines` (400) of the file, with a `(truncated)` marker when
  either limit cut it short.
- **An image** decodes to real pixels and is downscaled when the original is
  wider or taller than `[preview] max_image_dimension` (4096). Very large
  originals use `vipsthumbnail` when it is installed; source and output sizes
  remain bounded. How it is actually drawn — a terminal graphics protocol or
  half-blocks — is the `[ui] graphics` setting; see
  [Installing](installing.md#terminals-and-whether-you-get-a-picture-in-the-preview).
- **Anything else that is not text** — a binary blob without a recognised
  image format — shows as a hexdump, sixteen bytes to a row.
- **A symlink** shows what it points at, and whether the target is there.
- **An empty file** and **nothing selected** both preview as empty; anything
  that could not be read or decoded shows the reason instead.

## Operations: plan, conflicts and running

An operation starts by planning — every source directory is expanded, its bytes totalled, and
every name that would collide with something already at the destination is
found — and only then, if nothing needs asking, copies, moves, deletes or
renames anything. Planning never follows a symlink: a symlinked directory
among your marks is queued as one item, recreated as a link at the other end,
not walked into. Copying or moving a directory into itself, or into its own
descendant, is refused outright rather than attempted.

A drop joins the same serial queue as keyboard and menu operations. It waits
for earlier work, and for Resume if the queue is paused. It uses the same
planning and conflict handling as an ordinary copy or move.

When a plan finds a name already at the destination, the queue stops and
asks — the OPERATIONS module shows the conflicts, and there are three
answers: **overwrite** (replace what is there), **skip** (leave it alone), or
**rename** (keep both, giving the incoming one a new name like `a (1).txt`).
The answer applies to the whole queued operation, not file by file. Renaming
a name that only differs by case, on a filesystem where that does not count
as a different name, is not treated as a conflict at all — and works, going
through a hidden temporary name for the moment in between, rather than
silently doing nothing the way a direct rename to the same name would.

A delete goes to the trash where the platform has one; `[ops] trash` decides
whether that is even attempted (`auto`, `always` or `never`), and where there
is no trash to reach, STAR/FOLD asks before deleting permanently unless
`[ops] confirm_delete` is turned off. `dd` always asks once; a confirmed
delete starts as soon as the worker is free.

If an operation cannot finish everything — one file among fourteen was
unreadable, say — the status line says so plainly: `1 of 14 failed`. The
fourteen there counts everything the operation actually attempted (done,
skipped by a conflict policy, and failed); the rest still went through, so a
partial failure is not the same as the whole operation having failed.

Moving within the same filesystem is a rename and keeps the file's identity;
moving across a filesystem boundary falls back to copying the file and then
removing the source, because the kernel has no cheaper way to move data
between two devices.
