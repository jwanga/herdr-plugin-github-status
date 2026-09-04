//! The section tree: sections → groups → milestones → issues, flattened into visible rows
//! according to which nodes are expanded.

use crate::model::{Issue, Milestone, PullRequest, Snapshot};
use crate::ui::{fit, right_count, truncate};
use crate::util::parse_rfc3339;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

/// How long a closed issue stays in the "recently closed" group.
pub const RECENT_CLOSED_SECS: u64 = 24 * 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Milestones,
    Issues,
    PullRequests,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Milestones => "MILESTONES",
            Section::Issues => "ISSUES",
            Section::PullRequests => "PULL REQUESTS",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeId {
    Section(Section),
    Milestone(u64),
    ClosedMilestones,
    Issue(u64),
    RecentlyClosed,
    Pr(u64),
}

#[derive(Debug, Clone)]
pub struct Row {
    pub id: NodeId,
    /// `Some(expanded)` for nodes that can be toggled.
    pub expandable: Option<bool>,
    pub url: Option<String>,
    pub line: Line<'static>,
}

/// Expansion state as a set of nodes toggled away from their default.
#[derive(Debug, Default, Clone)]
pub struct TreeState {
    toggled: HashSet<NodeId>,
}

impl TreeState {
    fn default_open(id: &NodeId) -> bool {
        matches!(id, NodeId::Section(_) | NodeId::Milestone(_))
    }
    pub fn is_open(&self, id: &NodeId) -> bool {
        Self::default_open(id) ^ self.toggled.contains(id)
    }
    pub fn toggle(&mut self, id: &NodeId) {
        if !self.toggled.remove(id) {
            self.toggled.insert(id.clone());
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn recently_closed(issue: &Issue, now: u64) -> bool {
    issue
        .closed_at
        .as_deref()
        .and_then(parse_rfc3339)
        .is_some_and(|t| now.saturating_sub(t) <= RECENT_CLOSED_SECS)
}

fn issue_icon(i: &Issue) -> (&'static str, Color) {
    match (i.state.as_str(), i.state_reason.as_deref()) {
        ("open", _) => ("●", Color::Green),
        (_, Some("not_planned")) => ("⊘", Color::DarkGray),
        _ => ("✓", Color::Magenta),
    }
}

fn prefix(depth: usize, expandable: Option<bool>) -> String {
    let indent = " ".repeat(depth);
    match expandable {
        Some(true) => format!("{indent}▾ "),
        Some(false) => format!("{indent}▸ "),
        None => format!("{indent}  "),
    }
}

fn section_line(section: Section, open: bool, count: String, w: usize) -> Line<'static> {
    let head = format!("{}{}", prefix(0, Some(open)), section.title());
    let width = head.chars().count();
    right_count(vec![Span::styled(head, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))], width, count, w)
}

fn group_line(depth: usize, open: bool, label: &str, count: usize, w: usize) -> Line<'static> {
    let count = count.to_string();
    let head = format!("{}{}", prefix(depth, Some(open)), truncate(label, w.saturating_sub(depth + 2 + count.len() + 1)));
    let width = head.chars().count();
    right_count(vec![Span::styled(head, Style::default().fg(Color::DarkGray))], width, count, w)
}

fn milestone_line(m: &Milestone, depth: usize, open: bool, w: usize) -> Line<'static> {
    let count = format!("{}/{}", m.closed_issues, m.total());
    let bar = if w >= 32 {
        let cells = 6usize;
        let filled = if m.total() == 0 { 0 } else { (m.closed_issues as usize * cells) / m.total() as usize };
        format!(" {}{}", "▓".repeat(filled), "░".repeat(cells - filled))
    } else {
        String::new()
    };
    let tail = format!("{count}{bar}");
    let pre = prefix(depth, Some(open));
    let title = truncate(&m.title, w.saturating_sub(pre.chars().count() + tail.chars().count() + 1));
    let width = pre.chars().count() + title.chars().count();
    let style = if m.state == "open" { Style::default().add_modifier(Modifier::BOLD) } else { Style::default().fg(Color::DarkGray) };
    right_count(vec![Span::raw(pre), Span::styled(title, style)], width, tail, w)
}

fn issue_line(i: &Issue, depth: usize, w: usize) -> Line<'static> {
    let (icon, color) = issue_icon(i);
    let num = format!("#{}", i.number);
    let who = if w >= 36 {
        i.assignees.first().map(|u| format!(" @{}", truncate(&u.login, 8))).unwrap_or_default()
    } else {
        String::new()
    };
    let pre = " ".repeat(depth);
    let title_w = w.saturating_sub(pre.len() + 2 + num.len() + 1 + who.chars().count());
    let title_style = if i.is_open() { Style::default() } else { Style::default().fg(Color::DarkGray) };
    Line::from(vec![
        Span::raw(pre),
        Span::styled(format!("{icon} "), Style::default().fg(color)),
        Span::styled(num, Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(fit(&i.title, title_w), title_style),
        Span::styled(who, Style::default().fg(Color::DarkGray)),
    ])
}

fn pr_line(p: &PullRequest, depth: usize, w: usize) -> Line<'static> {
    let (icon, color) = if p.draft { ("◌", Color::DarkGray) } else { ("⇄", Color::Green) };
    let num = format!("#{}", p.number);
    let pre = " ".repeat(depth);
    Line::from(vec![
        Span::raw(pre.clone()),
        Span::styled(format!("{icon} "), Style::default().fg(color)),
        Span::styled(num.clone(), Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::raw(truncate(&p.title, w.saturating_sub(pre.len() + 2 + num.len() + 1))),
    ])
}

/// Flatten the snapshot into the visible rows for `state` at width `w`.
pub fn rows(s: &Snapshot, state: &TreeState, w: usize) -> Vec<Row> {
    let now = now_secs();
    let mut out = Vec::new();
    let repo_url = format!("https://github.com/{}/{}", s.repo.owner, s.repo.name);

    // ---- Milestones
    let mut milestones: Vec<&Milestone> = s.milestones.iter().collect();
    milestones.sort_by(|a, b| {
        (a.state != "open")
            .cmp(&(b.state != "open"))
            .then_with(|| a.due_on.is_none().cmp(&b.due_on.is_none()))
            .then_with(|| a.due_on.cmp(&b.due_on))
            .then_with(|| a.number.cmp(&b.number))
    });
    let open_ms: Vec<&Milestone> = milestones.iter().copied().filter(|m| m.state == "open").collect();
    let closed_ms: Vec<&Milestone> = milestones.iter().copied().filter(|m| m.state != "open").collect();
    let sid = NodeId::Section(Section::Milestones);
    let open = state.is_open(&sid);
    out.push(Row {
        id: sid.clone(),
        expandable: Some(open),
        url: Some(format!("{repo_url}/milestones")),
        line: section_line(Section::Milestones, open, open_ms.len().to_string(), w),
    });
    if open {
        for m in &open_ms {
            push_milestone(&mut out, s, state, m, w);
        }
        if !closed_ms.is_empty() {
            let gid = NodeId::ClosedMilestones;
            let g_open = state.is_open(&gid);
            out.push(Row {
                id: gid,
                expandable: Some(g_open),
                url: Some(format!("{repo_url}/milestones?state=closed")),
                line: group_line(1, g_open, "closed", closed_ms.len(), w),
            });
            if g_open {
                for m in &closed_ms {
                    push_milestone(&mut out, s, state, m, w);
                }
            }
        }
    }

    // ---- Issues without a milestone
    let mut unassigned: Vec<&Issue> = s.issues.iter().filter(|i| i.milestone.is_none() && i.is_open()).collect();
    unassigned.sort_by_key(|i| i.number);
    let mut recent: Vec<&Issue> = s
        .issues
        .iter()
        .filter(|i| i.milestone.is_none() && !i.is_open() && recently_closed(i, now))
        .collect();
    recent.sort_by(|a, b| b.closed_at.cmp(&a.closed_at));
    let sid = NodeId::Section(Section::Issues);
    let open = state.is_open(&sid);
    out.push(Row {
        id: sid.clone(),
        expandable: Some(open),
        url: Some(format!("{repo_url}/issues")),
        line: section_line(Section::Issues, open, format!("{} open", unassigned.len()), w),
    });
    if open {
        for i in &unassigned {
            out.push(issue_row(i, 1, w));
        }
        if !recent.is_empty() {
            let gid = NodeId::RecentlyClosed;
            let g_open = state.is_open(&gid);
            out.push(Row {
                id: gid,
                expandable: Some(g_open),
                url: Some(format!("{repo_url}/issues?q=is%3Aissue+is%3Aclosed+no%3Amilestone")),
                line: group_line(1, g_open, "recently closed", recent.len(), w),
            });
            if g_open {
                for i in &recent {
                    out.push(issue_row(i, 2, w));
                }
            }
        }
    }

    // ---- Pull requests
    let mut open_prs: Vec<&PullRequest> = s.prs.iter().filter(|p| p.is_open()).collect();
    open_prs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    let sid = NodeId::Section(Section::PullRequests);
    let open = state.is_open(&sid);
    out.push(Row {
        id: sid.clone(),
        expandable: Some(open),
        url: Some(format!("{repo_url}/pulls")),
        line: section_line(Section::PullRequests, open, format!("{} open", open_prs.len()), w),
    });
    if open {
        for p in &open_prs {
            out.push(Row { id: NodeId::Pr(p.number), expandable: None, url: Some(p.html_url.clone()), line: pr_line(p, 1, w) });
        }
    }
    out
}

fn issue_row(i: &Issue, depth: usize, w: usize) -> Row {
    Row { id: NodeId::Issue(i.number), expandable: None, url: Some(i.html_url.clone()), line: issue_line(i, depth, w) }
}

fn push_milestone(out: &mut Vec<Row>, s: &Snapshot, state: &TreeState, m: &Milestone, w: usize) {
    let id = NodeId::Milestone(m.number);
    let open = state.is_open(&id);
    out.push(Row { id: id.clone(), expandable: Some(open), url: Some(m.html_url.clone()), line: milestone_line(m, 1, open, w) });
    if open {
        let mut issues: Vec<&Issue> = s.issues.iter().filter(|i| i.milestone.as_ref().is_some_and(|r| r.number == m.number)).collect();
        issues.sort_by_key(|i| (!i.is_open(), i.number));
        for i in issues {
            out.push(issue_row(i, 2, w));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GitRef, MilestoneRef, Snapshot};
    use crate::repo::RepoRef;

    fn ms(number: u64, title: &str, state: &str, open: u64, closed: u64) -> Milestone {
        Milestone { number, title: title.into(), state: state.into(), open_issues: open, closed_issues: closed, due_on: None, html_url: format!("m{number}"), updated_at: String::new() }
    }
    fn issue(number: u64, title: &str, state: &str, milestone: Option<u64>, closed_at: Option<&str>) -> Issue {
        Issue { number, title: title.into(), state: state.into(), state_reason: (state == "closed").then(|| "completed".to_string()), milestone: milestone.map(|n| MilestoneRef { number: n }), labels: vec![], assignees: vec![], updated_at: String::new(), closed_at: closed_at.map(str::to_string), html_url: format!("i{number}"), pull_request: None }
    }
    fn pr(number: u64, draft: bool) -> PullRequest {
        PullRequest { number, title: format!("PR {number}"), state: "open".into(), draft, merged_at: None, head: GitRef { name: "b".into(), sha: String::new() }, base: GitRef { name: "main".into(), sha: String::new() }, user: None, updated_at: String::new(), html_url: format!("p{number}"), body: None }
    }
    fn snap() -> Snapshot {
        let recent = {
            let t = now_secs() - 3600;
            let days = t / 86_400;
            // Any RFC3339 in the last day works; build from epoch days (all in 2026+).
            let _ = days;
            "2999-01-01T00:00:00Z".to_string() // future => treated as recent (saturating_sub → 0)
        };
        Snapshot {
            repo: RepoRef { owner: "o".into(), name: "r".into(), branch: None, root: String::new() },
            milestones: vec![ms(2, "Second", "open", 1, 0), ms(1, "First", "open", 1, 1), ms(3, "Old", "closed", 0, 2)],
            issues: vec![
                issue(10, "in second", "open", Some(2), None),
                issue(4, "first closed", "closed", Some(1), Some("2020-01-01T00:00:00Z")),
                issue(5, "first open", "open", Some(1), None),
                issue(7, "loose", "open", None, None),
                issue(8, "just closed", "closed", None, Some(&recent)),
                issue(9, "closed long ago", "closed", None, Some("2020-01-01T00:00:00Z")),
            ],
            prs: vec![pr(20, false), pr(21, true)],
            fetched_at: SystemTime::now(),
            rate_remaining: None,
            authenticated: true,
        }
    }
    fn text(r: &Row) -> String {
        r.line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn default_expansion_and_ordering() {
        let s = snap();
        let rows = rows(&s, &TreeState::default(), 26);
        let t: Vec<String> = rows.iter().map(text).collect();
        assert!(t.iter().all(|l| l.chars().count() <= 26), "{t:?}");
        let ids: Vec<&NodeId> = rows.iter().map(|r| &r.id).collect();
        // Milestones by number (no due dates), open before closed, issues open-first by number.
        assert_eq!(ids[0], &NodeId::Section(Section::Milestones));
        assert_eq!(ids[1], &NodeId::Milestone(1));
        assert_eq!(ids[2], &NodeId::Issue(5));
        assert_eq!(ids[3], &NodeId::Issue(4));
        assert_eq!(ids[4], &NodeId::Milestone(2));
        assert_eq!(ids[5], &NodeId::Issue(10));
        assert_eq!(ids[6], &NodeId::ClosedMilestones, "closed milestones collapsed by default");
        assert_eq!(ids[7], &NodeId::Section(Section::Issues));
        assert_eq!(ids[8], &NodeId::Issue(7));
        assert_eq!(ids[9], &NodeId::RecentlyClosed);
        assert_eq!(ids[10], &NodeId::Section(Section::PullRequests));
        assert_eq!(ids[11], &NodeId::Pr(20));
        assert_eq!(ids.len(), 13);
        assert!(t[1].starts_with(" ▾ First") && t[1].ends_with("1/2"), "{:?}", t[1]);
        assert!(t[0].starts_with("▾ MILESTONES") && t[0].ends_with(" 2"));
    }

    #[test]
    fn toggling_changes_visible_rows() {
        let s = snap();
        let mut st = TreeState::default();
        st.toggle(&NodeId::Milestone(1));
        let ids: Vec<NodeId> = rows(&s, &st, 26).into_iter().map(|r| r.id).collect();
        assert!(!ids.contains(&NodeId::Issue(5)));
        st.toggle(&NodeId::ClosedMilestones);
        st.toggle(&NodeId::RecentlyClosed);
        let ids: Vec<NodeId> = rows(&s, &st, 26).into_iter().map(|r| r.id).collect();
        assert!(ids.contains(&NodeId::Milestone(3)));
        assert!(ids.contains(&NodeId::Issue(8)));
        assert!(!ids.contains(&NodeId::Issue(9)), "old closed issues are not 'recent'");
        st.toggle(&NodeId::Section(Section::Milestones));
        let ids: Vec<NodeId> = rows(&s, &st, 26).into_iter().map(|r| r.id).collect();
        assert_eq!(ids.iter().filter(|i| matches!(i, NodeId::Milestone(_))).count(), 0);
    }

    #[test]
    fn wide_rows_show_bar_and_assignee() {
        let mut s = snap();
        s.issues[2].assignees = vec![crate::model::User { login: "someone".into() }];
        let rows = rows(&s, &TreeState::default(), 40);
        let t: Vec<String> = rows.iter().map(text).collect();
        assert!(t[1].contains("▓▓▓░░░"), "{:?}", t[1]);
        assert!(t[2].contains("@someone"), "{:?}", t[2]);
        assert!(t.iter().all(|l| l.chars().count() <= 40), "{t:?}");
    }
}
