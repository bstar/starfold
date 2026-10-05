#!/usr/bin/env bash
# Build and test a pacman package as an ordinary user on Arch Linux.
set -euo pipefail
cd "$(dirname "$0")/../.."
[ "$(id -u)" -ne 0 ] || { echo "run Arch packaging as an unprivileged user" >&2; exit 1; }
command -v makepkg >/dev/null || { echo "Arch packaging requires makepkg" >&2; exit 1; }

ver=$(sed -n '0,/^version = /s/^version = "\(.*\)"/\1/p' Cargo.toml)
out=${DIST_DIR:-dist}
mkdir -p "$out"
out=$(realpath "$out")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Package the current sources, including the vendored input parser and test
# fixtures. Never reuse a host-built binary or expose its target directory.
tar --transform "s,^,starfold-$ver/," -czf "$work/starfold-$ver.tar.gz" \
    Cargo.toml Cargo.lock flake.nix flake.lock src tests testdata vendor \
    packaging scripts docs .github README.md LICENSE NOTICE LICENSES
sum=$(sha256sum "$work/starfold-$ver.tar.gz" | cut -d' ' -f1)
sed -e "s/@VERSION@/$ver/g" -e "s/@SHA256@/$sum/g" \
    packaging/arch/PKGBUILD.in > "$work/PKGBUILD"

cd "$work"
makepkg --noconfirm
# Ship the build recipe and checksummed source beside the installable package.
cp PKGBUILD "starfold-$ver.tar.gz" "$out/"
while IFS= read -r package; do
    cp "$package" "$out/"
done < <(makepkg --packagelist)
echo "wrote Arch package and source recipe to $out"
