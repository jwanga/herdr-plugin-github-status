//! Small process helpers shared by the git and gh integrations.

use std::path::Path;
use std::process::Command;

/// Run `program args…` (optionally in `cwd`) and return its trimmed stdout, or `None` when
/// the command fails, is missing, or prints nothing.
pub fn stdout(program: &str, args: &[&str], cwd: Option<&Path>) -> Option<String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Open a URL in the user's browser without blocking; the child is reaped on a helper
/// thread so no zombie is left behind.
pub fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(not(target_os = "macos"))]
    let program = "xdg-open";
    let spawned = Command::new(program)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Ok(mut child) = spawned {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// Parse an RFC 3339 UTC timestamp like `2026-09-04T07:45:32Z` into Unix seconds.
/// Fractional seconds are ignored; offsets other than `Z` are applied.
pub fn parse_rfc3339(s: &str) -> Option<u64> {
    let s = s.trim();
    let (date, rest) = s.split_once('T')?;
    let mut d = date.split('-');
    let (y, m, day): (i64, u32, u32) = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    let (time, offset) = match rest.find(['Z', '+', '-']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "Z"),
    };
    let time = time.split('.').next()?;
    let mut t = time.split(':');
    let (hh, mm, ss): (u64, u64, u64) = (
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
        t.next().unwrap_or("0").parse().ok()?,
    );
    let offset_secs: i64 = if offset == "Z" || offset.is_empty() {
        0
    } else {
        let sign = if offset.starts_with('-') { -1 } else { 1 };
        let mut o = offset[1..].split(':');
        let oh: i64 = o.next()?.parse().ok()?;
        let om: i64 = o.next().unwrap_or("0").parse().ok()?;
        sign * (oh * 3600 + om * 60)
    };
    let days = days_from_civil(y, m, day);
    let secs = days * 86_400 + (hh * 3600 + mm * 60 + ss) as i64 - offset_secs;
    u64::try_from(secs).ok()
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::parse_rfc3339;

    #[test]
    fn parses_github_timestamps() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-09-04T07:45:32Z"), Some(1_788_507_932));
        assert_eq!(
            parse_rfc3339("2026-09-04T07:45:32.123Z"),
            Some(1_788_507_932)
        );
        assert_eq!(
            parse_rfc3339("2026-09-04T09:45:32+02:00"),
            Some(1_788_507_932)
        );
        assert_eq!(parse_rfc3339("garbage"), None);
    }
}
