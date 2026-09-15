# The stack

STAR/FOLD is built around one idea: the directories you drill through do not
replace each other, they stack up.

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
folds the one you were on. Moving back out (`h`, `backspace`) pops the level
you are on and returns you to the one before it, with the cursor exactly where
it was.

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
want them, and press one key.

## The queue is the confirmation

Marking files and pressing `y` or `m` does not touch the filesystem. It adds
an entry to the OPERATIONS module:

```
COPY 2 items → ~/Archive                                    queued · enter run
```

Nothing is copied, moved, deleted or renamed until you run the queue —
`enter` on the module, or `X` from anywhere. `esc` clears it instead. This is
the confirmation dialog, except it is a list you can inspect, add to and take
things off, rather than a single yes-or-no you have to get right the first
time. A running operation shows a progress bar in the status row and can be
stopped with `ctrl+c` without leaving anything half-done: what has already
completed stays done, what has not started stays untouched.
