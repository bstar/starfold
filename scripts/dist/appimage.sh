#!/usr/bin/env bash
# The AppImage. Build in Debian Bullseye to retain the supported glibc floor.
#
# Native preview builds carry shared FFmpeg/ALSA dependencies. The walk below
# bundles non-runtime libraries and keeps the supported Bullseye glibc floor.
set -euo pipefail
cd "$(dirname "$0")/../.."

appimage_arch=$(uname -m)
case "$appimage_arch" in
  x86_64|aarch64) ;;
  *) echo "unsupported AppImage architecture: $appimage_arch" >&2; exit 1 ;;
esac

. scripts/dist/deps-debian.sh
. "$HOME/.cargo/env"
apt-get install -y -qq --no-install-recommends patchelf squashfs-tools

ver=$(sed -n '0,/^version = /s/^version = "\(.*\)"/\1/p' Cargo.toml)
out=${DIST_DIR:-dist}
mkdir -p "$out"

# --locked, not --frozen: one dependency comes from a git tag rather than from
# crates.io, and the container has no fetched copy of it yet.
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
prefix=starfold
build_args=()
if [ "${STARFOLD_GRAPHICAL:-0}" = 1 ]; then
  prefix=starfold-graphical
  apt-get install -y -qq --no-install-recommends python3
  . scripts/dist/build-amp-helper.sh
  build_args+=(--features terminal-graphics)
fi
cargo build --release --locked "${build_args[@]}"
# Not `target/`: the container is handed its own CARGO_TARGET_DIR so it cannot
# leave a Debian binary where the host's next `cargo run` expects a native one.
bin="${CARGO_TARGET_DIR:-target}/release/starfold"
scripts/dist/glibc-floor.sh "$bin"

appdir=$work/AppDir
install -Dm755 "$bin"                         "$appdir/usr/bin/starfold"
install -Dm644 packaging/starfold.desktop     "$appdir/starfold.desktop"
install -Dm644 packaging/starfold.png         "$appdir/starfold.png"
install -Dm644 packaging/starfold.desktop     "$appdir/usr/share/applications/starfold.desktop"
install -Dm644 packaging/starfold.png         "$appdir/usr/share/icons/hicolor/256x256/apps/starfold.png"
install -Dm644 packaging/starfold.svg         "$appdir/usr/share/icons/hicolor/scalable/apps/starfold.svg"
install -Dm644 README.md LICENSE NOTICE -t           "$appdir/usr/share/doc/starfold/"
install -Dm644 LICENSES/UnRAR.txt "$appdir/usr/share/doc/starfold/LICENSES/UnRAR.txt"
install -Dm644 LICENSES/OFL-Liberation.txt "$appdir/usr/share/doc/starfold/LICENSES/OFL-Liberation.txt"
for media_license in LICENSES/ffmpeg-*.txt; do
  install -Dm644 "$media_license" "$appdir/usr/share/doc/starfold/LICENSES/$(basename "$media_license")"
done
cp "$appdir/starfold.png" "$appdir/.DirIcon"
mkdir -p "$appdir/usr/lib"

# What stays behind, and why.
#
#   the C runtime    bundling a C library into an AppImage is how they break.
#   libgcc_s,        the host's is never older than bullseye's, and a bundled
#   libstdc++        old one is a real hazard on a newer host.
#
# Codec and audio libraries are bundled; the system runtime stays outside.
keep_out='^(ld-linux|libc\.so|libm\.so|libdl\.so|libpthread\.so|librt\.so|libresolv\.so|libutil\.so|libnsl\.so|libgcc_s\.so|libstdc\+\+\.so)'

# Walk NEEDED transitively. ldd on the binary already reports the whole graph,
# so one pass is enough; the loop is over what it found, not over levels.
libraries=("$appdir/usr/bin/starfold")
if [ "$prefix" = starfold-graphical ]; then
  libraries+=("$STARFOLD_BUNDLE_STARAMP")
  mkdir -p "$appdir/usr/share/starfold"
  touch "$appdir/usr/share/starfold/graphical"
  scripts/dist/glibc-floor.sh "$STARFOLD_BUNDLE_STARAMP"
  mkdir -p "$appdir/usr/share/doc/starfold/LICENSES/STARAMP"
  cp "$amp_source/LICENSE" "$appdir/usr/share/doc/starfold/LICENSES/STARAMP/"
  [ ! -f "$amp_source/NOTICE" ] || cp "$amp_source/NOTICE" "$appdir/usr/share/doc/starfold/LICENSES/STARAMP/"
  [ ! -d "$amp_source/LICENSES" ] || cp -R "$amp_source/LICENSES/". "$appdir/usr/share/doc/starfold/LICENSES/STARAMP/"
fi
ldd "${libraries[@]}" | awk '{print $3}' | grep -E '^/' | sort -u | while read -r lib; do
  base=$(basename "$lib")
  if echo "$base" | grep -qE "$keep_out"; then
    echo "host:   $base"
    continue
  fi
  echo "bundle: $base"
  cp -L "$lib" "$appdir/usr/lib/"
done

# The loader resolves any bundled copies with no environment set at all, and
# RUNPATH beats ld.so.cache, so a host library cannot shadow ours even when the
# soname matches exactly. Harmless while usr/lib is empty, and correct the day
# it is not.
patchelf --set-rpath '$ORIGIN/../lib' "$appdir/usr/bin/starfold"
for so in "$appdir"/usr/lib/*.so*; do
  [ -e "$so" ] || continue
  patchelf --set-rpath '$ORIGIN' "$so"
done

cat > "$appdir/AppRun" <<'EOF'
#!/bin/sh
# The binary's RUNPATH is $ORIGIN/../lib, so no LD_LIBRARY_PATH is needed and
# none is exported: nothing this launches should inherit our library path. It
# launches the program configured to open a file, with the user's own
# environment.
#
# "$@" is not optional. `starfold list` and its flags are the whole of the
# headless interface, and an AppImage that swallowed argv would be useless
# for them.
HERE=$(dirname "$(readlink -f "$0")")
if [ -f "$HERE/usr/share/starfold/graphical" ]; then
  case "${1-}" in
    list|update|help|graphical|--graphical-*|--preview-worker|--archive-worker|--elevated-delete|--bundled-amp-version|--version|-V|--help|-h) ;;
    *) set -- graphical "$@" ;;
  esac
fi
exec "$HERE/usr/bin/starfold" "$@"
EOF
chmod +x "$appdir/AppRun"

# The type-2 runtime, downloaded rather than executed: this is the small ELF
# that gets prepended to the filesystem image and does the mounting at run
# time. Nothing here has to run it, which is the point.
#
# Pinned to a dated tag and checksummed, because it is the first code that runs
# when anybody opens this AppImage. `continuous` is a rolling tag GitHub
# rewrites in place: a build that consumes it produces an artifact nobody can
# reproduce, and the provenance attestation would faithfully attest a build
# that pulled in whatever was at that URL on the day. The checksum is verified
# before the file is made executable, so bad bytes never reach `cat` below.
#
# To move it: pick a tag from
# https://github.com/AppImage/type2-runtime/releases, download its
# runtimes for both architectures, and put their `sha256sum` here.
runtime_tag=20251108
case "$appimage_arch" in
  x86_64) runtime_sha256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d ;;
  aarch64) runtime_sha256=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444 ;;
esac

curl -fsSL -o "$work/runtime" \
  "https://github.com/AppImage/type2-runtime/releases/download/$runtime_tag/runtime-$appimage_arch"
echo "$runtime_sha256  $work/runtime" | sha256sum -c -
chmod +x "$work/runtime"

# gzip rather than zstd: every AppImage runtime in the wild can read it, and
# this file is meant for the machines we have not thought of.
mksquashfs "$appdir" "$work/fs.squashfs" -root-owned -noappend -comp gzip -no-progress

target="$out/$prefix-$ver-$appimage_arch.AppImage"
cat "$work/runtime" "$work/fs.squashfs" > "$target"
chmod +x "$target"
ls -la "$target"
echo "wrote $target"
