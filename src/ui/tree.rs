//! The section tree: sections → groups → milestones → issues, flattened into nodes
//! according to which nodes are expanded, and rendered per node at a given width.

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
pub enum NodeKind {
    Section { section: Section, count: String },
    Group { label: &'static str, count: usize },
    Milestone(Milestone),
    Issue(Issue),
    Pr(PullRequest),
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

/// Flatten the snapshot into visible nodes for `state`; `now` (Unix seconds) bounds the
/// recently-closed group.
pub fn flatten(s: &Snapshot, state: &TreeState, now: u64) -> Vec<Node> {
    let mut out = Vec::new();
    let repo_url = format!("https://github.com/{}/{}", s.repo.owner, s.repo.name);

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
            push_milestone(&mut out, s, state, m);
        }
        if !closed_ms.is_empty()
            && group(&mut out, state, NodeId::ClosedMilestones, "closed", closed_ms.len(), format!("{repo_url}/milestones?state=closed"))
        {
            for m in &closed_ms {
                push_milestone(&mut out, s, state, m);
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
            out.push(issue_node(i, 1));
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
                out.push(issue_node(i, 2));
            }
        }
    }

    // ---- Pull requests
    let mut open_prs: Vec<&PullRequest> = s.prs.iter().filter(|p| p.is_open()).collect();
    open_prs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    if section(&mut out, state, Section::PullRequests, format!("{} open", open_prs.len()), format!("{repo_url}/pulls")) {
        for p in &open_prs {
            out.push(Node { id: NodeId::Pr(p.number), depth: 1, expandable: None, url: Some(p.html_url.clone()), kind: NodeKind::Pr((*p).clone()) });
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

fn issue_node(i: &Issue, depth: usize) -> Node {
    Node { id: NodeId::Issue(i.number), depth, expandable: None, url: Some(i.html_url.clone()), kind: NodeKind::Issue(i.clone()) }
}

fn push_milestone(out: &mut Vec<Node>, s: &Snapshot, state: &TreeState, m: &Milestone) {
    let id = NodeId::Milestone(m.number);
    let open = state.is_open(&id);
    out.push(Node { id, depth: 1, expandable: Some(open), url: Some(m.html_url.clone()), kind: NodeKind::Milestone(m.clone()) });
    if open {
        let mut issues: Vec<&Issue> = s.issues.iter().filter(|i| i.milestone.as_ref().is_some_and(|r| r.number == m.number)).collect();
        issues.sort_by_key(|i| (!i.is_open(), i.number));
        for i in issues {
            out.push(issue_node(i, 2));
        }
    }
}

// ---------------------------------------------------------------------------- rendering

fn prefix(depth: usize, open: bool) -> String {
    format!("{}{} ", " ".repeat(depth), if open { '▾' } else { '▸' })
}

fn issue_icon(i: &Issue) -> (&'static str, Color) {
    match (i.state.as_str(), i.state_reason.as_deref()) {
        ("open", _) => ("●", Color::Green),
        (_, Some("not_planned")) => ("⊘", Color::DarkGray),
        _ => ("✓", Color::Magenta),
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
        NodeKind::Issue(i) => {
            let (icon, color) = issue_icon(i);
            let who = match i.assignees.first() {
                Some(u) if w >= 30 => format!(" {}", initials(&u.login)),
                _ => String::new(),
            };
            let style = if i.is_open() { Style::default() } else { Style::default().fg(Color::DarkGray) };
            item_line(node.depth, icon, color, i.number, &i.title, style, who, w)
        }
        NodeKind::Pr(p) => {
            let (icon, color) = if p.draft { ("◌", Color::DarkGray) } else { ("⇄", Color::Green) };
            item_line(node.depth, icon, color, p.number, &p.title, Style::default(), String::new(), w)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GitRef, MilestoneRef, Snapshot, User};
    use crate::repo::RepoRef;

    const NOW: u64 = 1_788_507_932; // 2026-09-04T07:45:32Z

    fn ms(number: u64, title: &str, state: &str, open: u64, closed: u64) -> Milestone {
        Milestone { number, title: title.into(), state: state.into(), open_issues: open, closed_issues: closed, due_on: None, html_url: format!("https://m/{number}"), updated_at: String::new() }
    }
    fn issue(number: u64, title: &str, state: &str, milestone: Option<u64>, closed_at: Option<&str>) -> Issue {
        Issue { number, title: title.into(), state: state.into(), state_reason: (state == "closed").then(|| "completed".to_string()), milestone: milestone.map(|n| MilestoneRef { number: n }), labels: vec![], assignees: vec![], updated_at: String::new(), closed_at: closed_at.map(str::to_string), html_url: format!("https://i/{number}"), pull_request: None }
    }
    fn pr(number: u64, draft: bool) -> PullRequest {
        PullRequest { number, title: format!("PR {number}"), state: "open".into(), draft, merged_at: None, head: GitRef { name: "b".into(), sha: String::new() }, base: GitRef { name: "main".into(), sha: String::new() }, user: None, updated_at: String::new(), html_url: format!("https://p/{number}"), body: None }
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
        let nodes = flatten(&s, &TreeState::default(), NOW);
        let t = texts(&nodes, 26);
        assert!(t.iter().all(|l| l.chars().count() <= 26), "{t:?}");
        let ids: Vec<&NodeId> = nodes.iter().map(|n| &n.id).collect();
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
        assert_eq!(nodes[0].url.as_deref(), Some("https://github.com/o/r/milestones"));
        assert_eq!(nodes[1].url.as_deref(), Some("https://m/1"));
        assert_eq!(nodes[2].url.as_deref(), Some("https://i/5"));
        assert_eq!(nodes[11].url.as_deref(), Some("https://p/20"));
    }

    #[test]
    fn toggling_and_recent_window() {
        let s = snap();
        let mut st = TreeState::default();
        st.toggle(&NodeId::Milestone(1));
        let ids: Vec<NodeId> = flatten(&s, &st, NOW).into_iter().map(|n| n.id).collect();
        assert!(!ids.contains(&NodeId::Issue(5)));
        st.toggle(&NodeId::ClosedMilestones);
        st.toggle(&NodeId::RecentlyClosed);
        let ids: Vec<NodeId> = flatten(&s, &st, NOW).into_iter().map(|n| n.id).collect();
        assert!(ids.contains(&NodeId::Milestone(3)));
        assert!(ids.contains(&NodeId::Issue(8)), "closed 8 h ago is recent");
        assert!(!ids.contains(&NodeId::Issue(9)), "closed 25 h ago is not");
        // 20 h later #8 also ages out and the group disappears.
        let ids: Vec<NodeId> = flatten(&s, &st, NOW + 20 * 3600).into_iter().map(|n| n.id).collect();
        assert!(!ids.contains(&NodeId::RecentlyClosed));
        st.toggle(&NodeId::Section(Section::Milestones));
        let ids: Vec<NodeId> = flatten(&s, &st, NOW).into_iter().map(|n| n.id).collect();
        assert_eq!(ids.iter().filter(|i| matches!(i, NodeId::Milestone(_))).count(), 0);
        st.set_open(&NodeId::Section(Section::Milestones), true);
        assert!(st.is_open(&NodeId::Section(Section::Milestones)));
    }

    #[test]
    fn rows_fill_width_exactly_and_show_bar_and_initials() {
        let mut s = snap();
        s.issues[2].assignees = vec![User { login: "some-one".into() }];
        let nodes = flatten(&s, &TreeState::default(), NOW);
        for w in [26usize, 32, 40] {
            let t = texts(&nodes, w);
            assert!(t.iter().all(|l| l.chars().count() == w), "w={w}: {t:?}");
        }
        let t = texts(&nodes, 40);
        assert!(t[1].contains("▓▓▓░░░"), "{:?}", t[1]);
        assert!(t[2].ends_with(" SO"), "{:?}", t[2]);
        let t = texts(&nodes, 26);
        assert!(!t[1].contains('▓') && !t[2].contains("SO"), "{:?} {:?}", t[1], t[2]);
    }
}
