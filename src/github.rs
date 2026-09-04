//! Minimal GitHub REST client: token discovery, paginated GETs, rate-limit tracking.

use crate::model::{Issue, Milestone, PullRequest, Snapshot};
use crate::repo::RepoRef;
use crate::util;
use anyhow::{anyhow, bail, Context, Result};
use serde::de::DeserializeOwned;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
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
    util::stdout("gh", &["auth", "token"], None)
}

/// GitHub asked us to stop until `until` (primary or secondary rate limit).
#[derive(Debug)]
pub struct RateLimited {
    pub until: SystemTime,
}

impl std::fmt::Display for RateLimited {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let secs = self.until.duration_since(SystemTime::now()).map(|d| d.as_secs()).unwrap_or(0);
        write!(f, "GitHub rate limit reached; retrying in {secs}s")
    }
}

impl std::error::Error for RateLimited {}

pub struct Page<T> {
    /// `None` on a 304 Not Modified.
    pub items: Option<T>,
    /// Feeds the conditional-request cache (issue #6).
    #[allow(dead_code)]
    pub etag: Option<String>,
    pub next: Option<String>,
}

/// When to retry after a 403/429: `Retry-After` seconds (secondary limit), else the primary
/// limit's reset time when the remaining budget is exhausted, else `None` (a plain 403).
pub fn rate_limit_until(status: u16, retry_after: Option<u64>, reset_epoch: Option<u64>, remaining: Option<u32>) -> Option<SystemTime> {
    if let Some(secs) = retry_after {
        return Some(SystemTime::now() + Duration::from_secs(secs.max(1)));
    }
    if status == 429 || remaining == Some(0) {
        let reset = reset_epoch.map(|e| UNIX_EPOCH + Duration::from_secs(e));
        return Some(reset.unwrap_or_else(|| SystemTime::now() + Duration::from_secs(60)));
    }
    None
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

    /// One GET; a 304 yields `items: None`. Follows a single redirect (a renamed or
    /// transferred repository answers 301) since the agent itself has redirects disabled.
    pub fn get<T: DeserializeOwned>(&mut self, url: &str, etag: Option<&str>) -> Result<Page<T>> {
        match self.get_once(url, etag)? {
            Ok(page) => Ok(page),
            Err(location) => match self.get_once(&location, etag)? {
                Ok(page) => Ok(page),
                Err(_) => bail!("repository was moved more than once; update the git remote"),
            },
        }
    }

    /// `Ok(Err(location))` signals a redirect to follow.
    fn get_once<T: DeserializeOwned>(&mut self, url: &str, etag: Option<&str>) -> Result<std::result::Result<Page<T>, String>> {
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
        let location = header("location");
        let retry_after = header("retry-after").and_then(|v| v.parse::<u64>().ok());
        let reset = header("x-ratelimit-reset").and_then(|v| v.parse::<u64>().ok());
        let remaining = header("x-ratelimit-remaining").and_then(|v| v.parse::<u32>().ok());
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_to_string().context("reading response body")?;
        match status {
            200 => Ok(Ok(Page { items: Some(serde_json::from_str(&body).context("decoding JSON")?), etag, next })),
            304 => Ok(Ok(Page { items: None, etag, next })),
            301 | 307 | 308 => match location {
                Some(l) if l.starts_with(API) => Ok(Err(l)),
                _ => bail!("repository was renamed or moved (HTTP {status}); update the git remote"),
            },
            401 => bail!("GitHub rejected the token (401)"),
            403 | 429 => {
                let until = rate_limit_until(status, retry_after, reset, remaining);
                match until {
                    Some(until) => Err(RateLimited { until }.into()),
                    None => bail!("GitHub denied access ({status})"),
                }
            }
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
    use super::{next_link, rate_limit_until};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn rate_limit_backoff_rules() {
        let now = SystemTime::now();
        let ra = rate_limit_until(403, Some(30), None, Some(10)).unwrap();
        assert!(ra >= now + Duration::from_secs(29));
        let reset = 4_000_000_000u64;
        assert_eq!(rate_limit_until(403, None, Some(reset), Some(0)), Some(UNIX_EPOCH + Duration::from_secs(reset)));
        assert_eq!(rate_limit_until(403, None, Some(reset), Some(5)), None, "plain 403 is not a rate limit");
        assert!(rate_limit_until(429, None, None, None).is_some());
    }

    #[test]
    fn parses_next_link() {
        let link = r#"<https://api.github.com/repositories/1/issues?page=2>; rel="next", <https://api.github.com/repositories/1/issues?page=5>; rel="last""#;
        assert_eq!(next_link(link).as_deref(), Some("https://api.github.com/repositories/1/issues?page=2"));
        let last_only = r#"<https://api.github.com/repositories/1/issues?page=1>; rel="prev", <https://api.github.com/repositories/1/issues?page=1>; rel="first""#;
        assert_eq!(next_link(last_only), None);
        assert_eq!(next_link(""), None);
    }
}
