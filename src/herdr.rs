//! Thin wrapper over the herdr CLI (the plugin API). Every call shells out to
//! `$HERDR_BIN_PATH` (falling back to `herdr` on PATH) and parses the JSON envelope.

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::process::Command;

pub fn bin() -> String {
    std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".to_string())
}

/// Run a herdr command and return the parsed `.result` object.
pub fn call(args: &[&str]) -> Result<serde_json::Value> {
    let output = Command::new(bin())
        .args(args)
        .output()
        .with_context(|| format!("failed to run {} {}", bin(), args.join(" ")))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = serde_json::from_str::<serde_json::Value>(stderr.trim())
            .ok()
            .and_then(|v| v.get("error").cloned())
            .map(|e| {
                format!(
                    "{}: {}",
                    e.get("code").and_then(|c| c.as_str()).unwrap_or("error"),
                    e.get("message").and_then(|m| m.as_str()).unwrap_or("")
                )
            })
            .unwrap_or_else(|| stderr.trim().to_string());
        bail!("herdr {} failed: {msg}", args.join(" "));
    }
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .with_context(|| format!("herdr {} returned non-JSON output", args.join(" ")))?;
    value
        .get("result")
        .cloned()
        .ok_or_else(|| anyhow!("herdr {} returned no result", args.join(" ")))
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pane {
    pub pane_id: String,
    pub tab_id: String,
    #[allow(dead_code)]
    pub workspace_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub foreground_cwd: Option<String>,
    #[serde(default)]
    pub focused: bool,
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
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LayoutPane {
    pub pane_id: String,
    pub focused: bool,
    pub rect: Rect,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Layout {
    pub area: Rect,
    #[allow(dead_code)]
    pub focused_pane_id: String,
    pub panes: Vec<LayoutPane>,
    #[serde(default)]
    pub zoomed: bool,
}

pub fn pane_layout(pane: &str) -> Result<Layout> {
    let result = call(&["pane", "layout", "--pane", pane])?;
    let layout = result
        .get("layout")
        .cloned()
        .ok_or_else(|| anyhow!("pane layout: missing layout"))?;
    Ok(serde_json::from_value(layout)?)
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
        Err(err) if err.to_string().contains("pane_not_found") => Ok(None),
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

/// Move a pane's edge by a ratio delta: `direction` is the direction the edge moves.
pub fn pane_resize(pane: &str, direction: &str, amount: f64) -> Result<()> {
    let amount = format!("{amount:.4}");
    call(&["pane", "resize", "--pane", pane, "--direction", direction, "--amount", &amount])?;
    Ok(())
}

pub fn pane_close(pane: &str) -> Result<()> {
    match call(&["pane", "close", pane]) {
        Ok(_) => Ok(()),
        Err(err) if err.to_string().contains("pane_not_found") => Ok(()),
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
