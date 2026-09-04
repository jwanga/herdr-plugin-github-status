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

From GitHub (builds from source with `cargo` on install; prebuilt binaries arrive with the publishing milestone):

```sh
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

The tree shows open milestones with `closed/total` (and a progress bar when the pane is 32+ columns wide) and their issues (open first, with the assignee's initials when the pane is 30+ columns wide), a collapsed group of closed milestones, open issues with no milestone plus a collapsed group of issues closed in the last 24 hours, and open pull requests with the same review/checks tail; expanding a pull request shows its branch and the issues it closes, and a collapsed group lists pull requests merged or closed in the last 24 hours.

### Configuration
Documented as features land. Config lives in the directory printed by `herdr plugin config-dir jwanga.github-status`.
