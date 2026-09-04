//! Minimal GitHub REST client: token discovery, paginated GETs, rate-limit tracking.

use crate::model::{Issue, Milestone, PullRequest, Snapshot};
use crate::repo::RepoRef;
use anyhow::{anyhow, bail, Context, Result};
use serde::de::DeserializeOwned;
use std::process::Command;
use std::time::{Duration, SystemTime};
use ureq::Agent;

const API: &str = "https://api.github.com";
const MAX_ISSUE_PAGES: usize = 3;

pub struct Client {
    agent: Agent,
    token: Option<String>,
    pub rate_remaining: Option<u32>,
}

/// `GH_TOKEN` / `GITHUB_TOKEN`, else `gh auth token`.
pub fn discover_token() -> Option<String> {
    for key in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(v) = std::env::var(key) {
            let v = v.trim().to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    let out = Command::new("gh").args(["auth", "token"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!token.is_empty()).then_some(token)
}

/// `status` and `etag` feed the conditional-request cache (issue #6).
#[allow(dead_code)]
pub struct Page<T> {
    pub status: u16,
    pub items: Option<T>,
    pub etag: Option<String>,
    pub next: Option<String>,
}

/// The `rel="next"` URL from a `Link` header.
pub fn next_link(link: &str) -> Option<String> {
    link.split(',').find_map(|part| {
        let (url, rel) = part.split_once(';')?;
        rel.contains("rel=\"next\"")
            .then(|| url.trim().trim_start_matches('<').trim_end_matches('>').to_string())
    })
}

impl Client {
    pub fn new(token: Option<String>) -> Self {
        let config = Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(15)))
            .user_agent(concat!("herdr-github-status/", env!("CARGO_PKG_VERSION")))
            .build();
        Self { agent: Agent::new_with_config(config), token, rate_remaining: None }
    }

    pub fn authenticated(&self) -> bool {
        self.token.is_some()
    }

    /// One GET; a 304 yields `items: None`.
    pub fn get<T: DeserializeOwned>(&mut self, url: &str, etag: Option<&str>) -> Result<Page<T>> {
        let mut req = self
            .agent
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(t) = &self.token {
            req = req.header("Authorization", &format!("Bearer {t}"));
        }
        if let Some(e) = etag {
            req = req.header("If-None-Match", e);
        }
        let mut resp = req.call().with_context(|| format!("GET {url}"))?;
        let header = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        if let Some(rem) = header("x-ratelimit-remaining").and_then(|v| v.parse().ok()) {
            self.rate_remaining = Some(rem);
        }
        let etag = header("etag");
        let next = header("link").and_then(|l| next_link(&l));
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_to_string().context("reading response body")?;
        match status {
            200 => Ok(Page { status, items: Some(serde_json::from_str(&body).context("decoding JSON")?), etag, next }),
            304 => Ok(Page { status, items: None, etag, next }),
            401 => bail!("GitHub rejected the token (401)"),
            403 | 429 => bail!("GitHub rate limit or access denied ({status})"),
            404 => bail!("repository not found or not accessible (404)"),
            _ => Err(anyhow!("GitHub returned HTTP {status}")),
        }
    }

    /// Follow `Link: rel="next"` up to `max_pages` pages.
    pub fn get_all<T: DeserializeOwned>(&mut self, url: &str, max_pages: usize) -> Result<Vec<T>> {
        let mut out = Vec::new();
        let mut next = Some(url.to_string());
        let mut pages = 0;
        while let Some(u) = next {
            if pages >= max_pages {
                break;
            }
            let page: Page<Vec<T>> = self.get(&u, None)?;
            out.extend(page.items.unwrap_or_default());
            next = page.next;
            pages += 1;
        }
        Ok(out)
    }

    pub fn fetch_snapshot(&mut self, repo: &RepoRef) -> Result<Snapshot> {
        let base = format!("{API}/repos/{}/{}", repo.owner, repo.name);
        let milestones: Vec<Milestone> =
            self.get_all(&format!("{base}/milestones?state=all&per_page=100"), 2)?;
        let issues: Vec<Issue> = self
            .get_all::<Issue>(
                &format!("{base}/issues?state=all&per_page=100&sort=updated&direction=desc"),
                MAX_ISSUE_PAGES,
            )?
            .into_iter()
            .filter(|i| i.pull_request.is_none())
            .collect();
        let prs: Vec<PullRequest> =
            self.get_all(&format!("{base}/pulls?state=all&per_page=50&sort=updated&direction=desc"), 1)?;
        Ok(Snapshot {
            repo: repo.clone(),
            milestones,
            issues,
            prs,
            fetched_at: SystemTime::now(),
            rate_remaining: self.rate_remaining,
            authenticated: self.authenticated(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::next_link;

    #[test]
    fn parses_next_link() {
        let link = r#"<https://api.github.com/repositories/1/issues?page=2>; rel="next", <https://api.github.com/repositories/1/issues?page=5>; rel="last""#;
        assert_eq!(next_link(link).as_deref(), Some("https://api.github.com/repositories/1/issues?page=2"));
        let last_only = r#"<https://api.github.com/repositories/1/issues?page=1>; rel="prev", <https://api.github.com/repositories/1/issues?page=1>; rel="first""#;
        assert_eq!(next_link(last_only), None);
        assert_eq!(next_link(""), None);
    }
}
