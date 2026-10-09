# STAR/FOLD documentation

The [README](../README.md) is the short version. These pages are the long one,
grouped by what you are trying to do.

## Getting started

- [Installing](installing.md): Nix, AppImage, macOS, and from source.
- [The stack](the-stack.md): Fold and Commander views, Places, marks and the
  operations queue.
- [Keys and mouse](keys-and-mouse.md): every binding and gesture, per module.
- [Drag and drop](drag-and-drop.md): native file gestures, destinations, and SSH transfers.
- [Configuration](configuration.md): every setting, the themes, and where the
  files live.
- [If something is wrong](troubleshooting.md): the first things to check, and
  where the logs are.

## Going further

- [Intelligent previews, file icons and archive actions](previews-and-archives.md)
- [Archive formats](archive-formats.md): verified fixtures, codec limits and editing support.
- [Terminal graphics](terminal-graphics.md): native presentation, SSH, media and pointer controls.

- [Themes](themes.md): the format, your own themes, and the `[fold]` roles.
- [On the command line](cli.md): `starfold list`, which prints a directory
  with no window involved.

## About the project

- [File manager roadmap](file-manager-roadmap.md): staged feature work and
  acceptance checks from the comparison with terminal and desktop managers.
- [Baseline validation](baseline-validation.md): a disposable fixture and
  manual checks for the current application.
- [Creation validation](create-validation.md): milestone 1 checks for new files
  and directories in Fold and Commander.
- [Content search validation](content-search-validation.md): milestone 2b checks
  nested text matches, skips, cancellation, and result actions.
- [Recovery validation](recovery-validation.md): Trash restore and safe session undo,
  including platform limits and disposable checks.
- [Status](status.md): what is done, what is in progress, and what is not
  started.
- [Commander visual target](graphical/treatments/README.md): interactive redesign study and captures.
- [Contributing](../CONTRIBUTING.md), [Security](../SECURITY.md),
  [Changelog](../CHANGELOG.md).

## Documented in the source instead

Some things go stale the moment they move away from the code, so they stayed
there:

- `src/fold/mod.rs` explains why nothing in the core knows the terminal
  exists.
- `src/session.rs` explains what `session.toml` keeps and why.
- `AGENTS.md` keeps working notes for whoever is changing the code next.

[HTML preview extension developer guide](../documentation/index.html) — Video, NVIM, protocol, themes and provider hierarchy.

[Presentation modes and shortcuts](../documentation/presentation.html) describes live switching, F8/F9 and styled editor cells.

For coordinated unreleased KIT/AMP changes, run `scripts/build-local.sh`. It uses the sibling KIT and `staramp-ssh-audio` checkouts, builds through Nix, restores the git-sourced lockfiles and installs the matching private AMP helper beside FOLD. Published distributions use pinned revisions; update those pins after the shared changes are released.
