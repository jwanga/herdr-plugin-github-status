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
pub struct GitRef {
    #[serde(rename = "ref")]
    pub name: String,
    #[serde(default)]
    pub sha: String,
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
    /// MERGEABLE | CONFLICTING | UNKNOWN
    pub mergeable: Option<String>,
    pub checks: Option<Checks>,
    /// Issues this PR closes (from GraphQL, else parsed from the body).
    pub closes: Vec<u64>,
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
    const KEYWORDS: [&str; 9] = ["close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved"];
    let mut out = Vec::new();
    let words: Vec<&str> = body.split_whitespace().collect();
    for pair in words.windows(2) {
        let key = pair[0].trim_end_matches(':').to_ascii_lowercase();
        if !KEYWORDS.contains(&key.as_str()) {
            continue;
        }
        let num = pair[1].trim_matches(|c: char| !c.is_ascii_digit() && c != '#');
        if let Some(n) = num.strip_prefix('#').and_then(|n| n.parse::<u64>().ok()) {
            if !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out
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
}

#[cfg(test)]
mod tests {
    use super::closing_refs;

    #[test]
    fn parses_closing_keywords() {
        assert_eq!(closing_refs("Fixes #12 and closes #7, resolves: #7. Refs #9. close #x"), vec![12, 7]);
        assert_eq!(closing_refs("Closes #3\n\nCo-Authored-By: x"), vec![3]);
        assert!(closing_refs("nothing here #4").is_empty());
    }
}
