//! Background fetch loop: follows the workspace's live working directory on a short tick,
//! resolves the GitHub repository, fetches a snapshot when the repository changes, on an
//! interval, or on demand, and hands results to the UI over a channel.

use crate::github::{self, Client, RateLimited};
use crate::herdr;
use crate::model::{AgentInfo, Snapshot};
use crate::repo::{self, RepoRef};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime};

/// How often the followed directory is re-checked.
pub const CWD_TICK: Duration = Duration::from_secs(2);

pub enum Cmd {
    Refresh,
    Quit,
}

pub enum Msg {
    /// A fresh snapshot for the detected repository.
    Snapshot(Box<Snapshot>),
    /// The followed directory is not a GitHub checkout.
    NoRepo(String),
    /// A fetch for `repo` failed; a previous snapshot of the same repo may stay on screen.
    Error { repo: RepoRef, message: String },
    /// The herdr agents in this workspace changed.
    Agents(Vec<AgentInfo>),
}

/// herdr agents in the pane's workspace: empty outside herdr, `None` when the listing
/// failed transiently (the caller keeps the previous list).
pub fn workspace_agents() -> Option<Vec<AgentInfo>> {
    let Ok(ws) = std::env::var("HERDR_WORKSPACE_ID") else { return Some(Vec::new()) };
    let agents = herdr::agent_list().ok()?;
    Some(
        agents
            .into_iter()
            .filter(|a| a.workspace_id == ws)
            .map(|a| AgentInfo { pane_id: a.pane_id, agent: a.agent, status: a.agent_status, title: a.terminal_title_stripped })
            .collect(),
    )
}

/// The directory the pane should describe: the live cwd of the workspace's focused pane
/// (never the status pane itself), preferring the pane's own tab, else `fallback`.
pub fn live_cwd(fallback: &str) -> String {
    let (Ok(ws), Ok(me)) = (std::env::var("HERDR_WORKSPACE_ID"), std::env::var("HERDR_PANE_ID")) else {
        return fallback.to_string();
    };
    let tab = std::env::var("HERDR_TAB_ID").ok();
    let Ok(panes) = herdr::pane_list(Some(&ws)) else {
        return fallback.to_string();
    };
    let in_tab = |p: &herdr::Pane| tab.as_deref().is_none_or(|t| p.tab_id == t);
    panes
        .iter()
        .filter(|p| p.pane_id != me)
        .min_by_key(|p| (!p.focused, !in_tab(p)))
        .and_then(herdr::Pane::live_cwd)
        .unwrap_or_else(|| fallback.to_string())
}

/// Consume every queued command; `true` if a `Quit` was among them.
fn drain(cmd_rx: &Receiver<Cmd>) -> bool {
    let mut quit = false;
    while let Ok(c) = cmd_rx.try_recv() {
        quit |= matches!(c, Cmd::Quit);
    }
    quit
}

pub fn spawn(fallback_cwd: String, interval: Duration) -> (Sender<Cmd>, Receiver<Msg>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
    let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
    std::thread::spawn(move || {
        let mut client = Client::new(github::discover_token());
        let mut current: Option<RepoRef> = None;
        let mut last_fetch: Option<Instant> = None;
        let mut blocked_until: Option<SystemTime> = None;
        let mut want_fetch = true;
        let mut agents: Option<Vec<AgentInfo>> = None;
        loop {
            if let Some(now_agents) = workspace_agents() {
                if agents.as_ref() != Some(&now_agents) {
                    agents = Some(now_agents.clone());
                    if msg_tx.send(Msg::Agents(now_agents)).is_err() {
                        break;
                    }
                }
            }
            let cwd = live_cwd(&fallback_cwd);
            let detected = repo::detect(&cwd);
            if detected != current {
                current = detected.clone();
                want_fetch = true;
                if current.is_none() && msg_tx.send(Msg::NoRepo(cwd.clone())).is_err() {
                    break;
                }
            }
            let due = last_fetch.is_none_or(|t| t.elapsed() >= interval);
            let blocked = blocked_until.is_some_and(|u| SystemTime::now() < u);
            if let Some(r) = &current {
                if (want_fetch || due) && !blocked {
                    let msg = match client.fetch_snapshot(r) {
                        Ok(s) => Msg::Snapshot(Box::new(s)),
                        Err(e) => {
                            if let Some(rl) = e.downcast_ref::<RateLimited>() {
                                blocked_until = Some(rl.until);
                            }
                            Msg::Error { repo: r.clone(), message: format!("{e:#}") }
                        }
                    };
                    last_fetch = Some(Instant::now());
                    want_fetch = false;
                    if msg_tx.send(msg).is_err() {
                        break;
                    }
                }
            }
            match cmd_rx.recv_timeout(CWD_TICK) {
                Ok(Cmd::Refresh) => {
                    want_fetch = true;
                    if drain(&cmd_rx) {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Ok(Cmd::Quit) | Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    (cmd_tx, msg_rx)
}
