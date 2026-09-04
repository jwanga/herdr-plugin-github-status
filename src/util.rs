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
