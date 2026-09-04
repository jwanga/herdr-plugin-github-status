//! The section tree: sections → groups → milestones → issues, flattened into nodes
//! according to which nodes are expanded, and rendered per node at a given width.

use crate::model::{AgentInfo, Issue, Milestone, PullRequest, Snapshot};
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
    Now,
    Milestones,
    Issues,
    PullRequests,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Now => "NOW",
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
    RecentPrs,
    /// A detail line under a PR (branch, linked issue).
    PrDetail(u64, usize),
    NowIssue(u64),
    NowPr(u64),
    Agent(String),
    Idle,
}

#[derive(Debug, Clone)]
pub enum NodeKind {
    Section { section: Section, count: String },
    Group { label: &'static str, count: usize },
    Milestone(Milestone),
    /// `active` marks the issue being worked on (branch or open PR).
    Issue { issue: Issue, active: bool },
    Pr(PullRequest),
    Agent(AgentInfo),
    /// A dim informational line.
    Info(String),
}

/// A visible row of the tree, independent of render width.
#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    pub depth: usize,
    /// `Some(expanded)` for nodes that can be toggled.
    pub expandable: Option<bool>,
    pub url: Option<String>,
    pub kind: NodeKind,
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
    /// Force a node open or closed.
    pub fn set_open(&mut self, id: &NodeId, open: bool) {
        if self.is_open(id) != open {
            self.toggle(id);
        }
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn recently_closed(issue: &Issue, now: u64) -> bool {
    issue
        .closed_at
        .as_deref()
        .and_then(parse_rfc3339)
        .is_some_and(|t| now.saturating_sub(t) <= RECENT_CLOSED_SECS)
}

/// Issue number from a branch named `issue-<n>-…` (the engineering-plugin convention).
pub fn issue_from_branch(branch: &str) -> Option<u64> {
    let rest = branch.strip_prefix("issue-")?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() || !rest[digits.len()..].starts_with(['-', '/']) && rest.len() != digits.len() {
        return None;
    }
    digits.parse().ok()
}

/// The issue being worked on: from the branch name, else the first issue the current
/// branch's open PR closes.
pub fn active_issue(s: &Snapshot) -> Option<u64> {
    let branch = s.repo.branch.as_deref()?;
    issue_from_branch(branch).or_else(|| current_pr(s).and_then(|p| p.extra.closes.first().copied()))
}

/// The open PR whose head is the current branch.
pub fn current_pr(s: &Snapshot) -> Option<&PullRequest> {
    let branch = s.repo.branch.as_deref()?;
    s.prs.iter().find(|p| p.is_open() && p.head.name == branch)
}

fn recently_closed_pr(p: &PullRequest, now: u64) -> bool {
    p.closed_at
        .as_deref()
        .or(p.merged_at.as_deref())
        .and_then(parse_rfc3339)
        .is_some_and(|t| now.saturating_sub(t) <= RECENT_CLOSED_SECS)
}

/// Flatten the snapshot into visible nodes for `state`; `now` (Unix seconds) bounds the
/// recently-closed groups; `agents` are this workspace's herdr agents.
pub fn flatten(s: &Snapshot, state: &TreeState, now: u64, agents: &[AgentInfo]) -> Vec<Node> {
    let mut out = Vec::new();
    let repo_url = format!("https://github.com/{}/{}", s.repo.owner, s.repo.name);
    let active = active_issue(s);

    // ---- Now: the active issue, the current branch's PR, and the workspace's agents.
    let now_issue = active.and_then(|n| s.issues.iter().find(|i| i.number == n));
    let now_pr = current_pr(s);
    let busy = agents.iter().filter(|a| a.status == "working" || a.status == "blocked").count();
    let count = if now_issue.is_none() && now_pr.is_none() && agents.is_empty() { "idle".to_string() } else { format!("{busy} busy") };
    if section(&mut out, state, Section::Now, count, format!("{repo_url}/pulls")) {
        if let Some(i) = now_issue {
            out.push(Node { id: NodeId::NowIssue(i.number), depth: 1, expandable: None, url: Some(i.html_url.clone()), kind: NodeKind::Issue { issue: i.clone(), active: true } });
        } else if let Some(n) = active {
            out.push(Node { id: NodeId::NowIssue(n), depth: 1, expandable: None, url: Some(format!("{repo_url}/issues/{n}")), kind: NodeKind::Info(format!("▶ #{n} (not in view)")) });
        }
        if let Some(p) = now_pr {
            out.push(Node { id: NodeId::NowPr(p.number), depth: 1, expandable: None, url: Some(p.html_url.clone()), kind: NodeKind::Pr(p.clone()) });
        }
        for a in agents {
            out.push(Node { id: NodeId::Agent(a.pane_id.clone()), depth: 1, expandable: None, url: None, kind: NodeKind::Agent(a.clone()) });
        }
        if now_issue.is_none() && active.is_none() && now_pr.is_none() && agents.is_empty() {
            out.push(Node { id: NodeId::Idle, depth: 1, expandable: None, url: None, kind: NodeKind::Info("nothing in progress".into()) });
        }
    }

    // ---- Milestones: by due date (undated last), then number; closed ones in a group.
    let mut milestones: Vec<&Milestone> = s.milestones.iter().collect();
    milestones.sort_by(|a, b| {
        a.due_on
            .is_none()
            .cmp(&b.due_on.is_none())
            .then_with(|| a.due_on.cmp(&b.due_on))
            .then_with(|| a.number.cmp(&b.number))
    });
    let (open_ms, closed_ms): (Vec<&Milestone>, Vec<&Milestone>) = milestones.iter().partition(|m| m.state == "open");
    if section(&mut out, state, Section::Milestones, open_ms.len().to_string(), format!("{repo_url}/milestones")) {
        for m in &open_ms {
            push_milestone(&mut out, s, state, m, active);
        }
        if !closed_ms.is_empty()
            && group(&mut out, state, NodeId::ClosedMilestones, "closed", closed_ms.len(), format!("{repo_url}/milestones?state=closed"))
        {
            for m in &closed_ms {
                push_milestone(&mut out, s, state, m, active);
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
    if section(&mut out, state, Section::Issues, format!("{} open", unassigned.len()), format!("{repo_url}/issues")) {
        for i in &unassigned {
            out.push(issue_node(i, 1, active));
        }
        if !recent.is_empty()
            && group(
                &mut out,
                state,
                NodeId::RecentlyClosed,
                "recently closed",
                recent.len(),
                format!("{repo_url}/issues?q=is%3Aissue+is%3Aclosed+no%3Amilestone"),
            )
        {
            for i in &recent {
                out.push(issue_node(i, 2, active));
            }
        }
    }

    // ---- Pull requests
    let mut open_prs: Vec<&PullRequest> = s.prs.iter().filter(|p| p.is_open()).collect();
    open_prs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    let mut recent_prs: Vec<&PullRequest> = s.prs.iter().filter(|p| !p.is_open() && recently_closed_pr(p, now)).collect();
    recent_prs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    if section(&mut out, state, Section::PullRequests, format!("{} open", open_prs.len()), format!("{repo_url}/pulls")) {
        for p in &open_prs {
            push_pr(&mut out, s, state, p, &repo_url);
        }
        if !recent_prs.is_empty()
            && group(&mut out, state, NodeId::RecentPrs, "recently merged/closed", recent_prs.len(), format!("{repo_url}/pulls?q=is%3Apr+is%3Aclosed"))
        {
            for p in &recent_prs {
                push_pr(&mut out, s, state, p, &repo_url);
            }
        }
    }
    out
}

/// Push a section node; returns whether it is open.
fn section(out: &mut Vec<Node>, state: &TreeState, section: Section, count: String, url: String) -> bool {
    let id = NodeId::Section(section);
    let open = state.is_open(&id);
    out.push(Node { id, depth: 0, expandable: Some(open), url: Some(url), kind: NodeKind::Section { section, count } });
    open
}

/// Push a group node (depth 1); returns whether it is open.
fn group(out: &mut Vec<Node>, state: &TreeState, id: NodeId, label: &'static str, count: usize, url: String) -> bool {
    let open = state.is_open(&id);
    out.push(Node { id, depth: 1, expandable: Some(open), url: Some(url), kind: NodeKind::Group { label, count } });
    open
}

fn issue_node(i: &Issue, depth: usize, active: Option<u64>) -> Node {
    Node {
        id: NodeId::Issue(i.number),
        depth,
        expandable: None,
        url: Some(i.html_url.clone()),
        kind: NodeKind::Issue { issue: i.clone(), active: active == Some(i.number) },
    }
}

fn push_milestone(out: &mut Vec<Node>, s: &Snapshot, state: &TreeState, m: &Milestone, active: Option<u64>) {
    let id = NodeId::Milestone(m.number);
    let open = state.is_open(&id);
    out.push(Node { id, depth: 1, expandable: Some(open), url: Some(m.html_url.clone()), kind: NodeKind::Milestone(m.clone()) });
    if open {
        let mut issues: Vec<&Issue> = s.issues.iter().filter(|i| i.milestone.as_ref().is_some_and(|r| r.number == m.number)).collect();
        issues.sort_by_key(|i| (!i.is_open(), i.number));
        for i in issues {
            out.push(issue_node(i, 2, active));
        }
    }
}

/// A PR node plus, when expanded, its branch line and linked issues.
fn push_pr(out: &mut Vec<Node>, s: &Snapshot, state: &TreeState, p: &PullRequest, repo_url: &str) {
    let id = NodeId::Pr(p.number);
    let open = state.is_open(&id);
    out.push(Node { id, depth: 1, expandable: Some(open), url: Some(p.html_url.clone()), kind: NodeKind::Pr(p.clone()) });
    if open {
        out.push(Node {
            id: NodeId::PrDetail(p.number, 0),
            depth: 2,
            expandable: None,
            url: Some(format!("{repo_url}/tree/{}", p.head.name)),
            kind: NodeKind::Info(format!("⎇ {} → {}", p.head.name, p.base.name)),
        });
        for (k, n) in p.extra.closes.iter().enumerate() {
            let title = s.issues.iter().find(|i| i.number == *n).map(|i| format!(" {}", i.title)).unwrap_or_default();
            out.push(Node {
                id: NodeId::PrDetail(p.number, k + 1),
                depth: 2,
                expandable: None,
                url: Some(format!("{repo_url}/issues/{n}")),
                kind: NodeKind::Info(format!("closes #{n}{title}")),
            });
        }
    }
}

// ---------------------------------------------------------------------------- rendering

fn prefix(depth: usize, open: bool) -> String {
    format!("{}{} ", " ".repeat(depth), if open { '▾' } else { '▸' })
}

fn issue_icon(i: &Issue, active: bool) -> (&'static str, Color) {
    match (i.state.as_str(), i.state_reason.as_deref()) {
        ("open", _) if active => ("▶", Color::Yellow),
        ("open", _) => ("●", Color::Green),
        (_, Some("not_planned")) => ("⊘", Color::DarkGray),
        _ => ("✓", Color::Magenta),
    }
}

fn pr_icon(p: &PullRequest) -> (&'static str, Color) {
    if p.is_merged() {
        ("⭳", Color::Magenta)
    } else if !p.is_open() {
        ("⊘", Color::DarkGray)
    } else if p.draft {
        ("◌", Color::DarkGray)
    } else {
        ("⇄", Color::Green)
    }
}

/// Two-glyph tail: review decision then checks state; `·` when unknown.
pub fn pr_tail(p: &PullRequest) -> String {
    if !p.is_open() {
        return String::new();
    }
    let review = match p.extra.review.as_deref() {
        Some("APPROVED") => 'A',
        Some("CHANGES_REQUESTED") => 'C',
        Some("REVIEW_REQUIRED") => 'R',
        _ => '·',
    };
    let checks = match p.extra.checks.as_ref() {
        Some(c) if c.failed > 0 => '✗',
        Some(c) if c.pending > 0 => '◔',
        Some(c) if c.total > 0 => '✓',
        _ => '·',
    };
    format!(" {review}{checks}")
}

fn agent_icon(status: &str) -> (&'static str, Color) {
    match status {
        "working" => ("◐", Color::Cyan),
        "blocked" => ("■", Color::Red),
        "done" => ("✓", Color::Green),
        "idle" => ("○", Color::DarkGray),
        _ => ("?", Color::DarkGray),
    }
}

/// Two-character initials for an assignee login.
fn initials(login: &str) -> String {
    login.chars().filter(|c| c.is_alphanumeric()).take(2).collect::<String>().to_uppercase()
}

/// `<indent><icon> #n <title…><suffix>` sized to exactly `w` columns.
#[allow(clippy::too_many_arguments)]
fn item_line(depth: usize, icon: &str, color: Color, number: u64, title: &str, title_style: Style, suffix: String, w: usize) -> Line<'static> {
    let num = format!("#{number}");
    let pre = " ".repeat(depth);
    let title_w = w.saturating_sub(pre.len() + 2 + num.len() + 1 + suffix.chars().count());
    Line::from(vec![
        Span::raw(pre),
        Span::styled(format!("{icon} "), Style::default().fg(color)),
        Span::styled(num, Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(fit(title, title_w), title_style),
        Span::styled(suffix, Style::default().fg(Color::DarkGray)),
    ])
}

/// Render one node at width `w`.
pub fn render(node: &Node, w: usize) -> Line<'static> {
    let open = node.expandable.unwrap_or(false);
    match &node.kind {
        NodeKind::Section { section, count } => {
            let head = format!("{}{}", prefix(0, open), section.title());
            let width = head.chars().count();
            right_count(vec![Span::styled(head, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))], width, count.clone(), w)
        }
        NodeKind::Group { label, count } => {
            let count = count.to_string();
            let pre = prefix(node.depth, open);
            let head = format!("{pre}{}", truncate(label, w.saturating_sub(pre.chars().count() + count.len() + 1)));
            let width = head.chars().count();
            right_count(vec![Span::styled(head, Style::default().fg(Color::DarkGray))], width, count, w)
        }
        NodeKind::Milestone(m) => {
            let count = format!("{}/{}", m.closed_issues, m.total());
            let bar = if w >= 32 {
                let cells = 6usize;
                let filled = if m.total() == 0 { 0 } else { (m.closed_issues as usize * cells) / m.total() as usize };
                format!(" {}{}", "▓".repeat(filled), "░".repeat(cells - filled))
            } else {
                String::new()
            };
            let tail = format!("{count}{bar}");
            let pre = prefix(node.depth, open);
            let title = truncate(&m.title, w.saturating_sub(pre.chars().count() + tail.chars().count() + 1));
            let width = pre.chars().count() + title.chars().count();
            let style = if m.state == "open" { Style::default().add_modifier(Modifier::BOLD) } else { Style::default().fg(Color::DarkGray) };
            right_count(vec![Span::raw(pre), Span::styled(title, style)], width, tail, w)
        }
        NodeKind::Issue { issue: i, active } => {
            let (icon, color) = issue_icon(i, *active);
            let who = match i.assignees.first() {
                Some(u) if w >= 30 => format!(" {}", initials(&u.login)),
                _ => String::new(),
            };
            let style = if *active {
                Style::default().add_modifier(Modifier::BOLD)
            } else if i.is_open() {
                Style::default()
            } else {
                Style::default().fg(Color::DarkGray)
            };
            item_line(node.depth, icon, color, i.number, &i.title, style, who, w)
        }
        NodeKind::Pr(p) => {
            let (icon, color) = pr_icon(p);
            let style = if p.is_open() { Style::default() } else { Style::default().fg(Color::DarkGray) };
            item_line(node.depth, icon, color, p.number, &p.title, style, pr_tail(p), w)
        }
        NodeKind::Agent(a) => {
            let (icon, color) = agent_icon(&a.status);
            let pre = " ".repeat(node.depth);
            let head = format!("{icon} {} ", a.agent);
            let status = a.status.clone();
            let title_w = w.saturating_sub(pre.len() + head.chars().count() + status.len());
            let title = fit(&a.title.clone().unwrap_or_default(), title_w);
            Line::from(vec![
                Span::raw(pre),
                Span::styled(head, Style::default().fg(color)),
                Span::styled(title, Style::default().fg(Color::DarkGray)),
                Span::styled(status, Style::default().fg(color)),
            ])
        }
        NodeKind::Info(text) => {
            let pre = " ".repeat(node.depth);
            Line::from(vec![Span::raw(pre.clone()), Span::styled(fit(text, w.saturating_sub(pre.len())), Style::default().fg(Color::DarkGray))])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Checks, GitRef, MilestoneRef, PrExtra, Snapshot, User};
    use crate::repo::RepoRef;

    const NOW: u64 = 1_788_507_932; // 2026-09-04T07:45:32Z

    fn ms(number: u64, title: &str, state: &str, open: u64, closed: u64) -> Milestone {
        Milestone { number, title: title.into(), state: state.into(), open_issues: open, closed_issues: closed, due_on: None, html_url: format!("https://m/{number}"), updated_at: String::new() }
    }
    fn issue(number: u64, title: &str, state: &str, milestone: Option<u64>, closed_at: Option<&str>) -> Issue {
        Issue { number, title: title.into(), state: state.into(), state_reason: (state == "closed").then(|| "completed".to_string()), milestone: milestone.map(|n| MilestoneRef { number: n }), labels: vec![], assignees: vec![], updated_at: String::new(), closed_at: closed_at.map(str::to_string), html_url: format!("https://i/{number}"), pull_request: None }
    }
    fn pr(number: u64, draft: bool) -> PullRequest {
        PullRequest { number, title: format!("PR {number}"), state: "open".into(), draft, merged_at: None, closed_at: None, head: GitRef { name: format!("branch-{number}"), sha: String::new() }, base: GitRef { name: "main".into(), sha: String::new() }, user: None, updated_at: String::new(), html_url: format!("https://p/{number}"), body: None, extra: PrExtra::default() }
    }
    fn nodes(s: &Snapshot, st: &TreeState, now: u64) -> Vec<Node> {
        flatten(s, st, now, &[])
    }
    fn snap() -> Snapshot {
        Snapshot {
            repo: RepoRef { owner: "o".into(), name: "r".into(), branch: None, root: String::new() },
            milestones: vec![ms(2, "Second", "open", 1, 0), ms(1, "First", "open", 1, 1), ms(3, "Old", "closed", 0, 2)],
            issues: vec![
                issue(10, "in second", "open", Some(2), None),
                issue(4, "first closed", "closed", Some(1), Some("2020-01-01T00:00:00Z")),
                issue(5, "first open", "open", Some(1), None),
                issue(7, "loose", "open", None, None),
                issue(8, "just closed", "closed", None, Some("2026-09-04T00:00:00Z")), // ~8 h before NOW
                issue(9, "closed 25h ago", "closed", None, Some("2026-09-03T06:45:00Z")),
            ],
            prs: vec![pr(20, false), pr(21, true)],
            fetched_at: SystemTime::now(),
            rate_remaining: None,
            authenticated: true,
        }
    }
    fn texts(nodes: &[Node], w: usize) -> Vec<String> {
        nodes.iter().map(|n| render(n, w).spans.iter().map(|s| s.content.as_ref()).collect()).collect()
    }

    #[test]
    fn default_expansion_ordering_and_urls() {
        let s = snap();
        let nodes = nodes(&s, &TreeState::default(), NOW);
        let t = texts(&nodes, 26);
        assert!(t.iter().all(|l| l.chars().count() <= 26), "{t:?}");
        let ids: Vec<&NodeId> = nodes.iter().map(|n| &n.id).collect();
        assert_eq!(ids[0], &NodeId::Section(Section::Now));
        assert_eq!(ids[1], &NodeId::Idle, "no branch, no agents: idle");
        assert_eq!(ids[2], &NodeId::Section(Section::Milestones));
        assert_eq!(ids[3], &NodeId::Milestone(1));
        assert_eq!(ids[4], &NodeId::Issue(5));
        assert_eq!(ids[5], &NodeId::Issue(4));
        assert_eq!(ids[6], &NodeId::Milestone(2));
        assert_eq!(ids[7], &NodeId::Issue(10));
        assert_eq!(ids[8], &NodeId::ClosedMilestones, "closed milestones collapsed by default");
        assert_eq!(ids[9], &NodeId::Section(Section::Issues));
        assert_eq!(ids[10], &NodeId::Issue(7));
        assert_eq!(ids[11], &NodeId::RecentlyClosed);
        assert_eq!(ids[12], &NodeId::Section(Section::PullRequests));
        assert_eq!(ids[13], &NodeId::Pr(20));
        assert_eq!(ids.len(), 15);
        assert!(t[3].starts_with(" ▾ First") && t[3].ends_with("1/2"), "{:?}", t[3]);
        assert!(t[2].starts_with("▾ MILESTONES") && t[2].ends_with(" 2"));
        assert!(t[0].ends_with("idle"), "{:?}", t[0]);
        assert_eq!(nodes[2].url.as_deref(), Some("https://github.com/o/r/milestones"));
        assert_eq!(nodes[3].url.as_deref(), Some("https://m/1"));
        assert_eq!(nodes[4].url.as_deref(), Some("https://i/5"));
        assert_eq!(nodes[13].url.as_deref(), Some("https://p/20"));
        assert!(t[13].ends_with(" ··"), "unknown review/checks: {:?}", t[13]);
    }

    #[test]
    fn toggling_and_recent_window() {
        let s = snap();
        let mut st = TreeState::default();
        st.toggle(&NodeId::Milestone(1));
        let ids: Vec<NodeId> = nodes(&s, &st, NOW).into_iter().map(|n| n.id).collect();
        assert!(!ids.contains(&NodeId::Issue(5)));
        st.toggle(&NodeId::ClosedMilestones);
        st.toggle(&NodeId::RecentlyClosed);
        let ids: Vec<NodeId> = nodes(&s, &st, NOW).into_iter().map(|n| n.id).collect();
        assert!(ids.contains(&NodeId::Milestone(3)));
        assert!(ids.contains(&NodeId::Issue(8)), "closed 8 h ago is recent");
        assert!(!ids.contains(&NodeId::Issue(9)), "closed 25 h ago is not");
        // 20 h later #8 also ages out and the group disappears.
        let ids: Vec<NodeId> = nodes(&s, &st, NOW + 20 * 3600).into_iter().map(|n| n.id).collect();
        assert!(!ids.contains(&NodeId::RecentlyClosed));
        st.toggle(&NodeId::Section(Section::Milestones));
        let ids: Vec<NodeId> = nodes(&s, &st, NOW).into_iter().map(|n| n.id).collect();
        assert_eq!(ids.iter().filter(|i| matches!(i, NodeId::Milestone(_))).count(), 0);
        st.set_open(&NodeId::Section(Section::Milestones), true);
        assert!(st.is_open(&NodeId::Section(Section::Milestones)));
    }

    #[test]
    fn rows_fill_width_exactly_and_show_bar_and_initials() {
        let mut s = snap();
        s.issues[2].assignees = vec![User { login: "some-one".into() }];
        let nodes = nodes(&s, &TreeState::default(), NOW);
        for w in [26usize, 32, 40] {
            let t = texts(&nodes, w);
            assert!(t.iter().all(|l| l.chars().count() == w), "w={w}: {t:?}");
        }
        let t = texts(&nodes, 40);
        assert!(t[3].contains("▓▓▓░░░"), "{:?}", t[3]);
        assert!(t[4].ends_with(" SO"), "{:?}", t[4]);
        let t = texts(&nodes, 26);
        assert!(!t[3].contains('▓') && !t[4].contains("SO"), "{:?} {:?}", t[3], t[4]);
    }

    #[test]
    fn branch_and_pr_detection() {
        assert_eq!(issue_from_branch("issue-12-scaffold"), Some(12));
        assert_eq!(issue_from_branch("issue-7"), Some(7));
        assert_eq!(issue_from_branch("issue-7/x"), Some(7));
        assert_eq!(issue_from_branch("issues-12"), None);
        assert_eq!(issue_from_branch("issue-abc"), None);
        assert_eq!(issue_from_branch("main"), None);
        let mut s = snap();
        s.repo.branch = Some("branch-20".into());
        s.prs[0].extra.closes = vec![5];
        assert_eq!(current_pr(&s).map(|p| p.number), Some(20));
        assert_eq!(active_issue(&s), Some(5), "falls back to the branch PR's closing issue");
        s.repo.branch = Some("issue-10-x".into());
        assert_eq!(active_issue(&s), Some(10), "branch name wins");
    }

    #[test]
    fn now_section_and_pr_rows() {
        let mut s = snap();
        s.repo.branch = Some("branch-20".into());
        s.prs[0].extra = PrExtra { review: Some("APPROVED".into()), mergeable: None, checks: Some(Checks { state: "PENDING".into(), total: 2, failed: 0, pending: 1 }), closes: vec![5] };
        s.prs[1].state = "closed".into();
        s.prs[1].merged_at = Some("2026-09-04T06:00:00Z".into());
        s.prs[1].closed_at = Some("2026-09-04T06:00:00Z".into());
        let agents = vec![AgentInfo { pane_id: "w1:p1".into(), agent: "claude".into(), status: "working".into(), title: Some("Fixing tests".into()) }];
        let mut st = TreeState::default();
        st.toggle(&NodeId::Pr(20));
        let nodes = flatten(&s, &st, NOW, &agents);
        let t = texts(&nodes, 26);
        assert!(t.iter().all(|l| l.chars().count() == 26), "{t:?}");
        assert!(t[0].starts_with("▾ NOW") && t[0].ends_with("1 busy"), "{:?}", t[0]);
        assert_eq!(nodes[1].id, NodeId::NowIssue(5));
        assert!(t[1].starts_with(" ▶ #5"), "active issue marker: {:?}", t[1]);
        assert_eq!(nodes[2].id, NodeId::NowPr(20));
        assert!(t[2].ends_with(" A◔"), "review + pending checks: {:?}", t[2]);
        assert_eq!(nodes[3].id, NodeId::Agent("w1:p1".into()));
        assert!(t[3].starts_with(" ◐ claude") && t[3].ends_with("working"), "{:?}", t[3]);
        // The milestone tree marks the active issue too.
        let i5 = nodes.iter().position(|n| n.id == NodeId::Issue(5)).unwrap();
        assert!(t[i5].contains("▶ #5"), "{:?}", t[i5]);
        // Expanded PR shows its branch line and linked issue.
        let p20 = nodes.iter().position(|n| n.id == NodeId::Pr(20)).unwrap();
        assert_eq!(nodes[p20 + 1].id, NodeId::PrDetail(20, 0));
        assert!(t[p20 + 1].contains("branch-20 → main"), "{:?}", t[p20 + 1]);
        assert!(t[p20 + 2].contains("closes #5 first open"), "{:?}", t[p20 + 2]);
        assert_eq!(nodes[p20 + 2].url.as_deref(), Some("https://github.com/o/r/issues/5"));
        // Merged PR sits in the recently merged/closed group.
        let g = nodes.iter().position(|n| n.id == NodeId::RecentPrs).unwrap();
        assert!(t[g].contains("recently merged"), "{:?}", t[g]);
        st.toggle(&NodeId::RecentPrs);
        let nodes = flatten(&s, &st, NOW, &agents);
        assert!(nodes.iter().any(|n| n.id == NodeId::Pr(21)));
        let t = texts(&nodes, 26);
        let p21 = nodes.iter().position(|n| n.id == NodeId::Pr(21)).unwrap();
        assert!(t[p21].contains("⭳ #21"), "merged icon: {:?}", t[p21]);
    }
}
