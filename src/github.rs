//! GitHub REST + GraphQL client: token discovery, paginated GETs, rate-limit tracking, and
//! the per-refresh PR enrichment query.

use crate::cache::EtagCache;
use crate::model::{
    closing_refs, review_decision, CheckRun, Checks, Issue, Milestone, PrExtra, PullRequest,
    Review, Snapshot, WorkflowRun,
};
use crate::repo::RepoRef;
use crate::util;
use anyhow::{anyhow, bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use ureq::Agent;

const API: &str = "https://api.github.com";
const GRAPHQL: &str = "https://api.github.com/graphql";

/// Review decision, mergeability, checks rollup, and closing issues for open PRs.
const PR_EXTRA_QUERY: &str = r#"query($owner:String!,$name:String!){
  repository(owner:$owner,name:$name){
    pullRequests(first:50,states:[OPEN],orderBy:{field:UPDATED_AT,direction:DESC}){
      nodes{
        number reviewDecision
        closingIssuesReferences(first:20){nodes{number}}
        commits(last:1){nodes{commit{statusCheckRollup{
          state
          contexts(first:100){totalCount nodes{
            __typename
            ... on CheckRun{status conclusion}
            ... on StatusContext{state}
          }}
        }}}}
      }
    }
  }
}"#;
const MAX_ISSUE_PAGES: usize = 3;
/// Workflow runs shown in the Actions section.
pub const RUNS_LIMIT: usize = 15;
/// Open PR heads whose check runs are fetched per refresh (authenticated).
const CHECKS_PR_LIMIT: usize = 5;

#[derive(Deserialize)]
struct RunsPage {
    #[serde(default)]
    workflow_runs: Vec<WorkflowRun>,
}

#[derive(Deserialize)]
struct CheckRunsPage {
    #[serde(default)]
    check_runs: Vec<CheckRun>,
}

pub struct Client {
    agent: Agent,
    token: Option<String>,
    pub rate_remaining: Option<u32>,
    cache: EtagCache,
    debug_log: Option<std::path::PathBuf>,
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
        let secs = self
            .until
            .duration_since(SystemTime::now())
            .map(|d| d.as_secs())
            .unwrap_or(0);
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

/// Head SHAs whose check runs are worth fetching: the current branch's open PR first, then
/// the most recently updated open PRs, at most `CHECKS_PR_LIMIT`; only the current branch's
/// PR when unauthenticated (the 60/hour budget).
pub fn check_heads(prs: &[PullRequest], branch: Option<&str>, authenticated: bool) -> Vec<String> {
    let is_current = |p: &PullRequest| branch.is_some_and(|b| p.head.name == b);
    let mut open: Vec<&PullRequest> = prs
        .iter()
        .filter(|p| p.is_open() && !p.head.sha.is_empty())
        .filter(|p| authenticated || is_current(p))
        .collect();
    open.sort_by_key(|p| !is_current(p)); // stable: current branch first, updated-desc after
    let mut seen = std::collections::HashSet::new();
    let mut heads: Vec<String> = open
        .into_iter()
        .map(|p| p.head.sha.clone())
        .filter(|s| seen.insert(s.clone()))
        .collect();
    heads.truncate(CHECKS_PR_LIMIT);
    heads
}

/// When to retry after a 403/429: `Retry-After` seconds (secondary limit), else the primary
/// limit's reset time when the remaining budget is exhausted, else `None` (a plain 403).
pub fn rate_limit_until(
    status: u16,
    retry_after: Option<u64>,
    reset_epoch: Option<u64>,
    remaining: Option<u32>,
) -> Option<SystemTime> {
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
        rel.contains("rel=\"next\"").then(|| {
            url.trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_string()
        })
    })
}

impl Client {
    /// `state_dir` (the plugin state directory) hosts the persisted ETag cache and, when
    /// `HERDR_GITHUB_STATUS_DEBUG` is set, a `debug.log` of request statuses.
    pub fn new(token: Option<String>, state_dir: Option<&std::path::Path>) -> Self {
        let config = Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(15)))
            .user_agent(concat!("herdr-github-status/", env!("CARGO_PKG_VERSION")))
            .build();
        let debug_log = std::env::var("HERDR_GITHUB_STATUS_DEBUG")
            .ok()
            .filter(|v| !v.is_empty() && v != "0")
            .and(state_dir.map(|d| d.join("debug.log")));
        Self {
            agent: Agent::new_with_config(config),
            token,
            rate_remaining: None,
            cache: EtagCache::open(state_dir),
            debug_log,
        }
    }

    fn log(&self, status: u16, url: &str) {
        let Some(path) = &self.debug_log else { return };
        use std::io::Write;
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{ts} {status} {url}");
        }
    }

    /// A conditional GET through the ETag cache: sends `If-None-Match`, reuses the cached
    /// body on 304 (free of rate limit), stores the body on 200. Returns the items and
    /// whether the resource changed.
    pub fn get_cached<T: DeserializeOwned>(
        &mut self,
        url: &str,
    ) -> Result<(Option<T>, bool, Option<String>)> {
        let etag = self.cache.etag(url).map(str::to_string);
        let mut page: Page<serde_json::Value> = self.get(url, etag.as_deref())?;
        let changed = page.items.is_some();
        let body = match page.items.take() {
            Some(value) => {
                let text = value.to_string();
                if let Some(e) = page.etag.clone() {
                    self.cache.store(url, e, text.clone());
                }
                Some(text)
            }
            None => self.cache.body(url).map(str::to_string),
        };
        let items = match body {
            Some(text) => Some(serde_json::from_str::<T>(&text).context("decoding cached JSON")?),
            None => None,
        };
        Ok((items, changed, page.next))
    }

    /// Persist the ETag cache (no-op unless it changed).
    pub fn save_cache(&mut self) {
        self.cache.save();
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
    fn get_once<T: DeserializeOwned>(
        &mut self,
        url: &str,
        etag: Option<&str>,
    ) -> Result<std::result::Result<Page<T>, String>> {
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
        self.log(status, url);
        let body = resp
            .body_mut()
            .read_to_string()
            .context("reading response body")?;
        match status {
            200 => Ok(Ok(Page {
                items: Some(serde_json::from_str(&body).context("decoding JSON")?),
                etag,
                next,
            })),
            304 => Ok(Ok(Page {
                items: None,
                etag,
                next,
            })),
            301 | 307 | 308 => match location {
                Some(l) if l.starts_with(API) => Ok(Err(l)),
                _ => {
                    bail!("repository was renamed or moved (HTTP {status}); update the git remote")
                }
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
            let (items, _changed, page_next) = self.get_cached::<Vec<T>>(&u)?;
            out.extend(items.unwrap_or_default());
            next = page_next;
            pages += 1;
        }
        Ok(out)
    }

    /// POST a GraphQL query; requires a token.
    pub fn graphql(
        &mut self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let token = self
            .token
            .clone()
            .ok_or_else(|| anyhow!("GraphQL needs a token"))?;
        let body = serde_json::json!({ "query": query, "variables": variables });
        let mut resp = self
            .agent
            .post(GRAPHQL)
            .header("Authorization", &format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .send(body.to_string().as_bytes())
            .context("POST graphql")?;
        let status = resp.status().as_u16();
        let text = resp
            .body_mut()
            .read_to_string()
            .context("reading GraphQL body")?;
        if status != 200 {
            bail!("GraphQL returned HTTP {status}");
        }
        let value: serde_json::Value =
            serde_json::from_str(&text).context("decoding GraphQL JSON")?;
        if let Some(errors) = value
            .get("errors")
            .and_then(|e| e.as_array())
            .filter(|e| !e.is_empty())
        {
            let msg = errors[0]
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            bail!("GraphQL error: {msg}");
        }
        Ok(value)
    }

    /// Fill `extra` on open PRs: GraphQL when possible; otherwise body-parsed closing refs
    /// plus a REST reviews lookup for the current branch's PR only (one extra request, so
    /// the unauthenticated 60/hour budget survives).
    fn enrich_prs(&mut self, repo: &RepoRef, prs: &mut [PullRequest]) {
        for p in prs.iter_mut() {
            p.extra.closes = p.body.as_deref().map(closing_refs).unwrap_or_default();
        }
        if !prs.iter().any(|p| p.is_open()) {
            return;
        }
        let vars = serde_json::json!({ "owner": repo.owner, "name": repo.name });
        let graphql = if self.authenticated() {
            self.graphql(PR_EXTRA_QUERY, vars).ok()
        } else {
            None
        };
        if let Some(value) = graphql {
            let mut extras: HashMap<u64, PrExtra> =
                PrExtra::from_graphql(&value).into_iter().collect();
            for p in prs.iter_mut() {
                if let Some(mut extra) = extras.remove(&p.number) {
                    if extra.closes.is_empty() {
                        extra.closes = std::mem::take(&mut p.extra.closes);
                    }
                    p.extra = extra;
                }
            }
            return;
        }
        let Some(branch) = repo.branch.as_deref() else {
            return;
        };
        if let Some(p) = prs
            .iter_mut()
            .find(|p| p.is_open() && p.head.name == branch)
        {
            let url = format!(
                "{API}/repos/{}/{}/pulls/{}/reviews?per_page=100",
                repo.owner, repo.name, p.number
            );
            if let Ok(page) = self.get::<Vec<Review>>(&url, None) {
                p.extra.review = review_decision(&page.items.unwrap_or_default());
            }
        }
    }

    /// The latest workflow runs. A rate limit propagates; any other failure yields `None`
    /// so the caller can keep what it had.
    fn fetch_runs(&mut self, repo: &RepoRef) -> Result<Option<Vec<WorkflowRun>>> {
        let url = format!(
            "{API}/repos/{}/{}/actions/runs?per_page={RUNS_LIMIT}",
            repo.owner, repo.name
        );
        match self.get_cached::<RunsPage>(&url) {
            Ok((items, _, _)) => Ok(Some(items.map(|p| p.workflow_runs).unwrap_or_default())),
            Err(e) if e.downcast_ref::<RateLimited>().is_some() => Err(e),
            Err(_) => Ok(None),
        }
    }

    /// Check runs for the selected open PR heads (see `check_heads`), filling a missing
    /// `extra.checks` from them. A rate limit propagates; other failures skip that head.
    fn fetch_checks(
        &mut self,
        repo: &RepoRef,
        prs: &mut [PullRequest],
    ) -> Result<HashMap<String, Vec<CheckRun>>> {
        let heads = check_heads(prs, repo.branch.as_deref(), self.authenticated());
        let mut out = HashMap::new();
        for sha in heads {
            let url = format!(
                "{API}/repos/{}/{}/commits/{sha}/check-runs?per_page=100",
                repo.owner, repo.name
            );
            match self.get_cached::<CheckRunsPage>(&url) {
                Ok((items, _, _)) => {
                    out.insert(sha, items.map(|p| p.check_runs).unwrap_or_default());
                }
                Err(e) if e.downcast_ref::<RateLimited>().is_some() => return Err(e),
                Err(_) => {}
            }
        }
        for p in prs.iter_mut() {
            if p.extra.checks.is_none() {
                if let Some(runs) = out.get(&p.head.sha).filter(|r| !r.is_empty()) {
                    p.extra.checks = Some(Checks::from_check_runs(runs));
                }
            }
        }
        Ok(out)
    }

    /// The cheap fast-tick refresh: runs and check runs only, into an existing snapshot.
    pub fn refresh_runs(&mut self, repo: &RepoRef, snapshot: &mut Snapshot) -> Result<()> {
        if let Some(runs) = self.fetch_runs(repo)? {
            snapshot.runs = runs;
        }
        snapshot.checks = self.fetch_checks(repo, &mut snapshot.prs)?;
        snapshot.fetched_at = SystemTime::now();
        snapshot.rate_remaining = self.rate_remaining;
        Ok(())
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
        let mut prs: Vec<PullRequest> = self.get_all(
            &format!("{base}/pulls?state=all&per_page=50&sort=updated&direction=desc"),
            1,
        )?;
        self.enrich_prs(repo, &mut prs);
        let runs = self.fetch_runs(repo)?.unwrap_or_default();
        let checks = self.fetch_checks(repo, &mut prs)?;
        self.save_cache();
        Ok(Snapshot {
            repo: repo.clone(),
            milestones,
            issues,
            prs,
            runs,
            checks,
            fetched_at: SystemTime::now(),
            rate_remaining: self.rate_remaining,
            authenticated: self.authenticated(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{check_heads, next_link, rate_limit_until};
    use crate::model::{GitRef, PrExtra, PullRequest};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn pr(number: u64, state: &str, branch: &str, sha: &str) -> PullRequest {
        PullRequest {
            number,
            title: String::new(),
            state: state.into(),
            draft: false,
            merged_at: None,
            closed_at: None,
            head: GitRef {
                name: branch.into(),
                sha: sha.into(),
                repo: None,
            },
            base: GitRef {
                name: "main".into(),
                sha: String::new(),
                repo: None,
            },
            user: None,
            updated_at: String::new(),
            html_url: String::new(),
            body: None,
            extra: PrExtra::default(),
        }
    }

    #[test]
    fn check_heads_prefers_current_branch_and_dedups() {
        // updated-desc order as GitHub returns it; a closed PR precedes the current branch's PR.
        let prs = vec![
            pr(9, "closed", "old", "s9"),
            pr(8, "open", "feat-b", "s8"),
            pr(7, "open", "feat-a", "s7"),
            pr(6, "open", "feat-b", "s8"),
            pr(5, "open", "nosha", ""),
            pr(4, "open", "feat-c", "s4"),
            pr(3, "open", "feat-d", "s3"),
            pr(2, "open", "feat-e", "s2"),
            pr(1, "open", "feat-f", "s1"),
        ];
        assert_eq!(
            check_heads(&prs, Some("feat-a"), true),
            vec!["s7", "s8", "s4", "s3", "s2"]
        );
        assert_eq!(
            check_heads(&prs, None, true),
            vec!["s8", "s7", "s4", "s3", "s2"]
        );
        assert_eq!(
            check_heads(&prs, Some("feat-a"), false),
            vec!["s7"],
            "unauthenticated: current branch only"
        );
        assert!(check_heads(&prs, Some("main"), false).is_empty());
        assert!(
            check_heads(&prs, Some("old"), true)
                .iter()
                .all(|s| s != "s9"),
            "closed PRs never qualify"
        );
    }

    #[test]
    fn rate_limit_backoff_rules() {
        let now = SystemTime::now();
        let ra = rate_limit_until(403, Some(30), None, Some(10)).unwrap();
        assert!(ra >= now + Duration::from_secs(29));
        let reset = 4_000_000_000u64;
        assert_eq!(
            rate_limit_until(403, None, Some(reset), Some(0)),
            Some(UNIX_EPOCH + Duration::from_secs(reset))
        );
        assert_eq!(
            rate_limit_until(403, None, Some(reset), Some(5)),
            None,
            "plain 403 is not a rate limit"
        );
        assert!(rate_limit_until(429, None, None, None).is_some());
    }

    #[test]
    fn parses_next_link() {
        let link = r#"<https://api.github.com/repositories/1/issues?page=2>; rel="next", <https://api.github.com/repositories/1/issues?page=5>; rel="last""#;
        assert_eq!(
            next_link(link).as_deref(),
            Some("https://api.github.com/repositories/1/issues?page=2")
        );
        let last_only = r#"<https://api.github.com/repositories/1/issues?page=1>; rel="prev", <https://api.github.com/repositories/1/issues?page=1>; rel="first""#;
        assert_eq!(next_link(last_only), None);
        assert_eq!(next_link(""), None);
    }
}
