//! Background fetch loop: follows the workspace's live working directory, resolves the
//! GitHub repository, fetches a snapshot on an interval or on demand, and hands results to
//! the UI over a channel.

use crate::github::{self, Client};
use crate::herdr;
use crate::model::Snapshot;
use crate::repo;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

pub enum Cmd {
    Refresh,
    Quit,
}

pub enum Msg {
    /// A fresh snapshot for the detected repository.
    Snapshot(Box<Snapshot>),
    /// The followed directory is not a GitHub checkout.
    NoRepo(String),
    /// A fetch failed; the previous snapshot stays on screen.
    Error(String),
}

/// The directory the pane should describe: the live cwd of the workspace's focused pane
/// (never the status pane itself), else `fallback`.
pub fn live_cwd(fallback: &str) -> String {
    let (Ok(ws), Ok(me)) = (std::env::var("HERDR_WORKSPACE_ID"), std::env::var("HERDR_PANE_ID")) else {
        return fallback.to_string();
    };
    let tab = std::env::var("HERDR_TAB_ID").ok();
    let Ok(panes) = herdr::pane_list(Some(&ws)) else {
        return fallback.to_string();
    };
    let others: Vec<_> = panes.iter().filter(|p| p.pane_id != me).collect();
    let pick = others
        .iter()
        .find(|p| p.focused && tab.as_deref().is_none_or(|t| p.tab_id == t))
        .or_else(|| others.iter().find(|p| p.focused))
        .or_else(|| others.iter().find(|p| tab.as_deref().is_none_or(|t| p.tab_id == t)))
        .or_else(|| others.first());
    pick.and_then(|p| p.foreground_cwd.clone().or_else(|| p.cwd.clone()))
        .unwrap_or_else(|| fallback.to_string())
}

pub fn spawn(fallback_cwd: String, interval: Duration) -> (Sender<Cmd>, Receiver<Msg>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    std::thread::spawn(move || {
        let mut client = Client::new(github::discover_token());
        loop {
            let cwd = live_cwd(&fallback_cwd);
            let msg = match repo::detect(&cwd) {
                None => Msg::NoRepo(cwd),
                Some(r) => match client.fetch_snapshot(&r) {
                    Ok(s) => Msg::Snapshot(Box::new(s)),
                    Err(e) => Msg::Error(format!("{e:#}")),
                },
            };
            if msg_tx.send(msg).is_err() {
                break;
            }
            match cmd_rx.recv_timeout(interval) {
                Ok(Cmd::Refresh) | Err(RecvTimeoutError::Timeout) => continue,
                Ok(Cmd::Quit) | Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    (cmd_tx, msg_rx)
}
