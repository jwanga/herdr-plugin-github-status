# Architecture

<!-- AUTO:GENERATED — managed by /engineering-plugin:architecture.
     Sections between AUTO:* markers are regenerated on every refresh.
     Prose outside markers (notably ## Decisions) is preserved verbatim. -->

<!-- AUTO:SUMMARY -->
`herdr-plugin-github-status` is a herdr plugin that renders a live GitHub project status (NOW, milestones, issues, PRs, an ACTIONS section of workflow runs, and an ACTIVITY feed of recent changes) in a narrow 26-column sidebar pane: herdr reads `herdr-plugin.toml` and launches `herdr/launch.sh`, which runs the ratatui TUI or forwards `dock <toggle|open|close>` (via `herdr/pane.sh`), with `dock.rs` and every other host interaction going through `herdr.rs`, a typed wrapper over `$HERDR_BIN_PATH`. A background thread in `poll.rs` ticks every 2 s, resolves the active pane's cwd into owner/repo plus branch through `repo.rs`, polls `herdr agent list` for workspace agents, and drives two fetch cadences through `github.rs`: `fetch_snapshot(repo, previous_runs)` on repo change, every 10 s (`POLL_INTERVAL`), or on an `r` refresh, plus a cheap runs-only `refresh_runs` every 5 s (`ACTIVE_INTERVAL`) while a run is active, a token is present, and the rate-limit budget stays above 500 requests. `github.rs` is a ureq REST + GraphQL client (token from `GH_TOKEN`/`GITHUB_TOKEN` or `gh auth token`, Link pagination, one-hop redirects, `RateLimited` backoff, GraphQL PR enrichment with a REST reviews fallback, latest 15 workflow runs and check runs for up to five PR heads via `check_heads`) whose GETs now go through `get_cached` in `cache.rs`: an `EtagCache` of etag plus compact re-serialized body per URL, persisted atomically as `etag-cache.json` (≤2 MB) in `HERDR_PLUGIN_STATE_DIR`, swept per fetch, so a 304 reuses the cached body and costs no rate limit when a token is present, with optional request logging to `debug.log` under `HERDR_GITHUB_STATUS_DEBUG=1`. `model.rs` shapes results into a `Snapshot` delivered to `app.rs` as `Msg::{Snapshot,Agents,NoRepo,Error}` over an mpsc channel, where `app.rs` owns UI state, runs `activity::diff(prev, next, now)` on each new snapshot to keep a newest-first feed of 50 events (issue opened/closed/reopened, milestone created/closed, PR opened/ready/reviewed/merged/closed, run started/finished), a 2-minute `recent` highlight map, and an `events_repo` that resets the feed on a different owner/name but survives branch checkouts, while `ui/tree.rs` flattens the snapshot into a width-independent `Vec<Node>` (NOW, milestones, expandable PRs with check runs, ACTIONS, ACTIVITY with the 10 latest events and relative age, recently merged/closed) and `ui/header.rs` shows badge, repo, branch, refresh age, and the remaining API budget as a `compact_count`. A `.github/workflows/ci.yml` workflow runs fmt, clippy with `-D warnings`, and `cargo test --locked` on every push and PR; milestone 1 "Status pane core" is complete, only `config.rs` (issue #8) remains planned, and milestone 2 adds auto-dock event hooks, config, and release packaging.
<!-- /AUTO:SUMMARY -->

<!-- AUTO:DIAGRAM -->
```mermaid
flowchart LR
    subgraph host["herdr host"]
        herdr["herdr daemon and CLI"]
        pane["terminal pane"]
    end

    subgraph entry["plugin entrypoints"]
        manifest["herdr-plugin.toml"]
        launch["herdr/launch.sh and herdr/pane.sh"]
    end

    subgraph automation["repository automation"]
        ciYml["ci.yml fmt, clippy, test"]
    end

    subgraph bin["Rust binary herdr-github-status"]
        mainRs["main.rs CLI"]
        appRs["app.rs TUI loop, UI state, event feed"]
        dockRs["dock.rs dock logic"]
        herdrRs["herdr.rs CLI wrapper"]
        pollRs["poll.rs background thread, two cadences"]
        repoRs["repo.rs repo resolution"]
        githubRs["github.rs REST and GraphQL client, conditional GETs"]
        cacheRs["cache.rs EtagCache, persisted etag and body per URL"]
        activityRs["activity.rs diff snapshots into events"]
        modelRs["model.rs snapshot shapes, runs and checks"]
        utilRs["util.rs process, open_url, rfc3339"]
    end

    subgraph ui["ui views"]
        uiMod["ui/mod.rs shared helpers"]
        uiTree["ui/tree.rs NOW, ACTIONS, ACTIVITY, flatten and render"]
        uiHeader["ui/header.rs and ui/help.rs badge, API budget, key overlay"]
    end

    subgraph planned["planned"]
        configRs["config.rs"]
    end

    subgraph ext["external"]
        ghApi["GitHub REST and GraphQL API"]
        gitRepo["local git repo"]
        ghCli["gh CLI"]
        stateDir["plugin state dir"]
        browser["browser"]
    end

    herdr -->|"reads manifest"| manifest
    herdr -->|"launches pane command or action"| launch
    launch -->|"exec, no args or dock mode"| mainRs
    mainRs -->|"default: run TUI"| appRs
    mainRs -->|"dock toggle open close"| dockRs
    appRs -->|"draws header, body, footer into"| pane
    dockRs -->|"pane list, open, resize"| herdrRs
    herdrRs -->|"shells out via HERDR_BIN_PATH"| herdr
    appRs -->|"spawns and sends refresh"| pollRs
    pollRs -->|"Msg Snapshot Agents NoRepo Error over mpsc"| appRs
    pollRs -->|"2 s cwd tick via pane list, agent list"| herdrRs
    pollRs -->|"resolves cwd"| repoRs
    pollRs -->|"fetch_snapshot on change, 10 s, or r; runs-only every 5 s while active and budget allows"| githubRs
    repoRs -->|"git remote -v, origin first"| gitRepo
    repoRs -->|"stdout helper"| utilRs
    githubRs -->|"stdout helper"| utilRs
    githubRs -->|"token fallback: gh auth token"| ghCli
    githubRs -->|"REST with If-None-Match, Link pagination, backoff; GraphQL PR enrichment"| ghApi
    githubRs -->|"etag lookup, store, sweep"| cacheRs
    cacheRs -->|"etag-cache.json, debug.log"| stateDir
    githubRs -->|"deserializes into"| modelRs
    modelRs -->|"Snapshot with runs and checks, AgentInfo carried by"| pollRs
    ghApi -->|"runs fmt clippy test on push and PR"| ciYml
    appRs -->|"diff previous vs new snapshot"| activityRs
    activityRs -->|"events for ACTIVITY rows"| uiTree
    appRs -->|"flatten snapshot with TreeState, render nodes"| uiTree
    appRs -->|"header lines, API budget, ? overlay lines"| uiHeader
    uiTree -->|"right_count, truncate, fit, age_string"| uiMod
    uiHeader -->|"fit, wrap, age_string, compact_count"| uiMod
    appRs -->|"o key: open_url"| utilRs
    utilRs -->|"spawns open or xdg-open"| browser
    configRs -.->|"defaults and state dir"| pollRs
```
<!-- /AUTO:DIAGRAM -->

<!-- AUTO:COMPONENTS -->
## Components

| Component | Purpose | Key files |
|---|---|---|
| Plugin manifest | Declares pane `status`, actions `open`/`close`/`toggle`, build step, and future event hooks | `herdr-plugin.toml` |
| Launch wrapper | Single entrypoint: fixes PATH, finds the binary (`bin/` then `target/release/`), runs the TUI or forwards `dock <mode>` | `herdr/launch.sh` |
| Action wrapper | Thin shim herdr invokes for actions; delegates to `launch.sh` with the dock mode | `herdr/pane.sh` |
| CI workflow | GitHub Actions workflow that runs `cargo fmt --check`, `cargo clippy -D warnings`, and `cargo test --locked` on every push and pull request | `.github/workflows/ci.yml` |
| Crate manifest | Workspace/crate definition and dependencies (anyhow, crossterm, ratatui, serde, serde_json, ureq) | `Cargo.toml` |
| CLI entry | Dispatches: no args runs the TUI; `dock <toggle\|open\|close>` runs actions; `sidebar-width` prints the target width | `src/main.rs` |
| TUI loop | ratatui event loop plus UI state (cached tree `nodes`, cursor, scroll, help flag, body rect); key/mouse handling (j/k, Enter/Space toggle, arrows collapse/expand, Tab section jumps, g/G, paging, `o` open in browser, `r`, `?`, click/wheel); on each `Msg::Snapshot` runs `activity::diff` against the previous snapshot and keeps a newest-first event feed (50 events), a `recent` highlight map (2 min), and `events_repo` so the feed resets on a different owner/name but survives branch checkouts; composes header, body, and footer from `ui/` | `src/app.rs` |
| Dock logic | Sidebar width, split-target selection, open plus exact-width snap, per-tab detection of existing status panes via `pane process-info` | `src/dock.rs` |
| herdr CLI wrapper | Typed wrapper over `$HERDR_BIN_PATH` JSON commands (pane list/layout/resize/rename/close, plugin pane open) with typed errors; `agent_list` returns the workspace's running agents for the NOW section | `src/herdr.rs` |
| Repo resolution | cwd to owner/repo and branch by parsing `git remote -v` (origin first) | `src/repo.rs` |
| GitHub client | ureq REST + GraphQL client: token from `GH_TOKEN`/`GITHUB_TOKEN` or `gh auth token`, Link pagination, one-hop redirect follow, rate-limit errors propagated as `RateLimited` for backoff; all GETs go through `get_cached`, which sends `If-None-Match` from the `EtagCache` and reuses the cached body on 304 (free of rate limit when a token is present); optional `debug.log` of request statuses when `HERDR_GITHUB_STATUS_DEBUG=1`; `fetch_snapshot(repo, previous_runs)` builds the full snapshot, enriching open PRs with one GraphQL query (review decision, checks rollup, closing issues) with a REST `/pulls/{n}/reviews` fallback when unauthenticated, plus the latest 15 workflow runs from `/actions/runs` and check runs for up to five open PR heads picked by the pure `check_heads` selector; `refresh_runs` is a cheap runs-only refresh | `src/github.rs` |
| Cache | `EtagCache`: etag plus compact re-serialized body per URL, persisted as `etag-cache.json` in `HERDR_PLUGIN_STATE_DIR` (≤2 MB) with atomic tmp+rename writes; `begin`/`sweep` evict entries untouched during a fetch, `remove` drops undecodable bodies | `src/cache.rs` |
| Activity | Pure `diff(prev, next, now) -> Vec<Event>` between snapshots: issue opened/closed/reopened, milestone created/closed, PR opened/ready/reviewed/merged/closed, run started/finished; ignores window re-entry and unknown→known reviews; new items count only when `created_at` is at or after `fetch_started_at` | `src/activity.rs` |
| Snapshot model | `Milestone`, `Issue`, `PullRequest` shapes plus the `Snapshot` aggregate with `runs` (`WorkflowRun`) and `checks` (check runs keyed by head SHA); `PrExtra`/`Checks` (`PrExtra::from_graphql`, `Checks::from_rollup`, `Checks::from_check_runs`), `CheckRun`, `Review`/`review_decision`, `closing_refs` body parsing, and `AgentInfo` | `src/model.rs` |
| Poller | Background thread: 2 s tick resolves cwd via `herdr pane list` and polls `herdr agent list`, sending `Msg::Agents` on change; two fetch cadences: `fetch_snapshot` on repo change / every 10 s (`POLL_INTERVAL`) / `r` refresh, and a runs-only `refresh_runs` every 5 s (`ACTIVE_INTERVAL`) while a run is queued or in progress, a token is present, and the rate-limit budget exceeds 500 requests (`fast_poll_allowed`); sends `Msg::{Snapshot,Agents,NoRepo,Error}` over an mpsc channel | `src/poll.rs` |
| Utilities | Shared `stdout(program, args, cwd)` process helper used by `repo.rs` and `github.rs`; `open_url` spawns `open`/`xdg-open` with a reaper thread for the `o` key; `parse_rfc3339` for GitHub timestamps | `src/util.rs` |
| UI helpers | Shared rendering helpers for the 26-column pane: `right_count`, `truncate`, `fit`, `wrap`, `age_string` | `src/ui/mod.rs` |
| Tree view | Width-independent `flatten(snapshot, state, now) -> Vec<Node>` driven by a `TreeState` of toggled nodes: a NOW section (active issue from branch `issue-<n>-*` or the current branch's PR, that PR with a review/checks tail, active workflow runs, workspace agents), sections → milestones → issues, closed-milestone group, expandable PR nodes that list their check runs, an ACTIONS section (status icon, workflow name, branch at 36+ columns, elapsed/duration), an ACTIVITY section showing the 10 latest events with relative age and `recent` highlighting, and a recently merged/closed group; plus per-node `render(node, w) -> Line` | `src/ui/tree.rs` |
| Header | Badge, repo, branch, refresh age, remaining API budget rendered via `compact_count`, and no-token/error markers | `src/ui/header.rs` |
| Help overlay | The `?` key overlay listing key bindings | `src/ui/help.rs` |
| Config | (planned) config file plus defaults; state dir persistence (issue #8) | `src/config.rs` |
<!-- /AUTO:COMPONENTS -->

## Decisions

## Generated

<!-- AUTO:META -->
Last refreshed: 2026-09-04 09:30
Triggered by: issue-close #6
Diagram type: flowchart
Source-of-truth: REQUIREMENTS.md ## Architecture + code structure scan
<!-- /AUTO:META -->
