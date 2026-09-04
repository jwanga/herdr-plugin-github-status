//! Docking: open / close / toggle the status pane on the right edge of the current tab,
//! sized to herdr's sidebar width.

use crate::herdr::{self, Layout, Pane};
use crate::{BIN_NAME, PANE_ENTRYPOINT, PANE_LABEL, PLUGIN_ID};
use anyhow::{anyhow, bail, Context, Result};
use std::path::Path;

pub const DEFAULT_WIDTH: u32 = 26;

/// Column width for the pane: herdr's live sidebar width from `session.json`, else the
/// configured `[ui] sidebar_width`, else herdr's default of 26.
pub fn sidebar_width() -> u32 {
    let config_dir = herdr_config_dir();
    std::fs::read_to_string(config_dir.join("session.json"))
        .ok()
        .and_then(|s| parse_session_width(&s))
        .or_else(|| {
            std::fs::read_to_string(config_dir.join("config.toml"))
                .ok()
                .and_then(|s| parse_config_width(&s))
        })
        .unwrap_or(DEFAULT_WIDTH)
}

fn herdr_config_dir() -> std::path::PathBuf {
    if let Ok(sock) = std::env::var("HERDR_SOCKET_PATH") {
        if let Some(dir) = Path::new(&sock).parent() {
            return dir.to_path_buf();
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    Path::new(&home).join(".config").join("herdr")
}

pub fn parse_session_width(json: &str) -> Option<u32> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let width = value.get("sidebar_width")?.as_u64()?;
    u32::try_from(width).ok().filter(|w| *w > 0)
}

/// Minimal TOML scan for `sidebar_width = N` inside `[ui]` (herdr's config is small; a
/// full TOML parser is not worth a dependency for one integer).
pub fn parse_config_width(toml: &str) -> Option<u32> {
    let mut in_ui = false;
    for raw in toml.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            in_ui = line == "[ui]";
            continue;
        }
        if !in_ui {
            continue;
        }
        if let Some(rest) = line.strip_prefix("sidebar_width") {
            let rest = rest.trim_start();
            if let Some(v) = rest.strip_prefix('=') {
                if let Ok(n) = v.trim().parse::<u32>() {
                    if n > 0 {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}

/// Where to split and at what ratio so the new right pane is `width` columns.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenPlan {
    pub target: String,
    pub ratio: f64,
}

/// Pick the rightmost full-height pane (so the status pane becomes a full-height right
/// column), preferring the focused pane on ties; fall back to the focused pane.
pub fn open_plan(layout: &Layout, width: u32) -> Result<OpenPlan> {
    let right_edge = layout.area.x + layout.area.width;
    let mut candidates: Vec<_> = layout
        .panes
        .iter()
        .filter(|p| p.rect.x + p.rect.width == right_edge)
        .collect();
    candidates.sort_by(|a, b| {
        b.rect
            .height
            .cmp(&a.rect.height)
            .then_with(|| b.focused.cmp(&a.focused))
    });
    let target = candidates
        .first()
        .copied()
        .or_else(|| layout.panes.iter().find(|p| p.focused))
        .or_else(|| layout.panes.first())
        .ok_or_else(|| anyhow!("layout has no panes"))?;
    Ok(OpenPlan {
        target: target.pane_id.clone(),
        ratio: ratio_for_right_width(target.rect.width, width),
    })
}

/// Ratio of the *left* pane in a right split of a `total`-column pane such that the
/// right pane gets `right` columns (herdr gives the first pane floor(total * ratio)).
pub fn ratio_for_right_width(total: u32, right: u32) -> f64 {
    if total == 0 {
        return 0.5;
    }
    let right = right.min(total.saturating_sub(10)).max(1);
    let ratio = 1.0 - f64::from(right) / f64::from(total);
    // Nudge up slightly so floor() lands on exactly `total - right` columns.
    (ratio + 0.0005).clamp(0.05, 0.95)
}

/// Resize `pane` (already opened as a right split) to exactly `width` columns.
pub fn snap_width(pane: &str, width: u32) -> Result<()> {
    let layout = herdr::pane_layout(pane)?;
    let me = layout
        .panes
        .iter()
        .find(|p| p.pane_id == pane)
        .ok_or_else(|| anyhow!("pane {pane} not in its own layout"))?;
    // The left neighbour sharing our split: the pane whose right edge touches our left edge
    // on the same rows.
    let left = layout
        .panes
        .iter()
        .filter(|p| p.pane_id != pane && p.rect.x + p.rect.width == me.rect.x)
        .filter(|p| p.rect.y < me.rect.y + me.rect.height && me.rect.y < p.rect.y + p.rect.height)
        .max_by_key(|p| p.rect.height)
        .ok_or_else(|| anyhow!("no left neighbour for pane {pane}"))?;
    let total = left.rect.width + me.rect.width;
    let current = f64::from(left.rect.width) / f64::from(total);
    let target = ratio_for_right_width(total, width);
    let delta = target - current;
    if delta.abs() < 0.0005 {
        return Ok(());
    }
    // Positive delta = our left edge moves right (pane narrows).
    let direction = if delta > 0.0 { "right" } else { "left" };
    herdr::pane_resize(pane, direction, delta.abs())
}

/// Context herdr hands an action: the focused workspace/tab/pane.
#[derive(Debug, Default, Clone)]
pub struct ActionContext {
    pub workspace_id: Option<String>,
    pub tab_id: Option<String>,
    pub focused_pane_id: Option<String>,
    pub focused_pane_cwd: Option<String>,
    pub workspace_cwd: Option<String>,
}

pub fn action_context() -> ActionContext {
    let mut ctx = ActionContext::default();
    if let Ok(json) = std::env::var("HERDR_PLUGIN_CONTEXT_JSON") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
            let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
            ctx.workspace_id = s("workspace_id");
            ctx.tab_id = s("tab_id");
            ctx.focused_pane_id = s("focused_pane_id");
            ctx.focused_pane_cwd = s("focused_pane_cwd");
            ctx.workspace_cwd = s("workspace_cwd");
        }
    }
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    ctx.workspace_id = ctx.workspace_id.or_else(|| env("HERDR_WORKSPACE_ID"));
    ctx.tab_id = ctx.tab_id.or_else(|| env("HERDR_TAB_ID"));
    ctx.focused_pane_id = ctx.focused_pane_id.or_else(|| env("HERDR_PANE_ID"));
    ctx
}

fn is_status_process(argv0: Option<&str>, argv: &[String]) -> bool {
    let base = |s: &str| Path::new(s).file_name().and_then(|f| f.to_str()).map(str::to_string);
    let first = argv0.and_then(base).or_else(|| argv.first().and_then(|a| base(a)));
    // `dock` invocations are actions, never a status pane.
    first.as_deref() == Some(BIN_NAME) && argv.get(1).map(String::as_str) != Some("dock")
}

/// Panes in `panes` whose foreground process is the status TUI. Probes run concurrently.
pub fn find_status_panes(panes: &[Pane]) -> Result<Vec<Pane>> {
    let results: Vec<Result<bool>> = std::thread::scope(|scope| {
        let handles: Vec<_> = panes
            .iter()
            .map(|p| {
                scope.spawn(move || -> Result<bool> {
                    Ok(herdr::pane_processes(&p.pane_id)?
                        .map(|procs| {
                            procs
                                .iter()
                                .any(|pr| is_status_process(pr.argv0.as_deref(), &pr.argv))
                        })
                        .unwrap_or(false))
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| Err(anyhow!("probe thread panicked"))))
            .collect()
    });
    let mut found = Vec::new();
    for (pane, is_status) in panes.iter().zip(results) {
        if is_status? {
            found.push(pane.clone());
        }
    }
    Ok(found)
}

pub fn run(mode: &str) -> Result<()> {
    if !matches!(mode, "toggle" | "open" | "close") {
        bail!("unknown dock mode '{mode}' (toggle | open | close)");
    }
    let ctx = action_context();
    let ws = ctx
        .workspace_id
        .clone()
        .ok_or_else(|| anyhow!("no workspace context; invoke from inside herdr"))?;
    let panes = herdr::pane_list(Some(&ws)).context("listing workspace panes")?;
    let existing = find_status_panes(&panes).context("probing panes")?;

    // Resolve the tab the action targets: the context tab, else the focused pane's tab.
    let focused = ctx
        .focused_pane_id
        .as_deref()
        .and_then(|id| panes.iter().find(|p| p.pane_id == id))
        .or_else(|| panes.iter().find(|p| p.focused))
        .cloned();
    let tab_id = ctx
        .tab_id
        .clone()
        .or_else(|| focused.as_ref().map(|p| p.tab_id.clone()));
    let in_tab: Vec<&Pane> = existing
        .iter()
        .filter(|p| tab_id.as_deref().is_none_or(|t| p.tab_id == t))
        .collect();

    match mode {
        "close" => {
            if existing.is_empty() {
                println!("close: nothing open in {ws}");
                return Ok(());
            }
            close_all(&existing)
        }
        "toggle" if !in_tab.is_empty() => {
            let targets: Vec<Pane> = in_tab.into_iter().cloned().collect();
            close_all(&targets)
        }
        "open" if !in_tab.is_empty() => {
            let pane = &in_tab[0].pane_id;
            herdr::pane_focus(pane).ok();
            println!("open: already open ({pane}) in {ws}");
            Ok(())
        }
        _ => open(&ctx, focused.as_ref(), &panes),
    }
}

fn close_all(panes: &[Pane]) -> Result<()> {
    let mut closed = Vec::new();
    for p in panes {
        herdr::pane_close(&p.pane_id).with_context(|| format!("closing {}", p.pane_id))?;
        closed.push(p.pane_id.clone());
    }
    println!("closed {}", closed.join(" "));
    Ok(())
}

fn open(ctx: &ActionContext, focused: Option<&Pane>, panes: &[Pane]) -> Result<()> {
    let anchor = focused
        .map(|p| p.pane_id.clone())
        .or_else(|| panes.first().map(|p| p.pane_id.clone()))
        .ok_or_else(|| anyhow!("workspace has no panes to dock beside"))?;
    let layout = herdr::pane_layout(&anchor).context("reading tab layout")?;
    if layout.zoomed {
        bail!("the tab is zoomed; unzoom before opening the status pane");
    }
    let width = sidebar_width();
    let plan = open_plan(&layout, width)?;
    // Live cwd of the focused pane beats the launch cwd from the context.
    let cwd = focused
        .and_then(|p| p.foreground_cwd.clone().or_else(|| p.cwd.clone()))
        .or_else(|| ctx.focused_pane_cwd.clone())
        .or_else(|| ctx.workspace_cwd.clone());
    let plugin_id = std::env::var("HERDR_PLUGIN_ID").unwrap_or_else(|_| PLUGIN_ID.to_string());
    let pane = herdr::plugin_pane_open(&plugin_id, PANE_ENTRYPOINT, &plan.target, cwd.as_deref(), true)
        .context("opening plugin pane")?;
    herdr::pane_rename(&pane, PANE_LABEL).ok();
    if let Err(err) = snap_width(&pane, width) {
        eprintln!("{BIN_NAME}: opened {pane} but could not snap width: {err:#}");
    }
    println!("opened {pane} ({width} cols, split of {})", plan.target);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::{LayoutPane, Rect};

    fn pane(id: &str, x: u32, y: u32, w: u32, h: u32, focused: bool) -> LayoutPane {
        LayoutPane { pane_id: id.into(), focused, rect: Rect { x, y, width: w, height: h } }
    }

    #[test]
    fn session_width_parses() {
        assert_eq!(parse_session_width(r#"{"sidebar_width": 26}"#), Some(26));
        assert_eq!(parse_session_width(r#"{"sidebar_width": 0}"#), None);
        assert_eq!(parse_session_width("{}"), None);
        assert_eq!(parse_session_width("nope"), None);
    }

    #[test]
    fn config_width_parses_only_ui_section() {
        let toml = "[theme]\nsidebar_width = 99\n[ui]\n# sidebar_width = 30\nsidebar_width = 32 # cols\n";
        assert_eq!(parse_config_width(toml), Some(32));
        assert_eq!(parse_config_width("[ui]\n# sidebar_width = 30\n"), None);
    }

    #[test]
    fn ratio_gives_exact_right_width() {
        for total in [80u32, 120, 215, 300] {
            for right in [18u32, 26, 36] {
                let r = ratio_for_right_width(total, right);
                let left = (f64::from(total) * r).floor() as u32;
                assert_eq!(total - left, right, "total {total} right {right} ratio {r}");
            }
        }
        // Never leaves the left pane unusably narrow.
        let r = ratio_for_right_width(30, 26);
        assert!((f64::from(30) * r).floor() as u32 >= 10);
    }

    #[test]
    fn open_plan_prefers_rightmost_full_height_pane() {
        let layout = Layout {
            area: Rect { x: 26, y: 1, width: 215, height: 93 },
            focused_pane_id: "w1:p1".into(),
            zoomed: false,
            panes: vec![
                pane("w1:p1", 26, 1, 108, 93, true),
                pane("w1:p2", 134, 1, 107, 46, false),
                pane("w1:p3", 134, 47, 107, 47, false),
            ],
        };
        // No full-height pane on the right edge: the tallest right-edge pane wins.
        let plan = open_plan(&layout, 26).unwrap();
        assert_eq!(plan.target, "w1:p3");
        let single = Layout {
            area: Rect { x: 26, y: 1, width: 215, height: 93 },
            focused_pane_id: "w1:p1".into(),
            zoomed: false,
            panes: vec![pane("w1:p1", 26, 1, 215, 93, true)],
        };
        let plan = open_plan(&single, 26).unwrap();
        assert_eq!(plan.target, "w1:p1");
        assert_eq!((215.0 * plan.ratio).floor() as u32, 189);
    }

    #[test]
    fn status_process_detection() {
        let argv = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(is_status_process(Some("herdr-github-status"), &argv(&["/x/bin/herdr-github-status"])));
        assert!(is_status_process(None, &argv(&["/x/herdr-github-status"])));
        assert!(!is_status_process(Some("herdr-github-status"), &argv(&["herdr-github-status", "dock", "toggle"])));
        assert!(!is_status_process(Some("zsh"), &argv(&["-zsh"])));
        assert!(!is_status_process(None, &[]));
    }
}
