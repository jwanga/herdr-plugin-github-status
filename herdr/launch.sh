#!/bin/sh
# Single entrypoint for both the pane (no arguments: run the TUI) and the actions
# (`launch.sh dock <mode>`). Fixes PATH (herdr runs plugin commands with a minimal one)
# and locates the binary for installed (bin/) and linked (target/release/) checkouts.
export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"
root="${HERDR_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
bin="$root/bin/herdr-github-status"
[ -x "$bin" ] || bin="$root/target/release/herdr-github-status"
if [ ! -x "$bin" ]; then
  printf 'herdr-github-status: binary not found; run `cargo build --release` in %s\n' "$root" >&2
  if [ $# -eq 0 ]; then
    printf 'press Enter to close\n'
    read -r _
  fi
  exit 1
fi
exec "$bin" "$@"
