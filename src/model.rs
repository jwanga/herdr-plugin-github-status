//! Normalized GitHub data as the pane sees it.
//!
//! Fields not yet read by the UI are consumed by later issues (tree view, PR checks,
//! activity feed); keep the shapes complete so the JSON contract is fixed once.
#![allow(dead_code)]

use crate::repo::RepoRef;
use serde::Deserialize;
use std::time::SystemTime;

#[derive(Debug, Clone, Deserialize)]
pub struct Milestone {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub open_issues: u64,
    pub closed_issues: u64,
    #[serde(default)]
    pub due_on: Option<String>,
    pub html_url: String,
    pub updated_at: String,
}

impl Milestone {
    pub fn total(&self) -> u64 {
        self.open_issues + self.closed_issues
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MilestoneRef {
    pub number: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Label {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    pub login: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub state_reason: Option<String>,
    #[serde(default)]
    pub milestone: Option<MilestoneRef>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub assignees: Vec<User>,
    pub updated_at: String,
    #[serde(default)]
    pub closed_at: Option<String>,
    pub html_url: String,
    /// Present on the issues endpoint when the "issue" is really a pull request.
    #[serde(default)]
    pub pull_request: Option<serde_json::Value>,
}

impl Issue {
    pub fn is_open(&self) -> bool {
        self.state == "open"
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RepoShort {
    pub full_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitRef {
    #[serde(rename = "ref")]
    pub name: String,
    #[serde(default)]
    pub sha: String,
    /// The repository the ref lives in (`None` for a deleted fork).
    #[serde(default)]
    pub repo: Option<RepoShort>,
}

/// Check-run / status rollup for a PR head, summarized from GraphQL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checks {
    /// SUCCESS | FAILURE | PENDING | ERROR | EXPECTED
    pub state: String,
    pub total: usize,
    pub failed: usize,
    pub pending: usize,
}

/// Data only the GraphQL API provides; empty when unauthenticated or on GraphQL errors.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrExtra {
    /// APPROVED | CHANGES_REQUESTED | REVIEW_REQUIRED
    pub review: Option<String>,
    pub checks: Option<Checks>,
    /// Issues this PR closes (from GraphQL, else parsed from the body).
    pub closes: Vec<u64>,
}

impl PrExtra {
    /// `(number, extra)` pairs from the GraphQL `pullRequests` query (see `github::PR_EXTRA_QUERY`).
    pub fn from_graphql(value: &serde_json::Value) -> Vec<(u64, PrExtra)> {
        let Some(nodes) = value
            .pointer("/data/repository/pullRequests/nodes")
            .and_then(|n| n.as_array())
        else {
            return Vec::new();
        };
        nodes
            .iter()
            .filter_map(|n| {
                let number = n.get("number")?.as_u64()?;
                let review = n
                    .get("reviewDecision")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let closes = n
                    .pointer("/closingIssuesReferences/nodes")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|i| i.get("number").and_then(|x| x.as_u64()))
                            .collect()
                    })
                    .unwrap_or_default();
                let checks = n
                    .pointer("/commits/nodes/0/commit/statusCheckRollup")
                    .filter(|r| !r.is_null())
                    .map(Checks::from_rollup);
                Some((
                    number,
                    PrExtra {
                        review,
                        checks,
                        closes,
                    },
                ))
            })
            .collect()
    }
}

impl Checks {
    /// Summarize a GraphQL `statusCheckRollup`: check runs and status contexts.
    pub fn from_rollup(rollup: &serde_json::Value) -> Checks {
        let contexts = rollup
            .pointer("/contexts/nodes")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut failed = 0;
        let mut pending = 0;
        for c in &contexts {
            let s = |k: &str| c.get(k).and_then(|v| v.as_str()).unwrap_or("");
            match s("__typename") {
                "CheckRun" => match (s("status"), s("conclusion")) {
                    ("COMPLETED", "SUCCESS" | "NEUTRAL" | "SKIPPED") => {}
                    ("COMPLETED", _) => failed += 1,
                    _ => pending += 1,
                },
                _ => match s("state") {
                    "SUCCESS" => {}
                    "PENDING" | "EXPECTED" => pending += 1,
                    _ => failed += 1,
                },
            }
        }
        Checks {
            state: rollup
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            total: rollup
                .pointer("/contexts/totalCount")
                .and_then(|v| v.as_u64())
                .unwrap_or(contexts.len() as u64) as usize,
            failed,
            pending,
        }
    }
}

/// Review decision derived from REST `/pulls/{n}/reviews`: the latest review per user
/// decides; any CHANGES_REQUESTED wins over APPROVED.
pub fn review_decision(reviews: &[Review]) -> Option<String> {
    let mut latest: Vec<(&str, &str)> = Vec::new();
    for r in reviews {
        if !matches!(
            r.state.as_str(),
            "APPROVED" | "CHANGES_REQUESTED" | "DISMISSED"
        ) {
            continue;
        }
        let login = r.user.as_ref().map(|u| u.login.as_str()).unwrap_or("");
        match latest.iter_mut().find(|(l, _)| *l == login) {
            Some(slot) => slot.1 = &r.state,
            None => latest.push((login, &r.state)),
        }
    }
    if latest.iter().any(|(_, s)| *s == "CHANGES_REQUESTED") {
        Some("CHANGES_REQUESTED".into())
    } else if latest.iter().any(|(_, s)| *s == "APPROVED") {
        Some("APPROVED".into())
    } else {
        None
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Review {
    pub state: String,
    #[serde(default)]
    pub user: Option<User>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub merged_at: Option<String>,
    #[serde(default)]
    pub closed_at: Option<String>,
    pub head: GitRef,
    pub base: GitRef,
    #[serde(default)]
    pub user: Option<User>,
    pub updated_at: String,
    pub html_url: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(skip)]
    pub extra: PrExtra,
}

impl PullRequest {
    pub fn is_open(&self) -> bool {
        self.state == "open"
    }
    pub fn is_merged(&self) -> bool {
        self.merged_at.is_some()
    }
}

/// Issue numbers referenced by closing keywords (`closes #12`, `Fixes: #3`, `resolved #7`).
pub fn closing_refs(body: &str) -> Vec<u64> {
    const KEYWORDS: [&str; 9] = [
        "close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved",
    ];
    let mut out = Vec::new();
    let words: Vec<&str> = body.split_whitespace().collect();
    for pair in words.windows(2) {
        let key = pair[0].trim_end_matches(':').to_ascii_lowercase();
        if !KEYWORDS.contains(&key.as_str()) {
            continue;
        }
        // Only surrounding punctuation is stripped: `other/repo#7` must not become `#7`.
        let num = pair[1]
            .trim_start_matches(['(', '[', '*', '_', '`', '"', '\''])
            .trim_end_matches(|c: char| !c.is_ascii_digit());
        if let Some(n) = num.strip_prefix('#').and_then(|n| n.parse::<u64>().ok()) {
            if !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out
}

/// A GitHub Actions workflow run.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct WorkflowRun {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub display_title: Option<String>,
    /// queued | in_progress | completed | waiting | requested | pending
    pub status: String,
    /// success | failure | cancelled | skipped | timed_out | action_required | neutral | stale
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub event: String,
    #[serde(default)]
    pub head_branch: Option<String>,
    #[serde(default)]
    pub head_sha: String,
    #[serde(default)]
    pub run_number: u64,
    #[serde(default)]
    pub run_started_at: Option<String>,
    pub updated_at: String,
    pub html_url: String,
}

impl WorkflowRun {
    /// Not finished (shown in NOW): queued, in progress, or waiting on something.
    pub fn is_active(&self) -> bool {
        self.status != "completed"
    }
    /// Actually executing or about to: the only states worth polling faster for.
    pub fn is_running(&self) -> bool {
        matches!(self.status.as_str(), "queued" | "in_progress")
    }
}

/// A check run on a commit (one job of a workflow, or an external app's check).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CheckRun {
    pub id: u64,
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
}

impl Checks {
    /// Summarize REST check runs the way `from_rollup` summarizes GraphQL contexts.
    pub fn from_check_runs(runs: &[CheckRun]) -> Checks {
        let mut failed = 0;
        let mut pending = 0;
        for r in runs {
            match (r.status.as_str(), r.conclusion.as_deref().unwrap_or("")) {
                ("completed", "success" | "neutral" | "skipped") => {}
                ("completed", _) => failed += 1,
                _ => pending += 1,
            }
        }
        let state = if failed > 0 {
            "FAILURE"
        } else if pending > 0 {
            "PENDING"
        } else {
            "SUCCESS"
        };
        Checks {
            state: state.into(),
            total: runs.len(),
            failed,
            pending,
        }
    }
}

/// A herdr agent running in this workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInfo {
    pub pane_id: String,
    pub agent: String,
    /// idle | working | blocked | done | unknown
    pub status: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub repo: RepoRef,
    pub milestones: Vec<Milestone>,
    pub issues: Vec<Issue>,
    pub prs: Vec<PullRequest>,
    /// Most recent workflow runs, newest first.
    pub runs: Vec<WorkflowRun>,
    /// Check runs by head SHA for open PRs.
    pub checks: std::collections::HashMap<String, Vec<CheckRun>>,
    pub fetched_at: SystemTime,
    pub rate_remaining: Option<u32>,
    pub authenticated: bool,
}

impl Snapshot {
    pub fn open_issues(&self) -> usize {
        self.issues.iter().filter(|i| i.is_open()).count()
    }
    pub fn open_prs(&self) -> usize {
        self.prs.iter().filter(|p| p.is_open()).count()
    }
    /// Any workflow run queued or in progress.
    pub fn has_running_runs(&self) -> bool {
        self.runs.iter().any(WorkflowRun::is_running)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_closing_keywords() {
        assert_eq!(
            closing_refs("Fixes #12 and closes #7, resolves: #7. Refs #9. close #x"),
            vec![12, 7]
        );
        assert_eq!(closing_refs("Closes #3\n\nCo-Authored-By: x"), vec![3]);
        assert_eq!(closing_refs("closes (#5) and fixes [#6]"), vec![5, 6]);
        assert!(closing_refs("nothing here #4").is_empty());
        assert!(
            closing_refs("Closes other/repo#7").is_empty(),
            "cross-repo refs are not local issues"
        );
    }

    #[test]
    fn summarizes_graphql_pr_extras() {
        let v: serde_json::Value = serde_json::from_str(r#"{"data":{"repository":{"pullRequests":{"nodes":[
          {"number":10,"reviewDecision":"APPROVED",
           "closingIssuesReferences":{"nodes":[{"number":1},{"number":2}]},
           "commits":{"nodes":[{"commit":{"statusCheckRollup":{"state":"FAILURE","contexts":{"totalCount":3,"nodes":[
             {"__typename":"CheckRun","status":"COMPLETED","conclusion":"SUCCESS"},
             {"__typename":"CheckRun","status":"IN_PROGRESS","conclusion":null},
             {"__typename":"StatusContext","state":"FAILURE"}]}}}}]}},
          {"number":11,"reviewDecision":null,"closingIssuesReferences":{"nodes":[]},
           "commits":{"nodes":[{"commit":{"statusCheckRollup":null}}]}}
        ]}}}}"#).unwrap();
        let extras = PrExtra::from_graphql(&v);
        assert_eq!(extras.len(), 2);
        let (n, e) = &extras[0];
        assert_eq!(*n, 10);
        assert_eq!(e.review.as_deref(), Some("APPROVED"));
        assert_eq!(e.closes, vec![1, 2]);
        let c = e.checks.as_ref().unwrap();
        assert_eq!(
            (c.state.as_str(), c.total, c.failed, c.pending),
            ("FAILURE", 3, 1, 1)
        );
        assert!(extras[1].1.checks.is_none() && extras[1].1.review.is_none());
        assert!(PrExtra::from_graphql(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn summarizes_check_runs() {
        let run = |status: &str, conclusion: Option<&str>| CheckRun {
            id: 1,
            name: "x".into(),
            status: status.into(),
            conclusion: conclusion.map(str::to_string),
            html_url: None,
            started_at: None,
            completed_at: None,
        };
        let c = Checks::from_check_runs(&[
            run("completed", Some("success")),
            run("in_progress", None),
            run("completed", Some("failure")),
            run("completed", Some("skipped")),
        ]);
        assert_eq!(
            (c.state.as_str(), c.total, c.failed, c.pending),
            ("FAILURE", 4, 1, 1)
        );
        assert_eq!(
            Checks::from_check_runs(&[run("completed", Some("success"))]).state,
            "SUCCESS"
        );
        assert_eq!(
            Checks::from_check_runs(&[run("queued", None)]).state,
            "PENDING"
        );
    }

    #[test]
    fn derives_review_decision_from_rest_reviews() {
        let r = |state: &str, login: &str| Review {
            state: state.into(),
            user: Some(User {
                login: login.into(),
            }),
        };
        assert_eq!(review_decision(&[r("COMMENTED", "a")]), None);
        assert_eq!(
            review_decision(&[r("APPROVED", "a")]).as_deref(),
            Some("APPROVED")
        );
        assert_eq!(
            review_decision(&[r("APPROVED", "a"), r("CHANGES_REQUESTED", "b")]).as_deref(),
            Some("CHANGES_REQUESTED")
        );
        // The latest review per user wins.
        assert_eq!(
            review_decision(&[r("CHANGES_REQUESTED", "b"), r("APPROVED", "b")]).as_deref(),
            Some("APPROVED")
        );
        assert_eq!(
            review_decision(&[r("APPROVED", "b"), r("DISMISSED", "b")]),
            None
        );
    }
}
