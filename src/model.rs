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

#[derive(Debug, Clone, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub merged_at: Option<String>,
    pub head: GitRef,
    pub base: GitRef,
    #[serde(default)]
    pub user: Option<User>,
    pub updated_at: String,
    pub html_url: String,
    #[serde(default)]
    pub body: Option<String>,
}

impl PullRequest {
    pub fn is_open(&self) -> bool {
        self.state == "open"
    }
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
