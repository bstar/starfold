# The stack

STAR/FOLD is built around one idea: the directories you drill through do not
replace each other, they stack up.

## Commander and Places

Press `v` to switch between Fold and Commander. Commander puts two directory
listings side by side, with one active pane. `Tab` or `Shift+Tab` changes the
active pane; clicking a pane focuses it. Each pane remembers its own directory,
cursor and filter. The preview follows the active pane, and the operations
queue is shared between views.

In Commander, `space` marks entries in the active directory. `y` or `p` queues
those entries for copying to the opposite pane's directory; `m` queues a move.
With no marks, the highlighted entry is the source. The destination is
captured when the operation is queued, so later navigation cannot change it.
Run the queue with `X`. Marks in Fold keep their existing across-directory
behavior; Commander marks belong to each pane and clear when that pane changes
directory.

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

This is what makes `y` (copy) and `m` (move) make sense as "copy what I
marked, to here": you gather files from wherever they are, land wherever you
want them, and press one key. Once the operation has run, the marks it
consumed are gone: the copies are not marked, and neither are the originals,
so a `d` pressed next deletes what is under the cursor and not what you just
copied.

## The queue is the confirmation

Marking files and pressing `y` or `m` does not touch the filesystem. It adds
an entry to the OPERATIONS module:

```
COPY 2 items → ~/Archive                                    queued · enter run
```

Nothing is copied, moved, deleted or renamed until you run the queue —
`enter` on the module, or `X` from anywhere. `esc` on the module clears
whatever has not started yet; a running operation is left to finish, or is
stopped with `ctrl+x`. This is the confirmation dialog, except it is a list
you can inspect, add to and take things off, rather than a single yes-or-no
you have to get right the first time. A running operation shows a progress
bar in the status row (`COPYING ████████░░ 78%`) and `ctrl+x` stops it
without leaving anything half-done: what has already completed stays done,
whatever file was in flight finishes (cancellation is checked between files,
not partway through one), and nothing after it is touched.

`d` deletes the marked entries, or the entry under the cursor when nothing is
marked — the one queueing command with something to do when the selection is
empty.

## What `esc` does

`esc` never quits. Tried in order: it closes an open overlay (help, a
confirmation, a rename, a conflict prompt) first; then it clears the `/`
filter if one is active; then, on the OPERATIONS module, it clears whatever
in the queue has not started running yet.

## Sorting, hiding and filtering

`s` cycles the sort key — name, size, time, ext, back to name — and `S`
reverses it. Directories lead under every key, in either direction: reversing
the sort reverses the files among themselves, not whether a directory comes
before a file.

- **name** compares case-insensitively and treats a run of digits as a
  number, so `file2.txt` sorts before `file10.txt` rather than after it.
- **size** is smallest first; reversed, largest first.
- **time** is newest first *by default* — the one key whose forward
  direction is not "smallest value first", because nobody browsing by time
  wants to scroll past years of history to find what they touched five
  minutes ago. Reversed, it is oldest first.
- **ext** groups by extension (lowercased, without the dot), then falls back
  to the name within a group.

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
- **An image** decodes to real pixels, refusing to decode anything wider or
  taller than `[preview] max_image_dimension` (4096) rather than trusting a
  hostile header. How it is actually drawn — a terminal graphics protocol or
  half-blocks — is the `[ui] graphics` setting; see
  [Installing](installing.md#terminals-and-whether-you-get-a-picture-in-the-preview).
- **Anything else that is not text** — a binary blob without a recognised
  image format — shows as a hexdump, sixteen bytes to a row.
- **A symlink** shows what it points at, and whether the target is there.
- **An empty file** and **nothing selected** both preview as empty; anything
  that could not be read or decoded shows the reason instead.

## Operations: plan, conflicts and running

Queueing an operation only records what you asked for. Running it plans the
operation first — every source directory is expanded, its bytes totalled, and
every name that would collide with something already at the destination is
found — and only then, if nothing needs asking, copies, moves, deletes or
renames anything. Planning never follows a symlink: a symlinked directory
among your marks is queued as one item, recreated as a link at the other end,
not walked into. Copying or moving a directory into itself, or into its own
descendant, is refused outright rather than attempted.

A drop is the deliberate start gesture for a transfer: it enters OPERATIONS
and runs when the operations worker is free. Earlier keyboard-queued entries
do not start just because a drop did. A drop still uses the same planning and
conflict handling as an ordinary copy or move.

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
`[ops] confirm_delete` is turned off. Deleting is queued the same way copying
and moving are: nothing happens until the queue runs.

If an operation cannot finish everything — one file among fourteen was
unreadable, say — the status line says so plainly: `1 of 14 failed`. The
fourteen there counts everything the operation actually attempted (done,
skipped by a conflict policy, and failed); the rest still went through, so a
partial failure is not the same as the whole operation having failed.

Moving within the same filesystem is a rename and keeps the file's identity;
moving across a filesystem boundary falls back to copying the file and then
removing the source, because the kernel has no cheaper way to move data
between two devices.
