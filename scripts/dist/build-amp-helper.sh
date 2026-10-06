#!/usr/bin/env bash
# Source this after creating $work and selecting the native release toolchain.
# The graphical helper is the exact revision in the graphical Nix package.
amp_revision=$(python3 -c 'import json; print(json.load(open("flake.lock"))["nodes"]["staramp-native"]["locked"]["rev"])')
amp_source="$work/staramp-source"
git init -q "$amp_source"
git -C "$amp_source" remote add origin https://github.com/bstar/staramp.git
git -C "$amp_source" fetch --depth=1 origin "$amp_revision"
git -C "$amp_source" checkout --detach FETCH_HEAD
amp_target="$work/staramp-target"
(
  cd "$amp_source"
  CARGO_TARGET_DIR="$amp_target" CARGO_NET_GIT_FETCH_WITH_CLI=true cargo build --release --locked
)
export STARFOLD_BUNDLE_STARAMP="$amp_target/release/staramp"
"$STARFOLD_BUNDLE_STARAMP" --version
