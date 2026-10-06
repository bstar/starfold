# Release updates

STAR/FOLD uses STAR/KIT's optional `update` feature to check GitHub's latest
published stable release. Standalone Apple Silicon macOS archives and Linux
AppImages check in the background every six hours while the browser/client is
running. A verified update is staged privately and installed at the next
browser/client launch, before terminal setup. Playback and file operations are
not interrupted by downloads.

```sh
starfold update check       # Check now; no executable is changed
starfold update status      # Policy, installation support, staged version/errors
starfold update install     # Download and verify now; activate next launch
starfold update enable      # Automatic downloads and next-launch installation
starfold update notify      # Check only; availability appears in update status
starfold update disable     # Stop checks and discard any pending update
starfold update rollback    # Restore previous executable; disable updates
```

Automatic is the default for supported standalone release installations. Source
checkouts, Nix store executables, system/Arch packages, Homebrew/MacPorts and app
bundles are not overwritten. Use their normal Git/build or package-manager update
method. `check` can still inspect releases; `status` explains why self-update is
unavailable. Existing command symlinks remain intact: updates replace the resolved
standalone executable, not the command symlink. No elevation is requested.

Downloads use HTTPS, exact repository/platform/build-flavor asset names and the
release's `SHA256SUMS`. Download sizes, metadata and archive extraction are bounded.
Archives with links, traversal paths or duplicate executables are rejected. The
verified executable must run `--version` and identify itself as the expected app
and release before staging. Activation rechecks the staged executable's hash,
saves the prior executable, and publishes the new one by rename. A process lock
prevents two instances from installing together. A failed check/download leaves
the installed executable alone; failures appear in `update status`.

State lives under STAR/FOLD's cache in `updates/terminal` or `updates/graphical`.
The settings are independent of browsing sessions, and `STARFOLD_DIR` relocates
both. `list`, preview/archive helpers, attachment relays, help and version queries do
not start update workers; the persistent graphical browsing host can check releases. The SSH presentation client and server installation remain independent;
a client download never mutates the remote host or restarts its persistent session.
A running persistent host continues using its loaded version until that session
is restarted; updates do not stop ongoing playback or file operations.

## Graphical releases

The graphical client selects only `starfold-graphical-<version>-...` assets and
never falls back to an ordinary terminal release. Graphical release builds embed
the exact private STAR/AMP helper pinned in `flake.lock`. The helper is extracted
lazily into a private, content-addressed cache, so new client versions use their
matching helper while older running clients retain theirs. Updating the executable
(or the entire Linux AppImage) therefore updates both together by one rename.
The standalone STAR/AMP command is independent.

The release workflow builds both terminal and graphical variants. Existing
published releases predate graphical assets and updater support; users need a
release containing this implementation before future releases update themselves.
Experimental source checkouts still use Git and Nix builds. A graphical macOS
archive also contains the `starfold-graphical` launcher; a graphical AppImage
opens the graphical interface by default while still accepting `update`, `list`
and the internal worker commands.

## Release publishing

The existing release workflow generates `SHA256SUMS` over the exact uploaded
macOS archives and Linux AppImages. Tags produce draft releases; publishing a
draft makes it visible to clients. Branch pushes, workflow artifacts, drafts,
prereleases and versions equal to or older than the installed version do not
trigger updates. This implementation does not publish a new release itself.

## Other fleet applications

STAR/KIT owns repository requests, version/platform/flavor selection, checksum
verification, extraction, process checks, staging, locking, activation and rollback.
Each app supplies `update::Application`, its cache path, a writable standalone
target, its CLI/UI and the startup exec. Apps opt into the `update` Cargo feature;
applications that do not use updates acquire none of its network/archive support.
