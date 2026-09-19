# Project Instructions

## General
- Language: Rust (stable), TUI with ratatui + crossterm, HTTP with ureq. Keep the dependency set small so release builds stay fast and static.
- Follow the herdr plugin conventions captured in `documentation/herdr-plugin-notes.md` (verified against herdr 0.8.0). When in doubt, the installed `herdr` binary's `--help` output is the authority.
- The pane must be usable at 26 columns; test renders at that width.
- Never write to GitHub (read-only plugin). Never store tokens in the plugin root; read them from the environment or `gh auth token`.

## Build
cargo build --release

## Run
herdr plugin link . && herdr plugin action invoke toggle --plugin jwanga.github-status

## Test
cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test --locked

CI (`.github/workflows/ci.yml`) enforces exactly these three commands on every push and pull request.

## Deploy
Tag `vX.Y.Z` (matching `herdr-plugin.toml` and `Cargo.toml`; `scripts/check-versions.sh` enforces it in CI and in the release workflow). Pushing the tag runs `.github/workflows/release.yml`, which builds the four platform binaries and attaches them, with SHA-256 sidecars, to the tag's GitHub Release. `scripts/fetch-or-build.sh` (the manifest `[[build]]` step) downloads those on `herdr plugin install`.

## Releases
mode: single
trigger: milestone
components:
  - name: herdr-github-status
    path: .
    tag: v{version}
    manifests:
      - Cargo.toml:package.version
      - herdr-plugin.toml:version
      - Cargo.lock
    artifacts: [rust-bin]
    registry: none

`Cargo.lock` records the crate's own version: refresh it (`cargo update -p herdr-github-status --offline`, or any `cargo build`) in the release commit, or the `--locked` builds in CI and the release workflow fail.

## Overrides
- This repository is public. Use the GitHub handle `jwanga` for MEMORY.md entries (`@jwanga:`), never the OS username or an email address; commit with the GitHub noreply email (set in this repo's `.git/config`); never write absolute home-directory paths into tracked files.
