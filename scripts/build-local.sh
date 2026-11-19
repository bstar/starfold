#!/usr/bin/env bash
# Build FOLD and its matching AMP helper through Nix.
# Published pins are the default; a CORE source override is development-only.
set -euo pipefail
cd "$(dirname "$0")/.."
fold_root=$PWD
core_root=${STARFOLD_CORE_SOURCE:-}
amp_default="$fold_root/../staramp"
amp_root=${STARFOLD_AMP_SOURCE:-"$amp_default"}
[ -f "$amp_root/Cargo.toml" ] || {
    echo 'Set STARFOLD_AMP_SOURCE to the matching STAR/AMP checkout.' >&2
    exit 1
}
export CARGO_NET_GIT_FETCH_WITH_CLI=true
build_args=(--locked)
if [ -n "$core_root" ]; then
    [ -f "$core_root/Cargo.toml" ] || { echo 'Invalid STARFOLD_CORE_SOURCE' >&2; exit 1; }
    core_root=$(cd "$core_root" && pwd)
    backup=$(mktemp -d)
    cp Cargo.lock "$backup/fold.lock"
    cp "$amp_root/Cargo.lock" "$backup/amp.lock"
    restore() {
        cp "$backup/fold.lock" "$fold_root/Cargo.lock"
        cp "$backup/amp.lock" "$amp_root/Cargo.lock"
        rm -rf "$backup"
    }
    trap restore EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    # Cargo receives one literal argument; no shell evaluates the TOML value.
    build_args=(--config "patch.\"https://github.com/bstar/starcore\".starcore.path=\"$core_root\"")
fi
(cd "$amp_root" && nix develop -c cargo build --release "${build_args[@]}")
nix develop -c cargo build --workspace --release "${build_args[@]}"
install -m755 "$amp_root/target/release/staramp" target/release/starfold-staramp
