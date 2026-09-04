//! herdr-github-status — a herdr plugin pane that shows a project's GitHub status.
//!
//! Invocations:
//! - `herdr-github-status`                 run the status TUI (the plugin pane entrypoint)
//! - `herdr-github-status dock <mode>`     open / close / toggle the pane (action entrypoint)
//! - `herdr-github-status sidebar-width`   print the column width the pane will use
//! - `herdr-github-status --version`

mod app;
mod dock;
mod github;
mod herdr;
mod model;
mod poll;
mod repo;
mod ui;
mod util;

use std::process::ExitCode;

pub const BIN_NAME: &str = "herdr-github-status";
pub const PLUGIN_ID: &str = "jwanga.github-status";
pub const PANE_ENTRYPOINT: &str = "status";
pub const PANE_LABEL: &str = "status";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None => app::run(),
        Some("--version") | Some("-V") => {
            println!("{} {}", BIN_NAME, env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("sidebar-width") => {
            println!("{}", dock::sidebar_width());
            Ok(())
        }
        Some("dock") => args
            .get(1)
            .map(String::as_str)
            .unwrap_or("toggle")
            .parse::<dock::Mode>()
            .and_then(dock::run),
        Some(other) => Err(anyhow::anyhow!(
            "unknown command '{other}'; use: dock <toggle|open|close>, sidebar-width, --version"
        )),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{BIN_NAME}: {err:#}");
            ExitCode::FAILURE
        }
    }
}
