#!/usr/bin/env bash
# Action entrypoint: `pane.sh toggle|open|close`. All logic lives in the binary's
# `dock` subcommand; this wrapper only fixes PATH (herdr runs plugin commands with a
# minimal one) and locates the binary for both installed (bin/) and linked
# (target/release/) checkouts.
set -uo pipefail
export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"

root="${HERDR_PLUGIN_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)}"
bin="$root/bin/herdr-github-status"
[ -x "$bin" ] || bin="$root/target/release/herdr-github-status"
if [ ! -x "$bin" ]; then
  printf 'herdr-github-status: binary not found; run `cargo build --release` in %s\n' "$root" >&2
  exit 1
fi
exec "$bin" dock "${1:-toggle}"
