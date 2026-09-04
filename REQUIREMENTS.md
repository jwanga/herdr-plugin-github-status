# herdr-plugin-github-status
A herdr plugin that docks a persistent, real-time project **status** pane on the right edge of the screen, driven by GitHub milestones, issues, pull requests, and Actions.

## Table of Contents
- [Milestones](#milestones)
- [Requirements](#requirements)
  - [Pane and layout](#pane-and-layout)
  - [Repository detection and authentication](#repository-detection-and-authentication)
  - [Data shown](#data-shown)
  - [Real-time updates](#real-time-updates)
  - [Interaction](#interaction)
  - [Configuration](#configuration)
  - [Packaging and distribution](#packaging-and-distribution)
  - [Constraints](#constraints)
- [Architecture](#architecture)

## Milestones
- [ ] **Status pane core** — A Rust/ratatui TUI shipped as a herdr plugin pane. Docks on the right at the left sidebar's width, detects the workspace's GitHub repository, and renders milestones → issues, unassigned issues, pull requests, and Actions/check runs, polling GitHub continuously so state changes appear within seconds. Keyboard and mouse navigation, expand/collapse, open-in-browser.
- [ ] **Auto-dock, configuration, and publishing** — Event hooks keep a status pane present in every tab/workspace (with snooze when the user closes it), width re-applies on terminal resize, a user config file (interval, width, auto-open, sections), a GitHub Release workflow with prebuilt binaries plus a fetch-or-build install step, README/marketplace metadata, and the `herdr-plugin` topic on a public repository.

## Requirements

### Pane and layout
- The plugin registers a `[[panes]]` entrypoint with id `status`, title `status`, placement `split`.
- Opening the pane splits the rightmost full-height pane of the tab (the focused pane on ties) to the **right** via `plugin pane open`, then snaps the new pane to the herdr sidebar width with `pane resize`: `sidebar_width` from `~/.config/herdr/session.json`, else 26 columns.
- The pane keeps its column width when the surrounding tab is resized (re-applies the split ratio).
- Actions: `open`, `close`, `toggle` (contexts: workspace, pane). Opening is idempotent per tab; toggling closes an existing status pane.
- The pane never steals focus when opened by a hook; a manual open focuses it.

### Repository detection and authentication
- The pane resolves `owner/repo` from the git remote (`origin`, then any GitHub remote) of the workspace's live working directory, and re-resolves when the focused pane's `foreground_cwd` changes.
- Authentication: `GH_TOKEN` / `GITHUB_TOKEN`, then `gh auth token`. Unauthenticated public-repo fallback with a visible warning.
- If no GitHub repository is detected, the pane says so and keeps watching for one.

### Data shown
- **Now** section: the active issue (branch `issue-<n>-*` or open PR closing `#n`), open PRs on the current branch, in-progress/queued workflow runs, and herdr agents in this workspace with lifecycle state.
- **Milestones** section: each open milestone with `closed/total` and a progress bar; expanded to its issues with state icons; closed milestones collapsed under a separate group.
- **Issues** section: open issues without a milestone; recently closed issues available under a collapsed group.
- **Pull requests** section: open PRs with draft/review decision/checks rollup and linked issues; recently merged/closed PRs in a collapsed group.
- **Actions** section: the most recent workflow runs (status, conclusion, branch, elapsed), plus check runs for open PR heads.
- Every row is truncated to fit a 26-column pane without horizontal scrolling.

### Real-time updates
- Poll GitHub on a short interval (default 10 s) using conditional requests (ETag / `If-None-Match`) so unchanged resources cost no rate limit; in-progress workflow runs poll faster (default 5 s).
- Detect state transitions between snapshots (issue opened/closed, PR merged, run started/finished, milestone closed) and surface them: a change marker on the row for a short window and an **Activity** feed of the latest transitions.
- Show last-refresh age and rate-limit remaining in the header; show errors inline without crashing.
- Manual refresh on `r`.

### Interaction
- `j`/`k`/arrows move, `Enter`/`Space` expands or collapses, `Tab` jumps between sections, `o` opens the selected item in the browser, `r` refreshes, `?` shows help, `q` closes the pane.
- Mouse: click to select/expand, wheel to scroll.

### Configuration
- `config.toml` in `HERDR_PLUGIN_CONFIG_DIR` (fallback `herdr plugin config-dir`): `poll_interval_secs`, `active_poll_interval_secs`, `width` (`sidebar` or a column count), `auto_open`, `sections` order/visibility, `recent_window_hours`.
- Runtime state (ETags, snooze markers) in `HERDR_PLUGIN_STATE_DIR`.

### Packaging and distribution
- Manifest `herdr-plugin.toml` at the repository root with `id = "jwanga.github-status"`, `name`, `version`, `min_herdr_version = "0.8.0"`, `platforms = ["macos", "linux"]`.
- `[[build]]` runs a fetch-or-build script: download the release asset for the platform triple and verify SHA-256, else `cargo build --release`.
- GitHub Actions release workflow builds `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` on tag push.
- Public repository tagged with the `herdr-plugin` topic so the marketplace indexes it.

### Constraints
- Single static binary, no runtime dependency beyond `git` and (optionally) `gh`.
- Plugin commands run without a shell and with a minimal `PATH`; scripts must prepend common bin dirs and use `HERDR_BIN_PATH`.
- Never write to the GitHub repository; the pane is read-only.

## Architecture
- **`herdr-plugin.toml`** — manifest: pane `status`, actions `open`/`close`/`toggle`, event hooks (milestone 2), build step.
- **`herdr/launch.sh`** — single entrypoint: fixes PATH, finds the binary (`bin/` then `target/release/`), runs the TUI with no arguments or forwards `dock <mode>`; `herdr/pane.sh` is the action wrapper that calls it.
- **Rust crate `herdr-github-status`** (`src/`):
  - `main.rs` — CLI: default runs the TUI; `dock <toggle|open|close>` implements the actions; `sidebar-width` prints the target width.
  - `dock.rs` — dock logic: sidebar width, split-target selection, open + exact-width snap, per-tab detection of existing status panes via `pane process-info`.
  - `herdr.rs` — wrapper over `HERDR_BIN_PATH` JSON commands (pane list/layout/resize/rename/close, plugin pane open, agent list) with typed errors.
  - `repo.rs` — cwd → owner/repo + branch resolution via git.
  - `github.rs` — REST client (ureq) with ETag cache, rate-limit tracking; fetches milestones, issues, PRs, workflow runs, check runs.
  - `model.rs` — normalized snapshot; diff of consecutive snapshots → activity events.
  - `poll.rs` — background thread scheduling fetches; sends snapshots over a channel.
  - `ui/` — ratatui rendering: header, section tree, activity feed, help overlay; 26-column-aware truncation.
  - `config.rs` — config file + defaults; state dir persistence.
