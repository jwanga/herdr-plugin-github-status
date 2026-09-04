//! Thin wrapper over the herdr CLI (the plugin API). Every call shells out to
//! `$HERDR_BIN_PATH` (falling back to `herdr` on PATH) and parses the JSON envelope.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::process::Command;

pub fn bin() -> String {
    std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".to_string())
}

/// A failed herdr call: the `{"error":{"code","message"}}` envelope herdr prints on stderr.
#[derive(Debug)]
pub struct HerdrError {
    pub code: String,
    pub message: String,
    pub command: String,
}

impl std::fmt::Display for HerdrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "herdr {} failed: {}: {}", self.command, self.code, self.message)
    }
}

impl std::error::Error for HerdrError {}

pub fn is_not_found(err: &anyhow::Error) -> bool {
    err.downcast_ref::<HerdrError>()
        .is_some_and(|e| e.code == "pane_not_found")
}

/// Run a herdr command and return the parsed `.result` object.
pub fn call(args: &[&str]) -> Result<serde_json::Value> {
    let command = args.join(" ");
    let output = Command::new(bin())
        .args(args)
        .output()
        .with_context(|| format!("failed to run {} {command}", bin()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let envelope = serde_json::from_str::<serde_json::Value>(stderr.trim()).ok();
        let field = |k: &str| {
            envelope
                .as_ref()
                .and_then(|v| v.pointer(&format!("/error/{k}")))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        return Err(HerdrError {
            code: field("code").unwrap_or_else(|| "error".to_string()),
            message: field("message").unwrap_or_else(|| stderr.trim().to_string()),
            command,
        }
        .into());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .with_context(|| format!("herdr {command} returned non-JSON output"))?;
    value
        .get("result")
        .cloned()
        .ok_or_else(|| anyhow!("herdr {command} returned no result"))
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pane {
    pub pane_id: String,
    pub tab_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub foreground_cwd: Option<String>,
    #[serde(default)]
    pub focused: bool,
}

impl Pane {
    /// The pane's live working directory (foreground process), else its launch cwd.
    pub fn live_cwd(&self) -> Option<String> {
        self.foreground_cwd.clone().or_else(|| self.cwd.clone())
    }
}

pub fn pane_list(workspace: Option<&str>) -> Result<Vec<Pane>> {
    let mut args = vec!["pane", "list"];
    if let Some(ws) = workspace {
        args.extend(["--workspace", ws]);
    }
    let result = call(&args)?;
    let panes = result
        .get("panes")
        .cloned()
        .ok_or_else(|| anyhow!("pane list: missing panes"))?;
    Ok(serde_json::from_value(panes)?)
}

#[derive(Debug, Clone, Deserialize)]
pub struct Agent {
    pub pane_id: String,
    pub workspace_id: String,
    pub agent: String,
    pub agent_status: String,
    #[serde(default)]
    pub terminal_title_stripped: Option<String>,
}

pub fn agent_list() -> Result<Vec<Agent>> {
    let result = call(&["agent", "list"])?;
    let agents = result
        .get("agents")
        .cloned()
        .ok_or_else(|| anyhow!("agent list: missing agents"))?;
    Ok(serde_json::from_value(agents)?)
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn right(&self) -> u32 {
        self.x + self.width
    }
    pub fn bottom(&self) -> u32 {
        self.y + self.height
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LayoutPane {
    pub pane_id: String,
    #[serde(default)]
    pub focused: bool,
    pub rect: Rect,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LayoutSplit {
    pub direction: String,
    pub ratio: f64,
    pub rect: Rect,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Layout {
    pub area: Rect,
    pub panes: Vec<LayoutPane>,
    #[serde(default)]
    pub splits: Vec<LayoutSplit>,
    #[serde(default)]
    pub zoomed: bool,
}

pub fn parse_layout(result: &serde_json::Value) -> Result<Layout> {
    let layout = result
        .get("layout")
        .cloned()
        .ok_or_else(|| anyhow!("pane layout: missing layout"))?;
    Ok(serde_json::from_value(layout)?)
}

pub fn pane_layout(pane: &str) -> Result<Layout> {
    parse_layout(&call(&["pane", "layout", "--pane", pane])?)
}

#[derive(Debug, Clone, Deserialize)]
pub struct ForegroundProcess {
    #[serde(default)]
    pub argv0: Option<String>,
    #[serde(default)]
    pub argv: Vec<String>,
}

/// Foreground processes of a pane. `Ok(None)` when the pane is gone.
pub fn pane_processes(pane: &str) -> Result<Option<Vec<ForegroundProcess>>> {
    match call(&["pane", "process-info", "--pane", pane]) {
        Ok(result) => {
            let procs = result
                .pointer("/process_info/foreground_processes")
                .cloned()
                .ok_or_else(|| anyhow!("process-info: missing foreground_processes"))?;
            Ok(Some(serde_json::from_value(procs)?))
        }
        Err(err) if is_not_found(&err) => Ok(None),
        Err(err) => Err(err),
    }
}

/// Open the plugin's pane as a right split of `target`. Returns the new pane id.
pub fn plugin_pane_open(
    plugin_id: &str,
    entrypoint: &str,
    target: &str,
    cwd: Option<&str>,
    focus: bool,
) -> Result<String> {
    let mut args = vec![
        "plugin",
        "pane",
        "open",
        "--plugin",
        plugin_id,
        "--entrypoint",
        entrypoint,
        "--placement",
        "split",
        "--direction",
        "right",
        "--target-pane",
        target,
    ];
    if let Some(cwd) = cwd {
        args.extend(["--cwd", cwd]);
    }
    args.push(if focus { "--focus" } else { "--no-focus" });
    let result = call(&args)?;
    result
        .pointer("/plugin_pane/pane/pane_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("plugin pane open: missing pane id in response"))
}

/// Move a pane's edge by a ratio delta; `direction` is the direction the edge moves.
/// Returns whether herdr reports the layout changed.
pub fn pane_resize(pane: &str, direction: &str, amount: f64) -> Result<bool> {
    let amount = format!("{amount:.5}");
    let result = call(&["pane", "resize", "--pane", pane, "--direction", direction, "--amount", &amount])?;
    Ok(result
        .pointer("/resize/changed")
        .and_then(|v| v.as_bool())
        .unwrap_or(true))
}

pub fn pane_close(pane: &str) -> Result<()> {
    match call(&["pane", "close", pane]) {
        Ok(_) => Ok(()),
        Err(err) if is_not_found(&err) => Ok(()),
        Err(err) => Err(err),
    }
}

pub fn pane_rename(pane: &str, label: &str) -> Result<()> {
    call(&["pane", "rename", pane, label])?;
    Ok(())
}

/// herdr has no focus-by-id; a zoom on/off cycle focuses a pane deterministically.
pub fn pane_focus(pane: &str) -> Result<()> {
    call(&["pane", "zoom", pane, "--on"])?;
    call(&["pane", "zoom", pane, "--off"])?;
    Ok(())
}
