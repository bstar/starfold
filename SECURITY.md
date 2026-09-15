# Security

## Reporting

Use GitHub's private vulnerability reporting on this repository
(Security → Report a vulnerability). Please do not open a public issue for
something exploitable.

I work on this in my spare time, so expect a first reply within a week rather
than a day.

## Threat model

Who the attacker is, at each place STAR/FOLD takes input from somewhere else.

| Boundary | In scope |
| --- | --- |
| **The directories and files it is pointed at** | Yes. A file name, a symlink, a directory nested deep enough to be a denial of service on its own, or a picture crafted to crash or mislead the preview — all of it is somebody else's content read by this program without being asked twice. |
| **The image decoder** | Yes. A preview decodes whatever bytes are behind a path with an image extension. A crafted PNG, JPEG, WebP or GIF is the most plausible hostile file this program will ever open. |
| **Path handling in copy, move, delete and rename** | Yes. A symlink that resolves outside the tree it appears to be part of, a rename that lands outside the destination directory, or a name built out of another name in a way that escapes the directory it was meant to stay under, are all vulnerabilities. |
| **The external opener** | Yes. `o` and a double-click hand a path to the configured program, or to the platform's own opener, as `argv`. Anything that turns that into a shell line, or that lets a file's own name inject an argument, is a vulnerability. |
| **Other local users** | Yes, on a shared machine. Everything STAR/FOLD writes for itself lives under `~/.local/starfold`, the directory is mode 0700, and the files in it — the config, the session file — are mode 0600. |
| **The person running STAR/FOLD** | No. The files you point it at, the directory you tell it to delete, and the opener you configure are your own authority. |
| **The build** | Yes. What the release workflow downloads is pinned and checksummed, and what CI runs is pinned to commits. |

## What is worth reporting

- **The image decoder's limits.** Dimensions are checked from the header
  before anything is decoded, so a small file declaring itself absurd on a
  side is refused rather than believed. A way past that limit, or a crash in
  the decoder on a crafted file, is worth a report.
- **Anything that escapes the directory an operation was meant to stay
  inside of.** Plan and execution never follow a symlink, depth is capped, and
  a conflict is decided by comparing `dev` and `ino` rather than by whether a
  path merely exists. A copy, move or rename that lands somewhere other than
  where it was told to is a vulnerability, not a bug report about the wrong
  file being overwritten.
- **The external opener treating a path as anything other than one argument.**
  `[open] command` is `argv`, never a shell line, and the path is appended as
  a single element. A file whose name causes something other than "run this
  program with this path" is worth a report.

Nothing here listens on a network port, and nothing in this tree talks to one.
Everything it reads comes off the local filesystem.

## What is not a vulnerability

- **Deleting what you told it to delete.** A queued delete goes to the trash
  where the platform has one; a permanent delete asks first. Either way, an
  operation you confirmed doing what it said it would do is not a security
  report.
- **A permission error.** STAR/FOLD runs as you, sees what you can see, and
  refuses what the filesystem refuses you. That is the operating system's
  boundary working, not this program's.
