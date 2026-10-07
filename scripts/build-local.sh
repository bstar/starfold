#!/usr/bin/env bash
# Build the coordinated FOLD/KIT/AMP development checkouts through Nix.
# Restore git-sourced lockfiles after Cargo resolves the temporary path patch.
set -euo pipefail
cd "$(dirname "$0")/.."
fold_root=$PWD
kit_root=${STARFOLD_KIT_SOURCE:-"$fold_root/../starkit"}
amp_root=${STARFOLD_AMP_SOURCE:-"$fold_root/../staramp-ssh-audio"}
[ -f "$kit_root/Cargo.toml" ] && [ -f "$amp_root/Cargo.toml" ] || {
    echo 'Set STARFOLD_KIT_SOURCE and STARFOLD_AMP_SOURCE to the matching checkouts.' >&2
    exit 1
}
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
export CARGO_NET_GIT_FETCH_WITH_CLI=true
patch="patch.\"https://github.com/bstar/starkit\".starkit.path=\"$kit_root\""
nix develop -c cargo build --manifest-path "$amp_root/Cargo.toml" --release --config "$patch"
nix develop -c cargo build --workspace --release --config "$patch"
install -m755 "$amp_root/target/release/staramp" target/release/starfold-staramp
