# Project Memory

## Current State
- **Active Milestone**: Status pane core (#1)
- **Current Issue**: #1 Scaffold the plugin (implementation complete, merging)
- **Current Branch**: issue-1-scaffold
- **Plugin Version**: 1.2.1 (engineering-plugin)

## Progress Log
<!-- Each entry MUST use the format: [YYYY-MM-DD HH:MM] @username: description -->
- [2026-09-04 00:25] @jwanga: Bootstrapped project in unguided mode via `/next <description>`: git init, .gitignore, README/REQUIREMENTS/INSTRUCTIONS/MEMORY created (rules #1, #2). Private GitHub repo `jwanga/herdr-plugin-github-status` to be created; publishing milestone must flip it public and add the `herdr-plugin` topic.
- [2026-09-04 00:25] @jwanga: Housekeeping skipped this session per unguided rules #4–#7 (plugin compat, self-update, marketplace suggestions). Relevant installed plugins noted: rust-analyzer-lsp, github, security-guidance, superpowers.
- [2026-09-04 00:25] @jwanga: Researched herdr 0.8.0 plugin system (docs, API schema, marketplace plugins herdr-reviewr / herdr-sidebar / herdr-plus / ghzinga). Findings saved to `documentation/herdr-plugin-notes.md`.

- [2026-09-04 00:35] @jwanga: Created milestones #1 "Status pane core" (issues #1–#6) and #2 "Auto-dock, configuration, and publishing" (issues #7–#9) on GitHub (rules #10, #11).
- [2026-09-04 00:40] @jwanga: Issue #1 clarifying-question gate — accepted defaults: open via `plugin pane open` + `pane resize` snap; dock logic in Rust (`dock` subcommand) with a thin bash wrapper; one status pane per tab docked to the rightmost full-height pane; merge-commit strategy.
- [2026-09-04 00:55] @jwanga: Issue #1 implemented on `issue-1-scaffold`: Cargo crate (ratatui 0.30, crossterm 0.29), `herdr-plugin.toml`, `herdr/pane.sh` + `herdr/launch.sh`, `src/{main,herdr,dock,app}.rs`, 6 unit tests. Verified live in herdr 0.8.0: open → 26-column right pane, toggle/open/close idempotent, `q` closes the plugin pane, wrapper works under a minimal env.

## Key Decisions
<!-- Each entry MUST use the format: [YYYY-MM-DD HH:MM] @username: description -->
- [2026-09-04 00:25] @jwanga: Implementation language = Rust + ratatui + crossterm + ureq. Rationale: matches herdr itself and the leading sidebar plugins (reviewr, herdr-sidebar, ghzinga); single static binary; prebuilt-release install path is the ecosystem convention. Alternatives considered: (a) minimal bash + `gh` + `watch` script — cannot do interactive tree/mouse or ETag polling well; (b) Node/ink — needs a Node runtime at install and is not the ecosystem norm; (c) Go/bubbletea (herdr-plus style) — viable, but Rust keeps parity with herdr's own tooling and the reviewer/sidebar precedents.
- [2026-09-04 00:25] @jwanga: Pane width = herdr sidebar width (session.json `sidebar_width`, config `[ui] sidebar_width`, default 26). Verified: `pane split --ratio R` gives the first pane floor(W*R) columns, so a right pane of N columns uses ratio 1 - N/W.
- [2026-09-04 00:25] @jwanga: "Real time" = conditional-request polling (ETag) every 10 s (5 s while runs are active) plus snapshot diffing into an activity feed. GitHub has no push channel to a local TUI without webhooks; ETag 304s do not count against the rate limit.
- [2026-09-04 00:25] @jwanga: Plugin id `jwanga.github-status`, pane id `status`, binary `herdr-github-status`.

## Notes
<!-- Each entry MUST use the format: [YYYY-MM-DD HH:MM] @username: description -->
- [2026-09-04 00:55] @jwanga: Verified: a `plugin pane open` split pane closes by itself when its command exits; `pane resize --amount` is a split-ratio delta and `--direction` is the direction the pane's edge moves (result under `.result.resize.layout`); `plugin action invoke` always targets the focused workspace, so live tests run `herdr-github-status dock <mode>` directly with the caller pane's `HERDR_*` env.
- [2026-09-04 00:25] @jwanga: herdr runs plugin commands with a minimal PATH and no shell; `plugin pane open --placement split` requires `--target-pane`; `plugin pane close` only knows panes opened this server session — sweep with plain `pane close`.
