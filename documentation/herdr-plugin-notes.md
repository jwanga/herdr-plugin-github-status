# herdr plugin notes (verified against herdr 0.8.0, protocol 19, 2026-09-04)

Sources: `herdr --help`, `herdr --skill`, `herdr --default-config`, `herdr api schema --output`,
https://herdr.dev/docs/plugins/, /docs/marketplace/, /docs/socket-api/, /docs/cli-reference/, and the
marketplace plugins persiyanov/herdr-reviewr, alexarthurs/herdr-sidebar, cloudmanic/herdr-plus,
ogulcancelik/herdr-plugin-examples.

## Manifest (`herdr-plugin.toml`)
Required: `id`, `name`, `version`, `min_herdr_version`. Optional: `description`, `platforms`
(`linux`, `macos`, `windows`). `id` allows ASCII letters, digits, `.`, `:`, `_`, `-` (≤120 chars).
Action/pane/link-handler ids are plugin-local and must not contain dots. Per-entry `platforms` gates
each entry; ids must still be unique across platform variants.

```toml
[[build]]      # only on `plugin install` (never on `plugin link`); cwd = plugin checkout
command = ["sh", "scripts/fetch-or-build.sh"]

[[startup]]    # once per enabled plugin after session restore / live handoff
command = ["..."]

[[actions]]
id = "toggle"; title = "..."; description = "..."; contexts = ["workspace", "pane"]
command = ["bash", "herdr/pane.sh", "toggle"]

[[events]]     # on = workspace.*, tab.*, pane.*, worktree.*, layout.updated ...
on = "tab.focused"; command = ["bash", "herdr/ensure.sh"]

[[panes]]
id = "status"; title = "status"; placement = "split"   # overlay|popup|split|tab|zoomed
command = ["sh", "-c", "exec \"$HERDR_PLUGIN_ROOT/bin/herdr-github-status\""]
# width/height fields exist (PopupSize: cells or "80%") but apply to popups.

[[link_handlers]]
id = "gh"; pattern = "^https://github\\.com/..."; action = "toggle"
```

Commands are argv arrays run **without a shell** and with a **minimal PATH**; prepend
`/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin` in scripts. Relative action/event commands resolve
from the plugin root; **pane commands resolve against the pane cwd**, so invoke the binary by
absolute path via `$HERDR_PLUGIN_ROOT`.

## Environment injected into plugin commands
`HERDR_BIN_PATH`, `HERDR_SOCKET_PATH`, `HERDR_ENV=1`, `HERDR_PLUGIN_ID`, `HERDR_PLUGIN_ROOT` (read-only
for managed installs), `HERDR_PLUGIN_CONFIG_DIR` (`~/.config/herdr/plugins/config/<id>`),
`HERDR_PLUGIN_STATE_DIR` (`~/.local/state/herdr/plugins/<id>`), `HERDR_PLUGIN_CONTEXT_JSON`
(workspace_id, workspace_cwd, workspace_label, tab_id, focused_pane_id, focused_pane_cwd (launch cwd,
not live), focused_pane_agent/status, selected_text, clicked_url, worktree{repo_root, checkout_path}),
`HERDR_PLUGIN_ACTION_ID`, `HERDR_PLUGIN_EVENT` + `HERDR_PLUGIN_EVENT_JSON` (hooks),
`HERDR_PLUGIN_ENTRYPOINT_ID` (panes). Every managed pane also gets `HERDR_WORKSPACE_ID`, `HERDR_TAB_ID`,
`HERDR_PANE_ID`.

`plugin action invoke` resolves context from the **focused** workspace regardless of caller pane.

## CLI surface used by this plugin
- `herdr plugin install owner/repo[/subdir] [--ref REF] [--yes]`, `plugin link <path> [--disabled]`,
  `plugin unlink <id>`, `plugin uninstall`, `plugin list [--json]`, `plugin enable|disable <id>`,
  `plugin action list|invoke <id> [--plugin ID]`, `plugin log list --plugin ID`,
  `plugin config-dir <id>`.
- `herdr plugin pane open --plugin ID --entrypoint ID --placement split --direction right
  --target-pane <pane> [--cwd P] [--env K=V] [--focus|--no-focus]` → `.result.plugin_pane.pane.pane_id`.
  A split/zoomed open **requires `--target-pane`**. No ratio/width control for splits.
- `herdr plugin pane close <pane_id>` only knows panes opened in this server session; use plain
  `herdr pane close <pane_id>` to sweep.
- `herdr pane split <pane> --direction right --ratio R --cwd P --env K=V --no-focus` →
  `.result.pane.pane_id`. **Ratio = first pane's share**: width 215 with ratio 0.8 → 172 | 43. For an
  N-column right pane use `R = 1 - N/W` where W is the target pane's width from `pane layout`.
- `herdr pane layout --pane <id>` → `.result.layout.{area, panes[{pane_id, rect{x,y,width,height}}],
  splits[{id, direction, ratio, rect}], focused_pane_id, zoomed}`.
- `herdr pane resize --pane <id> --direction left|right|up|down [--amount FLOAT]`;
  socket `layout.set_split_ratio {tab_id|pane_id, path:[bool], ratio}`.
- `herdr pane list [--workspace W]` → panes with `pane_id, tab_id, workspace_id, cwd, foreground_cwd,
  agent, agent_status, terminal_title, focused, revision`.
- `herdr pane process-info --pane <id>` → foreground processes `{pid,name,argv0,argv,cwd}`; identify
  our pane by `argv0`/`argv[0]` basename (`name` is a rewritable title).
- `herdr pane rename <id> <label>`, `herdr pane run <id> <cmd>`, `herdr pane zoom <id> --on|--off`
  (a zoom on/off cycle is the deterministic way to focus a pane by id), `herdr pane focus --direction`.
- `herdr agent list` → agents `{pane_id, tab_id, workspace_id, agent, agent_status, cwd, name,...}`;
  states `idle|working|blocked|done|unknown`.
- `herdr workspace list|get`, `herdr workspace report-metadata <ws> --source ID --token k=v`
  (renders as `$k` in sidebar space rows), `herdr pane report-metadata`.
- `herdr notification show <title> [--body] [--position] [--sound none|done|request]`.
- Errors: JSON on stderr, exit 1 (`{"error":{"code":"pane_not_found",...}}`); syntax errors exit 2.

## Socket API
NDJSON over `~/.config/herdr/herdr.sock`: `{"id":"r1","method":"pane.list","params":{}}`. Methods
include `session.snapshot`, `events.subscribe` (subscriptions: `workspace.*`, `tab.*`, `pane.*`,
`worktree.*`, `pane.agent_status_changed`, `pane.output_matched`), `events.wait`,
`layout.set_split_ratio`, `pane.report_metadata`. Full schema: `herdr api schema --output f.json`.

## Sidebar width
`~/.config/herdr/session.json` → `sidebar_width` (26 by default); config `[ui] sidebar_width = 26`,
`sidebar_min_width = 18`, `sidebar_max_width = 36`. The sidebar occupies x = 0..26 and pane area
starts at x = 26.

## Docked-sidebar pattern (from herdr-sidebar / reviewr)
- Open: `pane split <focused> --direction right --ratio R --no-focus --cwd <live cwd>
  --env PATH=<bin dir>:$PATH`, then `pane run <new> <binary>` and `pane rename <new> <label>`;
  or `plugin pane open` + resize. Focus by zoom on/off if desired.
- Ensure hook on `tab.focused`, `tab.created`, `workspace.created`, `pane.focused`: idempotent, uses an
  atomic `mkdir` lock in `$TMPDIR`, snooze marker per tab when the user closed the pane, never steals
  focus. Detect an existing pane via `pane process-info` (argv0) rather than labels.
- Close gracefully: send `ctrl+q`, wait for exit, then `pane close`.

## Distribution
- Marketplace = public GitHub repos with topic `herdr-plugin` and a `herdr-plugin.toml` on the default
  branch (root or subdir); refreshed every 30 min; forks/archived excluded. Listing shows manifest
  `name`, `version`, `platforms`, `min_herdr_version`.
- Convention: `[[build]]` downloads a release asset named `<bin>-<target-triple>` (or `.tar.gz`) plus a
  SHA-256 sidecar from `https://github.com/<repo>/releases/download/v<version>/`, verifies, installs to
  `bin/`; falls back to `cargo build --release`. Release tag == manifest version.
- Users update by `plugin uninstall` + `plugin install`; config dir is keyed by plugin id and survives.
- Keybinding in **user** config: `[[keys.command]] key="prefix+g" type="plugin_action"
  command="<plugin_id>.<action_id>"`.
