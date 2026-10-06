#!/usr/bin/env bash
# Native Apple Silicon release package. Run on a macOS builder.
set -euo pipefail
cd "$(dirname "$0")/../.."
[ "$(uname -s)" = Darwin ] || { echo "macOS build requires a Mac" >&2; exit 1; }
ver=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
out=${DIST_DIR:-dist}
mkdir -p "$out"
brew list ffmpeg pkg-config llvm >/dev/null 2>&1 || brew install ffmpeg pkg-config llvm
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
prefix=starfold
build_args=()
if [ "${STARFOLD_GRAPHICAL:-0}" = 1 ]; then
  prefix=starfold-graphical
  . scripts/dist/build-amp-helper.sh
  build_args+=(--features terminal-graphics)
fi
cargo build --release --locked --target aarch64-apple-darwin "${build_args[@]}"
bin="${CARGO_TARGET_DIR:-target}/aarch64-apple-darwin/release/starfold"
"$bin" --version
if [ "$prefix" = starfold-graphical ]; then "$bin" --bundled-amp-version; fi
stage="$work/$prefix-$ver"
mkdir -p "$stage"
install -m755 "$bin" "$stage/starfold"
cp README.md LICENSE "$stage/"
if [ "$prefix" = starfold-graphical ]; then
  mkdir -p "$stage/LICENSES/STARAMP"
  cp "$amp_source/LICENSE" "$stage/LICENSES/STARAMP/"
  [ ! -f "$amp_source/NOTICE" ] || cp "$amp_source/NOTICE" "$stage/LICENSES/STARAMP/"
  [ ! -d "$amp_source/LICENSES" ] || cp -R "$amp_source/LICENSES/". "$stage/LICENSES/STARAMP/"
  cat > "$stage/starfold-graphical" <<'WRAPPER'
#!/bin/sh
launcher=$0
while [ -L "$launcher" ]; do
  directory=$(cd "$(dirname "$launcher")" && pwd)
  link=$(readlink "$launcher")
  case "$link" in /*) launcher=$link ;; *) launcher=$directory/$link ;; esac
done
exec "$(dirname "$launcher")/starfold" graphical "$@"
WRAPPER
  chmod +x "$stage/starfold-graphical"
fi
[ ! -f NOTICE ] || cp NOTICE "$stage/"
[ ! -d LICENSES ] || cp -R LICENSES "$stage/"
printf '%s\n' 'Requires Homebrew FFmpeg: brew install ffmpeg' > "$stage/INSTALL.txt"
tar -C "$work" -czf "$out/$prefix-$ver-aarch64-apple-darwin.tar.gz" "$prefix-$ver"
echo "wrote $out/$prefix-$ver-aarch64-apple-darwin.tar.gz"
