#!/usr/bin/env bash
# Build the supported package for this host. Linux AppImages use an isolated
# Bullseye target directory so they never replace the local Nix-built binary.
set -euo pipefail
cd "$(dirname "$0")/.."
target=${1:-$(if [ "$(uname -s)" = Darwin ]; then echo macos; else echo appimage; fi)}
case "$target" in
  nix) exec nix build .#default --print-build-logs ;;
  macos) exec scripts/dist/macos.sh ;;
  arch) ;;
  appimage) ;;
  *) echo "usage: $0 [nix|appimage|macos|arch]" >&2; exit 2 ;;
esac
CONTAINER=${CONTAINER:-$(command -v docker || command -v podman || true)}
[ -n "$CONTAINER" ] || { echo "need docker or podman" >&2; exit 1; }
mkdir -p dist
if [ "$target" = arch ]; then
  exec "$CONTAINER" run --rm \
    -v "$PWD:/src:ro" -v "$PWD/dist:/out" \
    -e DIST_UID="$(id -u)" -e DIST_GID="$(id -g)" \
    archlinux:latest bash -c '
      set -euo pipefail
      pacman -Syu --needed --noconfirm base-devel rust git ffmpeg alsa-lib clang pkgconf
      useradd --create-home builder
      mkdir /home/builder/starfold
      tar -C /src -cf - Cargo.toml Cargo.lock flake.nix flake.lock src tests \
        testdata vendor packaging scripts docs .github README.md LICENSE NOTICE LICENSES \
        | tar -C /home/builder/starfold -xf -
      chown -R builder:builder /home/builder/starfold /out
      cleanup() { chown -R "$DIST_UID:$DIST_GID" /out; }
      trap cleanup EXIT
      cd /home/builder/starfold
      runuser -u builder -- env DIST_DIR=/out ./scripts/dist/arch.sh
    '
fi
"$CONTAINER" run --rm \
  -v "$PWD:/src" -w /src \
  -v "starfold-target-appimage-$(uname -m):/build/target" \
  -v starfold-cargo:/root/.cargo \
  -e CARGO_TARGET_DIR=/build/target -e CARGO_HOME=/root/.cargo \
  -e DIST_UID="$(id -u)" -e DIST_GID="$(id -g)" \
  debian:bullseye-slim bash -c '
    cleanup() { chown -R "$DIST_UID:$DIST_GID" /src/dist; }
    trap cleanup EXIT
    scripts/dist/appimage.sh
  '
