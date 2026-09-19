#!/bin/sh
# Cargo.toml and herdr-plugin.toml must carry the same version; with a tag argument
# (vX.Y.Z, as the release workflow passes) the tag must match too.
set -eu
cd "$(dirname "$0")/.."
version_of() { sed -n 's/^version *= *"\(.*\)"/\1/p' "$1" | head -n 1; }
cargo=$(version_of Cargo.toml)
manifest=$(version_of herdr-plugin.toml)
if [ -z "$cargo" ] || [ "$cargo" != "$manifest" ]; then
  echo "version mismatch: Cargo.toml=$cargo herdr-plugin.toml=$manifest" >&2
  exit 1
fi
if [ $# -gt 0 ] && [ "$1" != "v$cargo" ]; then
  echo "tag $1 does not match version $cargo" >&2
  exit 1
fi
echo "version $cargo"
