//! Resolve the GitHub repository (owner/name) and branch for a working directory.

use crate::util;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRef {
    pub owner: String,
    pub name: String,
    pub branch: Option<String>,
    pub root: String,
}

impl RepoRef {
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

fn git(cwd: &Path, args: &[&str]) -> Option<String> {
    // `-C` rather than `current_dir`: a nonexistent cwd then fails inside git (None) instead
    // of erroring on spawn.
    let mut full = vec!["-C", cwd.to_str()?];
    full.extend_from_slice(args);
    util::stdout("git", &full, None)
}

/// Parse a GitHub remote URL in ssh, scp-like, https, or bare `github.com/o/r` form.
pub fn parse_github_remote(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("ssh://github.com/"))
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("git://github.com/"))
        .or_else(|| url.strip_prefix("github.com/"))?;
    let rest = rest.trim_start_matches('/');
    let mut parts = rest.split('/');
    let owner = parts.next()?.trim();
    let name = parts.next()?.trim();
    let name = name
        .strip_suffix(".git")
        .unwrap_or(name)
        .trim_end_matches('/');
    if owner.is_empty() || name.is_empty() {
        return None;
    }
    Some((owner.to_string(), name.to_string()))
}

/// Pick the GitHub remote: `origin` first, then any other remote pointing at github.com.
pub fn pick_remote(remotes: &[(String, String)]) -> Option<(String, String)> {
    remotes
        .iter()
        .find(|(n, _)| n == "origin")
        .and_then(|(_, u)| parse_github_remote(u))
        .or_else(|| remotes.iter().find_map(|(_, u)| parse_github_remote(u)))
}

/// Detect the repository for `cwd`, or `None` when it is not inside a git checkout with a
/// GitHub remote.
pub fn detect(cwd: &str) -> Option<RepoRef> {
    let cwd = Path::new(cwd);
    let root = git(cwd, &["rev-parse", "--show-toplevel"])?;
    let remotes: Vec<(String, String)> = git(cwd, &["remote", "-v"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.to_string(), it.next()?.to_string()))
        })
        .collect();
    let (owner, name) = pick_remote(&remotes)?;
    let branch = git(cwd, &["branch", "--show-current"]);
    Some(RepoRef {
        owner,
        name,
        branch,
        root,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_remote_forms() {
        let want = Some((
            "jwanga".to_string(),
            "herdr-plugin-github-status".to_string(),
        ));
        for url in [
            "git@github.com:jwanga/herdr-plugin-github-status.git",
            "git@github.com:jwanga/herdr-plugin-github-status",
            "ssh://git@github.com/jwanga/herdr-plugin-github-status.git",
            "https://github.com/jwanga/herdr-plugin-github-status.git",
            "https://github.com/jwanga/herdr-plugin-github-status",
            "https://github.com/jwanga/herdr-plugin-github-status/",
            "git://github.com/jwanga/herdr-plugin-github-status.git",
            "github.com/jwanga/herdr-plugin-github-status",
        ] {
            assert_eq!(parse_github_remote(url), want, "{url}");
        }
        assert_eq!(parse_github_remote("git@gitlab.com:a/b.git"), None);
        assert_eq!(parse_github_remote("https://github.com/only-owner"), None);
        assert_eq!(parse_github_remote(""), None);
    }

    #[test]
    fn prefers_origin_then_any_github_remote() {
        let remotes = vec![
            (
                "upstream".to_string(),
                "https://github.com/up/stream.git".to_string(),
            ),
            (
                "origin".to_string(),
                "git@github.com:me/mine.git".to_string(),
            ),
        ];
        assert_eq!(pick_remote(&remotes), Some(("me".into(), "mine".into())));
        let remotes = vec![
            (
                "origin".to_string(),
                "git@gitlab.com:me/mine.git".to_string(),
            ),
            ("gh".to_string(), "https://github.com/me/mirror".to_string()),
        ];
        assert_eq!(pick_remote(&remotes), Some(("me".into(), "mirror".into())));
        assert_eq!(pick_remote(&[]), None);
    }
}
