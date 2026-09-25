#!/usr/bin/env bash
# Build a disposable tree for the milestone 0 manual checks.
set -euo pipefail

baseline_dir=$(mktemp -d "${TMPDIR:-/tmp}/starfold-baseline.XXXXXX")
mkdir -p "$baseline_dir/files/project-a/nested" "$baseline_dir/files/project-b"
printf 'find this text\n' > "$baseline_dir/files/project-a/nested/note.txt"
printf 'a name with spaces\n' > "$baseline_dir/files/project-a/with spaces.txt"
printf 'hidden marker\n' > "$baseline_dir/files/project-a/.hidden"
printf 'source version\n' > "$baseline_dir/files/project-a/shared.txt"
printf 'destination version\n' > "$baseline_dir/files/project-b/shared.txt"
ln -s nested/note.txt "$baseline_dir/files/project-a/note-link"
ln -s missing.txt "$baseline_dir/files/project-a/broken-link"
printf '%s\n' "$baseline_dir"
