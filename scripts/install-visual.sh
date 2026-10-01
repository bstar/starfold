#!/usr/bin/env bash
# Install only the separate experimental command; retain the regular debug launcher.
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ -z ${IN_NIX_SHELL:-} ]]; then
  exec nix --extra-experimental-features 'nix-command flakes' develop -c bash scripts/install-visual.sh "$@"
fi
cargo build --release --locked --features desktop
star_visual_binary="$PWD/target/release/starfold"
star_visual_bin_dir="${STARFOLD_VISUAL_BIN_DIR:-$HOME/.local/bin}"
star_visual_launcher="$star_visual_bin_dir/starfold-visual"
if [[ -e "$star_visual_launcher" ]] && ! rg -q '^# STAR/FOLD visual experiment$' "$star_visual_launcher"; then
  echo "Existing $star_visual_launcher is not this experiment's launcher" >&2
  exit 1
fi
mkdir -p "$star_visual_bin_dir"
star_visual_temp=$(mktemp)
trap 'rm -f "$star_visual_temp"' EXIT
{
  printf '#!/usr/bin/env bash\n# STAR/FOLD visual experiment\n'
  printf 'export STARFOLD_LOG=${STARFOLD_LOG:-debug}\n'
  printf 'export XDG_DATA_DIRS=%q${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}\n' "${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
  printf 'export LD_LIBRARY_PATH=%q${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}\n' "${LD_LIBRARY_PATH:-}"
  printf 'if [[ ${1:-} == --version ]]; then exec %q --version; fi\n' "$star_visual_binary"
  printf 'exec %q visual "$@"\n' "$star_visual_binary"
} > "$star_visual_temp"
install -m755 "$star_visual_temp" "$star_visual_launcher"
"$star_visual_launcher" --help
