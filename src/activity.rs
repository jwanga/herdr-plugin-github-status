//! Change detection between two snapshots: the transitions the activity feed shows.

use crate::model::Snapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Issue(u64),
    Milestone(u64),
    Pr(u64),
    Run(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    IssueOpened,
    IssueClosed,
    IssueReopened,
    MilestoneCreated,
    MilestoneClosed,
    PrOpened,
    PrReady,
    PrReview(String),
    PrMerged,
    PrClosed,
    RunStarted,
    /// The run's conclusion (`success`, `failure`, …).
    RunFinished(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// Unix seconds when the change was observed.
    pub at: u64,
    pub kind: Kind,
    pub target: Target,
    /// `#12`, `CI`, or a milestone title.
    pub label: String,
    pub title: String,
    pub url: Option<String>,
}

impl Event {
    /// Short verb for the row: `closed`, `merged`, `approved`, `failed`, …
    pub fn verb(&self) -> String {
        match &self.kind {
            Kind::IssueOpened | Kind::PrOpened => "opened".into(),
            Kind::IssueClosed | Kind::PrClosed | Kind::MilestoneClosed => "closed".into(),
            Kind::IssueReopened => "reopened".into(),
            Kind::MilestoneCreated => "created".into(),
            Kind::PrReady => "ready".into(),
            Kind::PrReview(d) => match d.as_str() {
                "APPROVED" => "approved".into(),
                "CHANGES_REQUESTED" => "changes req.".into(),
                other => other.to_lowercase().replace('_', " "),
            },
            Kind::PrMerged => "merged".into(),
            Kind::RunStarted => "started".into(),
            Kind::RunFinished(c) => match c.as_str() {
                "success" => "passed".into(),
                "failure" => "failed".into(),
                other => other.replace('_', " "),
            },
        }
    }
}

/// Transitions from `prev` to `next`, in detection order (issues, milestones, PRs, runs).
pub fn diff(prev: &Snapshot, next: &Snapshot, now: u64) -> Vec<Event> {
    let mut out = Vec::new();
    let ev = |kind: Kind, target: Target, label: String, title: &str, url: &str| Event {
        at: now,
        kind,
        target,
        label,
        title: title.to_string(),
        url: Some(url.to_string()),
    };

    for i in &next.issues {
        let label = format!("#{}", i.number);
        match prev.issues.iter().find(|p| p.number == i.number) {
            None if i.is_open() => out.push(ev(
                Kind::IssueOpened,
                Target::Issue(i.number),
                label,
                &i.title,
                &i.html_url,
            )),
            None => {}
            Some(p) if p.is_open() && !i.is_open() => out.push(ev(
                Kind::IssueClosed,
                Target::Issue(i.number),
                label,
                &i.title,
                &i.html_url,
            )),
            Some(p) if !p.is_open() && i.is_open() => out.push(ev(
                Kind::IssueReopened,
                Target::Issue(i.number),
                label,
                &i.title,
                &i.html_url,
            )),
            _ => {}
        }
    }
    for m in &next.milestones {
        match prev.milestones.iter().find(|p| p.number == m.number) {
            None => out.push(ev(
                Kind::MilestoneCreated,
                Target::Milestone(m.number),
                m.title.clone(),
                "milestone",
                &m.html_url,
            )),
            Some(p) if p.state == "open" && m.state != "open" => out.push(ev(
                Kind::MilestoneClosed,
                Target::Milestone(m.number),
                m.title.clone(),
                "milestone",
                &m.html_url,
            )),
            _ => {}
        }
    }
    for p in &next.prs {
        let label = format!("#{}", p.number);
        let t = Target::Pr(p.number);
        match prev.prs.iter().find(|q| q.number == p.number) {
            None if p.is_open() => out.push(ev(Kind::PrOpened, t, label, &p.title, &p.html_url)),
            None => {}
            Some(q) => {
                if q.is_open() && p.is_merged() {
                    out.push(ev(
                        Kind::PrMerged,
                        t.clone(),
                        label.clone(),
                        &p.title,
                        &p.html_url,
                    ));
                } else if q.is_open() && !p.is_open() {
                    out.push(ev(
                        Kind::PrClosed,
                        t.clone(),
                        label.clone(),
                        &p.title,
                        &p.html_url,
                    ));
                }
                if p.is_open() && q.draft && !p.draft {
                    out.push(ev(
                        Kind::PrReady,
                        t.clone(),
                        label.clone(),
                        &p.title,
                        &p.html_url,
                    ));
                }
                if p.is_open() && p.extra.review.is_some() && p.extra.review != q.extra.review {
                    out.push(ev(
                        Kind::PrReview(p.extra.review.clone().unwrap_or_default()),
                        t,
                        label,
                        &p.title,
                        &p.html_url,
                    ));
                }
            }
        }
    }
    for r in &next.runs {
        let t = Target::Run(r.id);
        match prev.runs.iter().find(|q| q.id == r.id) {
            None if r.is_active() => out.push(ev(
                Kind::RunStarted,
                t,
                r.name.clone(),
                "workflow run",
                &r.html_url,
            )),
            None => out.push(ev(
                Kind::RunFinished(r.conclusion.clone().unwrap_or_default()),
                t,
                r.name.clone(),
                "workflow run",
                &r.html_url,
            )),
            Some(q) if q.is_active() && !r.is_active() => out.push(ev(
                Kind::RunFinished(r.conclusion.clone().unwrap_or_default()),
                t,
                r.name.clone(),
                "workflow run",
                &r.html_url,
            )),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GitRef, Issue, Milestone, PrExtra, PullRequest, WorkflowRun};
    use crate::repo::RepoRef;
    use std::time::SystemTime;

    fn issue(n: u64, state: &str) -> Issue {
        Issue {
            number: n,
            title: format!("I{n}"),
            state: state.into(),
            state_reason: None,
            milestone: None,
            labels: vec![],
            assignees: vec![],
            updated_at: String::new(),
            closed_at: None,
            html_url: format!("i{n}"),
            pull_request: None,
        }
    }
    fn ms(n: u64, state: &str) -> Milestone {
        Milestone {
            number: n,
            title: format!("M{n}"),
            state: state.into(),
            open_issues: 0,
            closed_issues: 0,
            due_on: None,
            html_url: format!("m{n}"),
            updated_at: String::new(),
        }
    }
    fn pr(n: u64, state: &str, merged: bool, draft: bool, review: Option<&str>) -> PullRequest {
        PullRequest {
            number: n,
            title: format!("P{n}"),
            state: state.into(),
            draft,
            merged_at: merged.then(|| "t".to_string()),
            closed_at: None,
            head: GitRef {
                name: "b".into(),
                sha: String::new(),
                repo: None,
            },
            base: GitRef {
                name: "main".into(),
                sha: String::new(),
                repo: None,
            },
            user: None,
            updated_at: String::new(),
            html_url: format!("p{n}"),
            body: None,
            extra: PrExtra {
                review: review.map(str::to_string),
                checks: None,
                closes: vec![],
            },
        }
    }
    fn run(id: u64, status: &str, conclusion: Option<&str>) -> WorkflowRun {
        WorkflowRun {
            id,
            name: "CI".into(),
            display_title: None,
            status: status.into(),
            conclusion: conclusion.map(str::to_string),
            event: String::new(),
            head_branch: None,
            head_sha: String::new(),
            run_number: id,
            run_started_at: None,
            updated_at: String::new(),
            html_url: format!("r{id}"),
        }
    }
    fn snap(
        issues: Vec<Issue>,
        milestones: Vec<Milestone>,
        prs: Vec<PullRequest>,
        runs: Vec<WorkflowRun>,
    ) -> Snapshot {
        Snapshot {
            repo: RepoRef {
                owner: "o".into(),
                name: "r".into(),
                branch: None,
                root: String::new(),
            },
            milestones,
            issues,
            prs,
            runs,
            checks: Default::default(),
            fetched_at: SystemTime::now(),
            rate_remaining: None,
            authenticated: true,
        }
    }

    #[test]
    fn detects_every_transition() {
        let prev = snap(
            vec![issue(1, "open"), issue(2, "closed"), issue(3, "open")],
            vec![ms(1, "open")],
            vec![
                pr(10, "open", false, true, None),
                pr(11, "open", false, false, None),
                pr(12, "open", false, false, Some("REVIEW_REQUIRED")),
            ],
            vec![
                run(100, "in_progress", None),
                run(101, "completed", Some("success")),
            ],
        );
        let next = snap(
            vec![
                issue(1, "closed"),
                issue(2, "open"),
                issue(3, "open"),
                issue(4, "open"),
                issue(5, "closed"),
            ],
            vec![ms(1, "closed"), ms(2, "open")],
            vec![
                pr(10, "open", false, false, None),
                pr(11, "closed", true, false, None),
                pr(12, "open", false, false, Some("APPROVED")),
                pr(13, "open", false, false, None),
            ],
            vec![
                run(100, "completed", Some("failure")),
                run(101, "completed", Some("success")),
                run(102, "queued", None),
                run(103, "completed", Some("success")),
            ],
        );
        let events = diff(&prev, &next, 42);
        let kinds: Vec<(Kind, Target)> = events
            .iter()
            .map(|e| (e.kind.clone(), e.target.clone()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (Kind::IssueClosed, Target::Issue(1)),
                (Kind::IssueReopened, Target::Issue(2)),
                (Kind::IssueOpened, Target::Issue(4)),
                (Kind::MilestoneClosed, Target::Milestone(1)),
                (Kind::MilestoneCreated, Target::Milestone(2)),
                (Kind::PrReady, Target::Pr(10)),
                (Kind::PrMerged, Target::Pr(11)),
                (Kind::PrReview("APPROVED".into()), Target::Pr(12)),
                (Kind::PrOpened, Target::Pr(13)),
                (Kind::RunFinished("failure".into()), Target::Run(100)),
                (Kind::RunStarted, Target::Run(102)),
                (Kind::RunFinished("success".into()), Target::Run(103)),
            ]
        );
        assert!(events.iter().all(|e| e.at == 42));
        assert_eq!(events[0].label, "#1");
        assert_eq!(events[0].verb(), "closed");
        assert_eq!(events[7].verb(), "approved");
        assert_eq!(events[9].verb(), "failed");
        assert_eq!(events[11].verb(), "passed");
        assert!(diff(&next, &next, 43).is_empty(), "no change, no events");
    }
}
