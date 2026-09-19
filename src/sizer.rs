//! Keeps the status pane at its column width when the terminal resizes. herdr stores
//! splits as ratios, so a wider terminal would otherwise widen the pane. A resize the
//! user makes by hand (same surrounding width, different pane width) is adopted as the
//! new width instead of being fought.

use crate::dock;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

/// Resize events arrive in bursts while a window is dragged; act once they settle.
const DEBOUNCE: Duration = Duration::from_millis(120);
/// Width changes we cause ourselves are not the user dragging the edge: `dock open` snaps
/// the width in a few steps right after launch, and so does every snap made here.
const SELF_RESIZE_QUIET: Duration = Duration::from_secs(3);

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Keep,
    /// The surrounding split changed size: go back to this many columns.
    Snap(u32),
    /// The user dragged the pane edge: this is the width to hold from now on.
    Adopt(u32),
}

/// `last_total` / `total`: columns of the split holding the pane before and now;
/// `width`: the pane's current columns; `desired`: the columns it should have.
pub fn decide(last_total: Option<u32>, total: u32, width: u32, desired: u32) -> Decision {
    if width == dock::clamp_width(total, desired) {
        Decision::Keep
    } else if last_total != Some(total) {
        Decision::Snap(desired)
    } else {
        Decision::Adopt(width)
    }
}

/// Spawn the sizer for this process's own pane. Send `()` on every terminal resize.
/// `None` unless this is the plugin's pane: a hand-run binary must not resize the shell
/// pane it was started from.
pub fn spawn() -> Option<Sender<()>> {
    if !crate::is_plugin_pane() {
        return None;
    }
    let pane = std::env::var("HERDR_PANE_ID")
        .ok()
        .filter(|p| !p.is_empty())?;
    let (tx, rx) = mpsc::channel::<()>();
    std::thread::spawn(move || run(&pane, &rx));
    Some(tx)
}

fn run(pane: &str, rx: &Receiver<()>) {
    let mut quiet_until = Instant::now() + SELF_RESIZE_QUIET;
    let mut manual: Option<u32> = None;
    let mut last_total = dock::measure(pane).ok().map(|(total, _)| total);
    while rx.recv().is_ok() {
        // Coalesce the burst.
        while rx.recv_timeout(DEBOUNCE).is_ok() {}
        let Ok((total, width)) = dock::measure(pane) else {
            continue;
        };
        let desired = manual.unwrap_or_else(dock::sidebar_width);
        match decide(last_total, total, width, desired) {
            Decision::Keep => {}
            Decision::Snap(cols) => {
                let _ = dock::snap_width(pane, cols);
                quiet_until = Instant::now() + SELF_RESIZE_QUIET;
            }
            Decision::Adopt(_) if Instant::now() < quiet_until => {}
            Decision::Adopt(cols) => manual = Some(cols),
        }
        last_total = Some(total);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_resize_snaps_back() {
        // 260 → 200 columns: the ratio kept the pane at 10%, so it shrank to 20.
        assert_eq!(decide(Some(260), 200, 20, 26), Decision::Snap(26));
        assert_eq!(decide(None, 200, 20, 26), Decision::Snap(26));
    }

    #[test]
    fn manual_resize_is_adopted() {
        assert_eq!(decide(Some(260), 260, 42, 26), Decision::Adopt(42));
        // ...and then held across a terminal resize.
        assert_eq!(decide(Some(260), 300, 48, 42), Decision::Snap(42));
    }

    #[test]
    fn already_right_is_left_alone() {
        assert_eq!(decide(Some(260), 260, 26, 26), Decision::Keep);
        assert_eq!(decide(Some(260), 300, 26, 26), Decision::Keep);
        // A split too narrow for the full width counts as right at its clamped width.
        assert_eq!(decide(Some(260), 30, 20, 26), Decision::Keep);
    }
}
