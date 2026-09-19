#!/bin/sh
# The manifest's [[build]] step (runs once on `herdr plugin install`, cwd = the checkout).
# Installs the binary into bin/, where herdr/launch.sh looks first:
#   1. download the prebuilt release asset for this platform and the manifest's version,
#      verify it against its SHA-256 sidecar;
#   2. failing that (no release for this version, unknown platform, no network, checksum
#      mismatch), build from source with cargo.
# FETCH_OR_BUILD_SKIP_DOWNLOAD=1 forces the source build; FETCH_OR_BUILD_BASE_URL replaces
# the GitHub release URL (a mirror, or file:// in tests).
set -eu

BIN=herdr-github-status
REPO=jwanga/herdr-plugin-github-status
export PATH="${HOME:-}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

say() { printf '%s: %s\n' "$BIN" "$*" >&2; }

version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' herdr-plugin.toml | head -n 1)

target() {
  case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) echo aarch64-apple-darwin ;;
    Darwin-x86_64) echo x86_64-apple-darwin ;;
    Linux-x86_64) echo x86_64-unknown-linux-musl ;;
    Linux-aarch64 | Linux-arm64) echo aarch64-unknown-linux-musl ;;
    *) return 1 ;;
  esac
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d ' ' -f 1
  else
    shasum -a 256 "$1" | cut -d ' ' -f 1
  fi
}

fetch() {
  [ "${FETCH_OR_BUILD_SKIP_DOWNLOAD:-}" != 1 ] || return 1
  [ -n "$version" ] || { say "no version in herdr-plugin.toml"; return 1; }
  command -v curl >/dev/null 2>&1 || { say "curl not found"; return 1; }
  triple=$(target) || { say "no prebuilt binary for $(uname -s) $(uname -m)"; return 1; }
  asset="$BIN-$triple"
  url="${FETCH_OR_BUILD_BASE_URL:-https://github.com/$REPO/releases/download/v$version}/$asset"
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  say "downloading $asset v$version"
  curl -fsSL --retry 2 -o "$tmp/$asset" "$url" || { say "no release asset at $url"; return 1; }
  curl -fsSL --retry 2 -o "$tmp/$asset.sha256" "$url.sha256" || { say "no checksum for $asset"; return 1; }
  want=$(cut -d ' ' -f 1 "$tmp/$asset.sha256")
  got=$(sha256 "$tmp/$asset")
  if [ -z "$want" ] || [ "$want" != "$got" ]; then
    say "checksum mismatch for $asset (expected $want, got $got)"
    return 1
  fi
  mkdir -p bin
  mv "$tmp/$asset" "bin/$BIN"
  chmod +x "bin/$BIN"
  # The download must actually run here (wrong libc, quarantine, truncated file...).
  "bin/$BIN" --version >/dev/null 2>&1 || { say "downloaded binary does not run"; rm -f "bin/$BIN"; return 1; }
  say "installed prebuilt bin/$BIN"
}

build() {
  command -v cargo >/dev/null 2>&1 || {
    say "no prebuilt binary was available and cargo is not installed; install Rust from https://rustup.rs and reinstall the plugin"
    exit 1
  }
  say "building from source"
  cargo build --release --locked
  mkdir -p bin
  cp "target/release/$BIN" "bin/$BIN"
  say "installed source-built bin/$BIN"
}

fetch || build
