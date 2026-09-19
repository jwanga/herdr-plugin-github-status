# herdr-plugin-github-status
A [herdr](https://herdr.dev) plugin that docks a real-time **status** pane on the right edge of the screen. It shows, at a glance, where a project is: GitHub milestones and their issues, open and closed issues and pull requests, what is actively being worked on, and the live state of GitHub Actions workflow runs and check runs.

## Table of Contents
- [Installation](#installation)
- [Usage](#usage)
  - [Opening the pane](#opening-the-pane)
  - [Keys](#keys)
  - [Configuration](#configuration)

## Installation
Requires herdr ≥ 0.8.0 and `git`. Recommended: an authenticated GitHub CLI (`gh auth login`) or a `GH_TOKEN` / `GITHUB_TOKEN` environment variable, needed for private repositories and the higher rate limit.

```sh
herdr plugin install jwanga/herdr-plugin-github-status
```

The install step downloads the prebuilt binary for your platform from the matching GitHub Release and verifies its SHA-256 (macOS arm64/x86_64, Linux x86_64/aarch64, static musl), so no Rust toolchain is needed. If there is no prebuilt binary for the version or platform, or the download cannot be verified, it builds from source instead, which needs `cargo` ([rustup.rs](https://rustup.rs)).

To update, reinstall; your `config.toml` is kept:

```sh
herdr plugin uninstall jwanga.github-status
herdr plugin install jwanga/herdr-plugin-github-status
```

From a local checkout while developing (linked plugins skip the build step, so build first):

```sh
cargo build --release
herdr plugin link .
```

## Usage
### Opening the pane
```sh
herdr plugin action invoke toggle --plugin jwanga.github-status
```

Bind it to a key in `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+g"
type = "plugin_action"
command = "jwanga.github-status.toggle"
```

The pane follows the working directory of the workspace's focused pane (so pointing an agent at another checkout switches the status view), resolves the GitHub repository from the `origin` remote (or any other github.com remote), and refreshes every 10 seconds. It authenticates with `GH_TOKEN` / `GITHUB_TOKEN` or `gh auth token`; without a token it still reads public repositories and shows a `no-token` marker.

The pane docks on the right of the current tab at the same width as herdr's left sidebar (read live from herdr's `session.json`, default 26 columns). `toggle` closes an open status pane in the tab; `open` focuses an existing one; `close` closes every status pane in the workspace. Note that `herdr plugin action invoke` targets the *focused* workspace, wherever you run it.

### Auto-dock
Once the plugin is enabled, a status pane docks itself in every tab the first time you create or focus it — in every workspace, whether or not it is a GitHub repository — and never takes focus when it does. Close it (`q`, the `toggle`/`close` actions, or herdr's close-pane) and it stays closed for that tab until you `toggle` it back; closing with `q` or an action is remembered across herdr restarts. Zoomed tabs and tabs narrower than three pane widths are skipped.

The pane holds its column width when the terminal, the sidebar, or a neighbouring split changes size. If you drag the pane's edge yourself, that becomes the width it holds until it is closed.

### Keys
| Key | Action |
| --- | --- |
| `j` / `k`, `↑` / `↓` | Move the cursor |
| `Enter` / `Space` | Expand or collapse the selected section, milestone, group, or pull request |
| `←` / `→` | Collapse / expand the selected node |
| `Tab` / `Shift+Tab` | Jump to the next / previous section |
| `g` / `G`, `Home` / `End` | Top / bottom |
| `Ctrl+D` / `Ctrl+U` | Half page down / up |
| `PageDown` / `PageUp` | Page down / up |
| `o` | Open the selected item on github.com |
| `r` | Refresh now |
| `?` | Key help |
| `q` / `Ctrl+Q` / `Ctrl+C` | Close the pane |

Mouse: click a row to select it, click it again to expand or collapse, wheel to scroll.

The **NOW** section at the top shows what is in progress: the active issue (from a branch named `issue-<n>-…`, or the issue the current branch's pull request closes), the current branch's pull request with its review decision (`A` approved, `C` changes requested, `R` review required) and checks state (`✓` passing, `◔` pending, `✗` failing; `·` when unknown, which is the case for all but the current branch's PR without a token), and the herdr agents in this workspace with their state (`◐` working, `■` blocked, `✓` done, `○` idle). The active issue is marked `▶` in the milestone tree as well.

The tree shows open milestones with `closed/total` (and a progress bar when the pane is 32+ columns wide) and their issues (open first, with the assignee's initials when the pane is 30+ columns wide), a collapsed group of closed milestones, open issues with no milestone plus a collapsed group of issues closed in the last 24 hours, and open pull requests with the same review/checks tail; expanding a pull request shows its branch and the issues it closes, and a collapsed group lists pull requests merged or closed in the last 24 hours. Expanded pull requests also list their check runs (fetched for up to five open pull requests, or only the current branch's without a token).

Every refresh is a conditional request: unchanged resources answer `304 Not Modified`, which GitHub does not count against the rate limit when a token is present, and the ETags with their cached responses live in the plugin state directory so a restarted pane starts warm. Rows that changed in the last two minutes are highlighted in yellow, and the **ACTIVITY** section at the bottom lists the latest transitions (issue opened/closed/reopened, milestone created/closed, pull request opened/ready/reviewed/merged/closed, workflow run started/finished) with how long ago each was seen. The header shows the refresh age and, at 30+ columns or when it runs low, the remaining API budget. Set `HERDR_GITHUB_STATUS_DEBUG=1` to append request statuses to `debug.log` in the state directory.

The **ACTIONS** section lists the latest workflow runs (newest first) with a status icon (`◌` queued, `◐` running, `✓` success, `✗` failure, `⊘` cancelled, `→` skipped, `!` action required), the workflow name, the branch at 36+ columns, and elapsed time or duration. Runs that are queued or in progress also appear in NOW, and while a run is queued or in progress (and a token is present with budget to spare) the pane refreshes runs and check runs every 5 seconds on top of the 10-second full refresh.

### Configuration
Optional. Create `config.toml` in the directory printed by `herdr plugin config-dir jwanga.github-status`. Every option can be left out; the values below are the defaults.

```toml
poll_interval_secs = 10          # seconds between full refreshes (5–3600)
active_poll_interval_secs = 5    # seconds between run/check refreshes while a workflow run is active (2–3600)
width = "sidebar"                # "sidebar" = match herdr's left sidebar, or a column count (16–200)
auto_open = true                 # false: never dock by itself; use the toggle action
sections = ["now", "milestones", "issues", "pull_requests", "actions", "activity"]
                                 # shown in this order; leave a section out to hide it
recent_window_minutes = 2        # how long a changed row stays highlighted (0–1440)
recent_closed_hours = 24         # how long closed items stay in the "recently closed" groups (0–8760)
runs_limit = 15                  # workflow runs fetched and listed (1–100)
```

Options are read when a pane launches (and by each dock action and hook), so close and reopen the pane to apply a change. An invalid or unknown option never stops the pane: that option falls back to its default, a yellow `⚠` line appears under the header, and `?` lists every problem in full.
