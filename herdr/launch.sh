#!/bin/sh
# Pane entrypoint: run the status TUI. Prefers the installed binary in bin/ and
# falls back to a local cargo build for `herdr plugin link` checkouts.
root="${HERDR_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
bin="$root/bin/herdr-github-status"
[ -x "$bin" ] || bin="$root/target/release/herdr-github-status"
if [ ! -x "$bin" ]; then
  printf 'herdr-github-status: binary not found; run `cargo build --release` in %s\n' "$root" >&2
  printf 'press Enter to close\n'
  read -r _
  exit 1
fi
exec "$bin"
