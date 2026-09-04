//! Docking: open / close / toggle the status pane on the right edge of the current tab,
//! sized to herdr's sidebar width.

use crate::herdr::{self, Layout, Pane};
use crate::{BIN_NAME, PANE_ENTRYPOINT, PANE_LABEL, PLUGIN_ID};
use anyhow::{anyhow, bail, Context, Result};
use std::path::Path;

pub const DEFAULT_WIDTH: u32 = 26;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Toggle,
    Open,
    Close,
}

impl std::str::FromStr for Mode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "toggle" => Ok(Mode::Toggle),
            "open" => Ok(Mode::Open),
            "close" => Ok(Mode::Close),
            other => bail!("unknown dock mode '{other}' (toggle | open | close)"),
        }
    }
}

/// Column width for the pane: herdr's live sidebar width from `session.json`, else
/// herdr's default of 26. (`config.toml`'s `sidebar_width` is only a starting value that
/// herdr auto-scales, so the session file is the one source of truth while herdr runs.)
pub fn sidebar_width() -> u32 {
    std::fs::read_to_string(herdr_config_dir().join("session.json"))
        .ok()
        .and_then(|s| parse_session_width(&s))
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

/// The pane to split so the status pane becomes a full-height right column: the
/// rightmost full-height pane, preferring the focused pane on ties; else the focused pane.
pub fn open_target(layout: &Layout) -> Result<String> {
    let right_edge = layout.area.right();
    let mut candidates: Vec<_> = layout
        .panes
        .iter()
        .filter(|p| p.rect.right() == right_edge)
        .collect();
    candidates.sort_by(|a, b| {
        b.rect
            .height
            .cmp(&a.rect.height)
            .then_with(|| b.focused.cmp(&a.focused))
    });
    candidates
        .first()
        .copied()
        .or_else(|| layout.panes.iter().find(|p| p.focused))
        .or_else(|| layout.panes.first())
        .map(|p| p.pane_id.clone())
        .ok_or_else(|| anyhow!("layout has no panes"))
}

/// Ratio of the *left* pane in a right split of a `total`-column region such that the
/// right pane gets `right` columns (herdr gives the first pane floor(total * ratio)).
pub fn ratio_for_right_width(total: u32, right: u32) -> f64 {
    if total == 0 {
        return 0.5;
    }
    let right = right.min(total.saturating_sub(10)).max(1);
    // Nudge up slightly so floor() lands on exactly `total - right` columns.
    1.0 - f64::from(right) / f64::from(total) + 0.0005
}

/// The immediate horizontal split containing `pane` and its left neighbour, as
/// (left pane, total columns, current ratio).
fn parent_split(layout: &Layout, pane: &str) -> Result<(String, u32, f64)> {
    let me = layout
        .panes
        .iter()
        .find(|p| p.pane_id == pane)
        .ok_or_else(|| anyhow!("pane {pane} not in its own layout"))?;
    // Smallest right-split whose rect ends at our right edge and contains us.
    let split = layout
        .splits
        .iter()
        .filter(|s| s.direction == "right")
        .filter(|s| s.rect.right() == me.rect.right() && s.rect.x < me.rect.x)
        .filter(|s| s.rect.y <= me.rect.y && me.rect.bottom() <= s.rect.bottom())
        .min_by_key(|s| s.rect.width * s.rect.height)
        .ok_or_else(|| anyhow!("no enclosing right split for pane {pane}"))?;
    let left = layout
        .panes
        .iter()
        .filter(|p| p.pane_id != pane && p.rect.right() == me.rect.x)
        .filter(|p| p.rect.y < me.rect.bottom() && me.rect.y < p.rect.bottom())
        .max_by_key(|p| p.rect.height)
        .ok_or_else(|| anyhow!("no left neighbour for pane {pane}"))?;
    Ok((left.pane_id.clone(), split.rect.width, split.ratio))
}

/// Resize `pane` (already opened as a right split) to exactly `width` columns, verifying
/// the result and nudging once more if herdr's rounding landed one column off.
pub fn snap_width(pane: &str, width: u32) -> Result<u32> {
    let mut layout = herdr::pane_layout(pane)?;
    for _ in 0..3 {
        let me_width = layout
            .panes
            .iter()
            .find(|p| p.pane_id == pane)
            .map(|p| p.rect.width)
            .ok_or_else(|| anyhow!("pane {pane} not in its own layout"))?;
        let (_, total, current) = parent_split(&layout, pane)?;
        let want = width.min(total.saturating_sub(10)).max(1);
        if me_width == want {
            return Ok(me_width);
        }
        let target = ratio_for_right_width(total, want);
        let mut delta = target - current;
        if delta.abs() < 0.0005 {
            // Ratio already "right" but the width is off: nudge by one column.
            delta = if me_width > want { 1.0 } else { -1.0 } / f64::from(total);
        }
        // Positive delta = our left edge moves right (pane narrows).
        let direction = if delta > 0.0 { "right" } else { "left" };
        if !herdr::pane_resize(pane, direction, delta.abs())? {
            bail!("herdr refused to resize {pane}");
        }
        layout = herdr::pane_layout(pane)?;
    }
    let me_width = layout
        .panes
        .iter()
        .find(|p| p.pane_id == pane)
        .map(|p| p.rect.width)
        .unwrap_or(0);
    if me_width == width {
        Ok(me_width)
    } else {
        bail!("pane {pane} is {me_width} columns wide after resizing (wanted {width})")
    }
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

pub fn run(mode: Mode) -> Result<()> {
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
    let in_tab: Vec<Pane> = existing
        .iter()
        .filter(|p| tab_id.as_deref().is_none_or(|t| p.tab_id == t))
        .cloned()
        .collect();

    match mode {
        Mode::Close => {
            if existing.is_empty() {
                println!("close: nothing open in {ws}");
                return Ok(());
            }
            close_all(&existing)
        }
        Mode::Toggle if !in_tab.is_empty() => close_all(&in_tab),
        Mode::Open if !in_tab.is_empty() => {
            let pane = &in_tab[0].pane_id;
            herdr::pane_focus(pane).ok();
            println!("open: already open ({pane}) in {ws}");
            Ok(())
        }
        Mode::Toggle | Mode::Open => open(&ctx, focused.as_ref(), &panes),
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
    let target = open_target(&layout)?;
    // Live cwd of the focused pane beats the launch cwd from the context.
    let cwd = focused
        .and_then(|p| p.foreground_cwd.clone().or_else(|| p.cwd.clone()))
        .or_else(|| ctx.focused_pane_cwd.clone())
        .or_else(|| ctx.workspace_cwd.clone());
    let plugin_id = std::env::var("HERDR_PLUGIN_ID").unwrap_or_else(|_| PLUGIN_ID.to_string());
    let pane = herdr::plugin_pane_open(&plugin_id, PANE_ENTRYPOINT, &target, cwd.as_deref(), true)
        .context("opening plugin pane")?;
    herdr::pane_rename(&pane, PANE_LABEL).ok();
    match snap_width(&pane, width) {
        Ok(cols) => println!("opened {pane} ({cols} cols, split of {target})"),
        Err(err) => eprintln!("{BIN_NAME}: opened {pane} but could not snap width: {err:#}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::{parse_layout, LayoutPane, Rect};

    fn pane(id: &str, x: u32, y: u32, w: u32, h: u32, focused: bool) -> LayoutPane {
        LayoutPane { pane_id: id.into(), focused, rect: Rect { x, y, width: w, height: h } }
    }

    fn layout(panes: Vec<LayoutPane>) -> Layout {
        Layout { area: Rect { x: 26, y: 1, width: 215, height: 93 }, panes, splits: vec![], zoomed: false }
    }

    /// A real `herdr pane layout --pane w7:p1` envelope (herdr 0.8.0): 215 columns split 0.8.
    const LAYOUT_JSON: &str = r#"{"id":"cli:pane:layout","result":{"layout":{"area":{"height":93,"width":215,"x":26,"y":1},"focused_pane_id":"w7:p1","panes":[{"focused":true,"pane_id":"w7:p1","rect":{"height":93,"width":172,"x":26,"y":1}},{"focused":false,"pane_id":"w7:p3","rect":{"height":93,"width":43,"x":198,"y":1}}],"splits":[{"direction":"right","id":"split_0_root","ratio":0.8,"rect":{"height":93,"width":215,"x":26,"y":1}}],"tab_id":"w7:t1","workspace_id":"w7","zoomed":false},"type":"pane_layout"}}"#;

    #[test]
    fn mode_parses() {
        assert_eq!("toggle".parse::<Mode>().unwrap(), Mode::Toggle);
        assert_eq!("open".parse::<Mode>().unwrap(), Mode::Open);
        assert_eq!("close".parse::<Mode>().unwrap(), Mode::Close);
        assert!("nope".parse::<Mode>().is_err());
    }

    #[test]
    fn session_width_parses() {
        assert_eq!(parse_session_width(r#"{"sidebar_width": 26}"#), Some(26));
        assert_eq!(parse_session_width(r#"{"sidebar_width": 0}"#), None);
        assert_eq!(parse_session_width("{}"), None);
        assert_eq!(parse_session_width("nope"), None);
    }

    #[test]
    fn ratio_gives_exact_right_width() {
        for total in [80u32, 120, 215, 300, 515, 600, 800, 1200] {
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
    fn open_target_prefers_rightmost_full_height_pane() {
        let l = layout(vec![
            pane("w1:p1", 26, 1, 108, 93, true),
            pane("w1:p2", 134, 1, 107, 46, false),
            pane("w1:p3", 134, 47, 107, 47, false),
        ]);
        // No full-height pane on the right edge: the tallest right-edge pane wins.
        assert_eq!(open_target(&l).unwrap(), "w1:p3");
        let single = layout(vec![pane("w1:p1", 26, 1, 215, 93, true)]);
        assert_eq!(open_target(&single).unwrap(), "w1:p1");
        // Ties on height go to the focused pane.
        let tie = layout(vec![pane("w1:p1", 26, 1, 100, 93, false), pane("w1:p2", 126, 1, 115, 93, true)]);
        assert_eq!(open_target(&tie).unwrap(), "w1:p2");
    }

    #[test]
    fn real_layout_json_parses_and_yields_split_ratio() {
        let envelope: serde_json::Value = serde_json::from_str(LAYOUT_JSON).unwrap();
        let l = parse_layout(&envelope["result"]).unwrap();
        assert_eq!(l.panes.len(), 2);
        assert_eq!(open_target(&l).unwrap(), "w7:p3");
        let (left, total, ratio) = parent_split(&l, "w7:p3").unwrap();
        assert_eq!(left, "w7:p1");
        assert_eq!(total, 215);
        assert!((ratio - 0.8).abs() < 1e-9);
        // Going from 43 to 26 columns needs the split moved right by ~0.08.
        let delta = ratio_for_right_width(total, 26) - ratio;
        assert!(delta > 0.0 && ((215.0 * (ratio + delta)).floor() as u32) == 189);
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
