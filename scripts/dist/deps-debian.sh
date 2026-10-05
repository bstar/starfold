#!/usr/bin/env bash
# Build prerequisites inside a Debian or Ubuntu container.
#
# Native video previews use FFmpeg, ALSA and bindgen.
# Keep this list aligned with flake.nix and CI.
#
# Rust comes from rustup rather than apt: the MSRV is newer than anything
# bookworm or bullseye package, and the whole point of building in an old
# container is the old *glibc*, not an old toolchain.
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive
# Bullseye left LTS in August 2026 and its security suite was removed from the
# mirrors, while the image was built with those updates installed, so no live
# mirror can satisfy the versions already on it. Debian's snapshot service has
# both suites frozen at the last day of LTS. What this container is for is its
# glibc, so a frozen package set is right. A supported release is left alone.
if grep -q '^VERSION_CODENAME=bullseye' /etc/os-release; then
  printf 'deb http://snapshot.debian.org/archive/debian/20260830T000000Z bullseye main\ndeb http://snapshot.debian.org/archive/debian-security/20260830T000000Z bullseye-security main\n' > /etc/apt/sources.list
  printf 'Acquire::Check-Valid-Until "false";\nAcquire::Retries "5";\n' > /etc/apt/apt.conf.d/99bullseye-eol
fi
apt-get update -qq
apt-get install -y -qq --no-install-recommends \
  ca-certificates curl file git xz-utils \
  build-essential pkg-config clang libclang-dev libasound2-dev \
  libavcodec-dev libavformat-dev libavutil-dev libavfilter-dev \
  libswscale-dev libswresample-dev

if ! command -v cargo >/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path
fi
. "$HOME/.cargo/env"

export LIBCLANG_PATH=$(dirname "$(find /usr/lib -name 'libclang.so*' -o -name 'libclang-*.so*' | head -1)")
