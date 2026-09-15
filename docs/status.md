# Status

Nothing is built yet. This page exists from the first commit so that it never
has to be written retroactively, and so the list below can be read against
what is actually in the tree rather than against what the README says the
project is for.

The honest one-line summary: the repository has its scaffold — packaging, CI,
documentation — and no file manager behind it yet.

## Milestone 1

| Area | State |
| --- | --- |
| The fold stack | planned |
| Persistent selection | planned |
| The operations queue (copy, move, delete, rename) | planned |
| Trash integration | planned |
| The preview panel (text, image, directory summary, hexdump) | planned |
| `starfold list` | planned |
| The window: one column, stack / preview / operations | planned |
| Themes, via STAR/KIT | planned |
| Packaging: Nix flake, PKGBUILD, `.deb`, AppImage, portable tarball, CI | in progress |

## Not planned for milestone 1

Tabs, forked stacks, the action palette, archives and git status in the
listing. The data model leaves room for tabs and for more than one stack from
day one, but none of it is wired up yet.

## What is still only a plan, not an observation

Everything on this page. There is no live behaviour to check yet, so there is
nothing here that has been watched running against a real filesystem. Once a
milestone lands, this section is where a claim that turned out to be wrong, or
a rough edge nobody smoothed over yet, gets recorded honestly rather than left
for someone to discover on their own.
