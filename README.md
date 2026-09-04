# herdr-plugin-github-status
A [herdr](https://herdr.dev) plugin that docks a real-time **status** pane on the right edge of the screen. It shows, at a glance, where a project is: GitHub milestones and their issues, open and closed issues and pull requests, what is actively being worked on, and the live state of GitHub Actions workflow runs and check runs.

## Table of Contents
- [Installation](#installation)
- [Usage](#usage)
  - [Opening the pane](#opening-the-pane)
  - [Keys](#keys)
  - [Configuration](#configuration)

## Installation
Requires herdr ≥ 0.8.0 and an authenticated GitHub CLI (`gh auth login`), or a `GH_TOKEN` / `GITHUB_TOKEN` environment variable.

From GitHub (prebuilt binary when available, source build otherwise):

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

The pane docks on the right of the current tab at the same width as herdr's left sidebar (read live from herdr's `session.json`, default 26 columns). `toggle` closes an open status pane in the tab; `open` focuses an existing one; `close` closes every status pane in the workspace. Note that `herdr plugin action invoke` targets the *focused* workspace, wherever you run it.

### Keys
| Key | Action |
| --- | --- |
| `q` / `Ctrl+C` | Close the pane |

More keys land with each feature; press `?` inside the pane for the live list.

### Configuration
Documented as features land. Config lives in the directory printed by `herdr plugin config-dir jwanga.github-status`.
