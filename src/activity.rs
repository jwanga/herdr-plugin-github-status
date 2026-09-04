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
                "REVIEW_REQUIRED" => "needs review".into(),
                other => other.to_lowercase().replace('_', " "),
            },
            Kind::PrMerged => "merged".into(),
            Kind::RunStarted => "started".into(),
            Kind::RunFinished(c) => match c.as_str() {
                "success" => "passed".into(),
                "failure" => "failed".into(),
                "action_required" => "action req.".into(),
                other => other.replace('_', " "),
            },
        }
    }
}

/// Transitions from `prev` to `next`, in detection order (issues, milestones, PRs, runs).
///
/// An item absent from `prev` counts as new only if its number is above everything `prev`
/// knew: the fetch windows (300 issues, 50 PRs, 15 runs) slide, so an old item re-entering
/// a window is not "opened". Unknown → known review decisions are not transitions either.
pub fn diff(prev: &Snapshot, next: &Snapshot, now: u64) -> Vec<Event> {
    let mut out = Vec::new();
    let max_seen = prev
        .issues
        .iter()
        .map(|i| i.number)
        .chain(prev.prs.iter().map(|p| p.number))
        .max()
        .unwrap_or(0);
    let max_run = prev.runs.iter().map(|r| r.id).max().unwrap_or(0);

    for i in &next.issues {
        let mut push = |kind: Kind| {
            out.push(Event {
                at: now,
                kind,
                target: Target::Issue(i.number),
                label: format!("#{}", i.number),
                url: Some(i.html_url.clone()),
            })
        };
        match prev.issues.iter().find(|p| p.number == i.number) {
            None if i.is_open() && i.number > max_seen => push(Kind::IssueOpened),
            None => {}
            Some(p) if p.is_open() && !i.is_open() => push(Kind::IssueClosed),
            Some(p) if !p.is_open() && i.is_open() => push(Kind::IssueReopened),
            _ => {}
        }
    }
    for m in &next.milestones {
        let mut push = |kind: Kind| {
            out.push(Event {
                at: now,
                kind,
                target: Target::Milestone(m.number),
                label: m.title.clone(),
                url: Some(m.html_url.clone()),
            })
        };
        match prev.milestones.iter().find(|p| p.number == m.number) {
            None => push(Kind::MilestoneCreated),
            Some(p) if p.state == "open" && m.state != "open" => push(Kind::MilestoneClosed),
            _ => {}
        }
    }
    for p in &next.prs {
        let mut push = |kind: Kind| {
            out.push(Event {
                at: now,
                kind,
                target: Target::Pr(p.number),
                label: format!("#{}", p.number),
                url: Some(p.html_url.clone()),
            })
        };
        match prev.prs.iter().find(|q| q.number == p.number) {
            None if p.is_open() && p.number > max_seen => push(Kind::PrOpened),
            None => {}
            Some(q) => {
                if q.is_open() && p.is_merged() {
                    push(Kind::PrMerged);
                } else if q.is_open() && !p.is_open() {
                    push(Kind::PrClosed);
                }
                if p.is_open() && q.draft && !p.draft {
                    push(Kind::PrReady);
                }
                if p.is_open()
                    && q.extra.review.is_some()
                    && p.extra.review.is_some()
                    && p.extra.review != q.extra.review
                {
                    push(Kind::PrReview(p.extra.review.clone().unwrap_or_default()));
                }
            }
        }
    }
    for r in &next.runs {
        let mut push = |kind: Kind| {
            out.push(Event {
                at: now,
                kind,
                target: Target::Run(r.id),
                label: r.name.clone(),
                url: Some(r.html_url.clone()),
            })
        };
        match prev.runs.iter().find(|q| q.id == r.id) {
            None if r.id <= max_run => {}
            None if r.is_active() => push(Kind::RunStarted),
            None => push(Kind::RunFinished(r.conclusion.clone().unwrap_or_default())),
            Some(q) if q.is_active() && !r.is_active() => {
                push(Kind::RunFinished(r.conclusion.clone().unwrap_or_default()))
            }
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
                issue(14, "open"),
                issue(15, "closed"),
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
                (Kind::IssueOpened, Target::Issue(14)),
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

    #[test]
    fn ignores_window_shifts_and_unknown_reviews() {
        // #5 re-entering the 300-issue window is not "opened"; an old run entering the run
        // window is not "finished"; review None → Some is not a review change.
        let prev = snap(
            vec![issue(9, "open")],
            vec![],
            vec![pr(20, "open", false, false, None)],
            vec![run(50, "completed", Some("success"))],
        );
        let next = snap(
            vec![issue(9, "open"), issue(5, "open")],
            vec![],
            vec![
                pr(20, "open", false, false, Some("APPROVED")),
                pr(8, "open", false, false, None),
            ],
            vec![
                run(50, "completed", Some("success")),
                run(40, "completed", Some("failure")),
            ],
        );
        assert!(diff(&prev, &next, 1).is_empty());
    }
}
