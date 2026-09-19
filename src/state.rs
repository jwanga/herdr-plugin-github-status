//! Per-tab dock state shared by the auto-dock hook, the actions, and the TUI: which pane
//! was docked in a tab, which tabs the user snoozed, and a lock so concurrent hooks
//! never open two panes. Everything is a small file under the plugin state directory.

use crate::PLUGIN_ID;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A lock older than this belongs to a hook that died; take it over.
const STALE_LOCK: Duration = Duration::from_secs(10);

/// `HERDR_PLUGIN_STATE_DIR`, else herdr's default location for this plugin (so a hand-run
/// `pane.sh` shares state with herdr-launched commands).
pub fn dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("HERDR_PLUGIN_STATE_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    Path::new(&home)
        .join(".local/state/herdr/plugins")
        .join(PLUGIN_ID)
}

/// Tab ids look like `w7:t1`; keep file names to `[A-Za-z0-9_-]`.
fn file_name(tab: &str) -> String {
    tab.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub struct TabState {
    root: PathBuf,
}

impl TabState {
    pub fn open() -> Self {
        Self::at(dir())
    }

    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }

    fn path(&self, kind: &str, tab: &str) -> PathBuf {
        self.root.join(kind).join(file_name(tab))
    }

    fn write(&self, kind: &str, tab: &str, content: &str) {
        let path = self.path(kind, tab);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, content);
    }

    /// The pane last docked in `tab`, whether or not it still exists.
    pub fn docked(&self, tab: &str) -> Option<String> {
        std::fs::read_to_string(self.path("docked", tab))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    pub fn set_docked(&self, tab: &str, pane: &str) {
        self.write("docked", tab, pane);
    }

    /// Every docked record, as (record file, pane id).
    fn docked_records(&self) -> Vec<(PathBuf, String)> {
        let Ok(entries) = std::fs::read_dir(self.root.join("docked")) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|e| {
                let pane = std::fs::read_to_string(e.path()).ok()?;
                Some((e.path(), pane.trim().to_string()))
            })
            .collect()
    }

    /// Drop docked records whose pane is gone (herdr restarted without restoring it), so
    /// the auto-dock hook docks those tabs again. Snoozes are the user's and are kept.
    pub fn forget_missing(&self, live_panes: &[String]) -> usize {
        let mut dropped = 0;
        for (path, pane) in self.docked_records() {
            if !live_panes.contains(&pane) && std::fs::remove_file(path).is_ok() {
                dropped += 1;
            }
        }
        dropped
    }

    /// Drop snoozes of tabs that no longer exist. Returns how many were dropped.
    pub fn forget_closed_tabs(&self, live_tabs: &[String]) -> usize {
        let live: Vec<String> = live_tabs.iter().map(|t| file_name(t)).collect();
        let Ok(entries) = std::fs::read_dir(self.root.join("snoozed")) else {
            return 0;
        };
        entries
            .flatten()
            .filter(|e| !live.contains(&e.file_name().to_string_lossy().into_owned()))
            .filter(|e| std::fs::remove_file(e.path()).is_ok())
            .count()
    }

    pub fn snoozed(&self, tab: &str) -> bool {
        self.path("snoozed", tab).exists()
    }

    pub fn snooze(&self, tab: &str) {
        self.write("snoozed", tab, "");
    }

    pub fn unsnooze(&self, tab: &str) {
        let _ = std::fs::remove_file(self.path("snoozed", tab));
    }

    /// Atomic per-tab lock (`mkdir`). `None` when another command holds a fresh one.
    pub fn lock(&self, tab: &str) -> Option<Lock> {
        let path = self.path("lock", tab);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let owner = std::process::id().to_string();
        for _ in 0..2 {
            if std::fs::create_dir(&path).is_ok() {
                let _ = std::fs::write(path.join(OWNER_FILE), &owner);
                return Some(Lock { path, owner });
            }
            if !is_stale(&path) {
                return None;
            }
            // Take a dead command's lock over by renaming it away: only one contender's
            // rename succeeds, so two can never both clear it and both lock.
            let grave = path.with_extension(format!("stale-{owner}"));
            if std::fs::rename(&path, &grave).is_err() {
                return None;
            }
            let _ = std::fs::remove_dir_all(&grave);
        }
        None
    }
}

const OWNER_FILE: &str = "owner";

fn is_stale(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age > STALE_LOCK)
}

pub struct Lock {
    path: PathBuf,
    owner: String,
}

impl Drop for Lock {
    /// Only remove a lock that is still ours: a command that outlived `STALE_LOCK` may
    /// have had it taken over.
    fn drop(&mut self) {
        let ours =
            std::fs::read_to_string(self.path.join(OWNER_FILE)).is_ok_and(|o| o == self.owner);
        if ours {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> TabState {
        let root = std::env::temp_dir().join(format!(
            "herdr-github-status-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        TabState::at(root)
    }

    #[test]
    fn docked_and_snooze_round_trip() {
        let s = temp("roundtrip");
        assert_eq!(s.docked("w7:t1"), None);
        assert!(!s.snoozed("w7:t1"));
        s.set_docked("w7:t1", "w7:pV");
        s.snooze("w7:t1");
        assert_eq!(s.docked("w7:t1").as_deref(), Some("w7:pV"));
        assert!(s.snoozed("w7:t1"));
        assert!(!s.snoozed("w7:t2"));
        s.unsnooze("w7:t1");
        assert!(!s.snoozed("w7:t1"));
    }

    #[test]
    fn forget_missing_keeps_live_panes_and_snoozes() {
        let s = temp("forget");
        s.set_docked("w1:t1", "w1:p2");
        s.set_docked("w2:t1", "w2:p9");
        s.snooze("w2:t1");
        assert_eq!(s.forget_missing(&["w1:p2".to_string()]), 1);
        assert_eq!(s.docked("w1:t1").as_deref(), Some("w1:p2"));
        assert_eq!(s.docked("w2:t1"), None);
        assert!(s.snoozed("w2:t1"));
    }

    #[test]
    fn forget_closed_tabs_drops_only_dead_snoozes() {
        let s = temp("closed-tabs");
        s.snooze("w1:t1");
        s.snooze("w1:t2");
        assert_eq!(s.forget_closed_tabs(&["w1:t1".to_string()]), 1);
        assert!(s.snoozed("w1:t1"));
        assert!(!s.snoozed("w1:t2"));
    }

    #[test]
    fn lock_is_exclusive_until_dropped() {
        let s = temp("lock");
        let held = s.lock("w7:t1").expect("first lock");
        assert!(s.lock("w7:t1").is_none());
        assert!(s.lock("w7:t2").is_some());
        drop(held);
        assert!(s.lock("w7:t1").is_some());
    }

    #[test]
    fn a_taken_over_lock_is_not_removed_by_its_old_owner() {
        let s = temp("takeover");
        let old = s.lock("w7:t1").expect("lock");
        // Simulate a takeover by another process.
        std::fs::write(s.path("lock", "w7:t1").join(OWNER_FILE), "someone-else").unwrap();
        drop(old);
        assert!(s.lock("w7:t1").is_none());
    }

    #[test]
    fn file_names_are_sanitized() {
        assert_eq!(file_name("w7:t1"), "w7_t1");
        assert_eq!(file_name("../x"), "___x");
    }
}
