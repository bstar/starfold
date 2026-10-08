#!/usr/bin/env bash
# Native Apple Silicon release package. Run on a macOS builder.
set -euo pipefail
cd "$(dirname "$0")/../.."
[ "$(uname -s)" = Darwin ] || { echo "macOS build requires a Mac" >&2; exit 1; }
ver=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
out=${DIST_DIR:-dist}
mkdir -p "$out"
brew list ffmpeg pkg-config llvm sevenzip unar >/dev/null 2>&1 || brew install ffmpeg pkg-config llvm sevenzip unar
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
prefix=starfold
build_args=()
if [ "${STARFOLD_GRAPHICAL:-0}" = 1 ]; then
  prefix=starfold-graphical
fi
. scripts/dist/build-amp-helper.sh
cargo build --workspace --release --locked --target aarch64-apple-darwin "${build_args[@]}"
bin="${CARGO_TARGET_DIR:-target}/aarch64-apple-darwin/release/starfold"
# KIT's standalone updater replaces only FOLD. Include matching compressed
# helpers so future updates/rollbacks cannot select stale adjacent providers.
gzip -n -c "$(dirname "$bin")/starfold-preview-pdf" > "$work/pdf.gz"
gzip -n -c "$(dirname "$bin")/starfold-preview-video" > "$work/video.gz"
gzip -n -c "$(dirname "$bin")/starfold-archive" > "$work/archive.gz"
export STARFOLD_BUNDLE_ARCHIVE="$work/archive.gz"
export STARFOLD_BUNDLE_PREVIEW_PDF="$work/pdf.gz"
export STARFOLD_BUNDLE_PREVIEW_VIDEO="$work/video.gz"
cargo build -p starfold --release --locked --target aarch64-apple-darwin "${build_args[@]}"

"$bin" --version
"$bin" --bundled-amp-version
stage="$work/$prefix-$ver"
mkdir -p "$stage"
install -m755 "$bin" "$stage/starfold"
for helper in starfold-preview-pdf starfold-preview-video starfold-preview-nvim starfold-archive; do
  install -m755 "$(dirname "$bin")/$helper" "$stage/$helper"
done
cp README.md LICENSE "$stage/"
cp -R documentation "$stage/"
mkdir -p "$stage/LICENSES/STARAMP"
cp "$amp_source/LICENSE" "$stage/LICENSES/STARAMP/"
[ ! -f "$amp_source/NOTICE" ] || cp "$amp_source/NOTICE" "$stage/LICENSES/STARAMP/"
[ ! -d "$amp_source/LICENSES" ] || cp -R "$amp_source/LICENSES/". "$stage/LICENSES/STARAMP/"
if [ "$prefix" = starfold-graphical ]; then
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
printf '%s\n' 'Requires Homebrew runtime codecs: brew install ffmpeg sevenzip unar' > "$stage/INSTALL.txt"
tar -C "$work" -czf "$out/$prefix-$ver-aarch64-apple-darwin.tar.gz" "$prefix-$ver"
echo "wrote $out/$prefix-$ver-aarch64-apple-darwin.tar.gz"
