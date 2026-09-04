//! The status TUI: owns the UI state, consumes poll messages, handles input, and composes
//! the header / body / footer views from `ui`.

use crate::activity::{self, Target};
use crate::model::{AgentInfo, Snapshot};
use crate::poll::{self, Cmd, Msg};
use crate::ui::tree::{self, Node, NodeId, TreeState};
use crate::ui::{header, help, wrap};
use crate::util::open_url;
use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEventKind,
};
use crossterm::execute;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

const FOOTER_ROWS: u16 = 1;
/// Activity events kept in memory.
pub const MAX_EVENTS: usize = 50;
/// How long a changed row stays highlighted.
pub const RECENT_WINDOW_SECS: u64 = 120;

/// Tree nodes that represent an activity target (the main tree and NOW copies).
fn target_nodes(t: &Target) -> Vec<NodeId> {
    match t {
        Target::Issue(n) => vec![NodeId::Issue(*n), NodeId::NowIssue(*n)],
        Target::Milestone(n) => vec![NodeId::Milestone(*n)],
        Target::Pr(n) => vec![NodeId::Pr(*n), NodeId::NowPr(*n)],
        Target::Run(id) => vec![NodeId::Run(*id), NodeId::NowRun(*id)],
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Loading,
    NoRepo(String),
    Error(String),
    Ok,
}

pub struct App {
    pub snapshot: Option<Snapshot>,
    pub status: Status,
    pub should_quit: bool,
    pub tree: TreeState,
    /// herdr agents in this workspace, shown in the Now section.
    pub agents: Vec<AgentInfo>,
    /// Latest detected transitions, newest first (at most `MAX_EVENTS`).
    pub events: Vec<activity::Event>,
    /// Nodes changed recently, with the time of the change, for the highlight.
    pub recent: HashMap<NodeId, u64>,
    /// Visible tree nodes, rebuilt when the snapshot or expansion state changes.
    pub nodes: Vec<Node>,
    pub cursor: usize,
    pub scroll: usize,
    pub show_help: bool,
    /// Body geometry from the last draw, for mouse hit-testing and paging.
    pub body: Rect,
    cmd: Option<Sender<Cmd>>,
}

impl App {
    pub fn new(cmd: Option<Sender<Cmd>>) -> Self {
        Self {
            snapshot: None,
            status: Status::Loading,
            should_quit: false,
            tree: TreeState::default(),
            agents: Vec::new(),
            events: Vec::new(),
            recent: HashMap::new(),
            nodes: Vec::new(),
            cursor: 0,
            scroll: 0,
            show_help: false,
            body: Rect::new(0, header::ROWS, 26, 20),
            cmd,
        }
    }

    /// Recompute the visible nodes and keep the cursor valid.
    pub fn rebuild(&mut self) {
        self.nodes = match &self.snapshot {
            Some(s) => tree::flatten(s, &self.tree, tree::now_secs(), &self.agents, &self.events),
            None => Vec::new(),
        };
        self.clamp();
    }

    fn page(&self) -> usize {
        (self.body.height as usize).max(1)
    }

    /// Keep the cursor inside the node list and the viewport around the cursor.
    fn clamp(&mut self) {
        let n = self.nodes.len();
        if n == 0 {
            self.cursor = 0;
            self.scroll = 0;
            return;
        }
        self.cursor = self.cursor.min(n - 1);
        let page = self.page();
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + page {
            self.scroll = self.cursor + 1 - page;
        }
        self.scroll = self.scroll.min(n.saturating_sub(page));
    }

    fn set_cursor(&mut self, cursor: usize) {
        self.cursor = cursor;
        self.clamp();
    }

    fn move_cursor(&mut self, delta: isize) {
        self.set_cursor(self.cursor.saturating_add_signed(delta));
    }

    fn jump_section(&mut self, forward: bool) {
        let sections: Vec<usize> = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| matches!(n.id, NodeId::Section(_)))
            .map(|(i, _)| i)
            .collect();
        let target = if forward {
            sections
                .iter()
                .copied()
                .find(|&i| i > self.cursor)
                .or_else(|| sections.first().copied())
        } else {
            sections
                .iter()
                .rev()
                .copied()
                .find(|&i| i < self.cursor)
                .or_else(|| sections.last().copied())
        };
        if let Some(t) = target {
            self.set_cursor(t);
        }
    }

    /// Whether `id` changed within the highlight window.
    pub fn is_recent(&self, id: &NodeId, now: u64) -> bool {
        self.recent
            .get(id)
            .is_some_and(|t| now.saturating_sub(*t) <= RECENT_WINDOW_SECS)
    }

    pub fn current(&self) -> Option<&Node> {
        self.nodes.get(self.cursor)
    }

    /// Where `o` would go for the selected node.
    pub fn current_url(&self) -> Option<&str> {
        self.current().and_then(|n| n.url.as_deref())
    }

    /// Toggle the selected node, or force it `open` when given.
    fn set_current_open(&mut self, open: Option<bool>) {
        let Some(node) = self.current() else { return };
        if node.expandable.is_none() {
            return;
        }
        let id = node.id.clone();
        match open {
            Some(o) => self.tree.set_open(&id, o),
            None => self.tree.toggle(&id),
        }
        self.rebuild();
    }

    fn refresh(&self) {
        if let Some(cmd) = &self.cmd {
            let _ = cmd.send(Cmd::Refresh);
        }
    }

    pub fn handle_event(&mut self, ev: Event) {
        match ev {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                if self.show_help {
                    match key.code {
                        KeyCode::Char('c' | 'q') if ctrl => self.should_quit = true,
                        _ => self.show_help = false,
                    }
                    return;
                }
                let page = self.page() as isize;
                match key.code {
                    KeyCode::Char('q' | 'Q') => self.should_quit = true,
                    KeyCode::Char('c' | 'C') if ctrl => self.should_quit = true,
                    KeyCode::Char('r' | 'R') => self.refresh(),
                    KeyCode::Char('?') => self.show_help = true,
                    KeyCode::Char('j') | KeyCode::Down => self.move_cursor(1),
                    KeyCode::Char('k') | KeyCode::Up => self.move_cursor(-1),
                    KeyCode::Char('d') if ctrl => self.move_cursor(page / 2),
                    KeyCode::Char('u') if ctrl => self.move_cursor(-(page / 2)),
                    KeyCode::PageDown => self.move_cursor(page),
                    KeyCode::PageUp => self.move_cursor(-page),
                    KeyCode::Char('g') | KeyCode::Home => self.set_cursor(0),
                    KeyCode::Char('G') | KeyCode::End => self.set_cursor(usize::MAX),
                    KeyCode::Enter | KeyCode::Char(' ') => self.set_current_open(None),
                    KeyCode::Left => self.set_current_open(Some(false)),
                    KeyCode::Right => self.set_current_open(Some(true)),
                    KeyCode::Tab => self.jump_section(true),
                    KeyCode::BackTab => self.jump_section(false),
                    KeyCode::Char('o' | 'O') => {
                        if let Some(url) = self.current_url() {
                            open_url(url);
                        }
                    }
                    _ => {}
                }
            }
            Event::Mouse(m) => match m.kind {
                MouseEventKind::ScrollDown => self.scroll_by(3),
                MouseEventKind::ScrollUp => self.scroll_by(-3),
                MouseEventKind::Down(MouseButton::Left) => {
                    if self.show_help {
                        self.show_help = false;
                    } else {
                        self.click(m.row);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn scroll_by(&mut self, delta: isize) {
        let n = self.nodes.len();
        let page = self.page();
        let max = n.saturating_sub(page);
        self.scroll = self.scroll.saturating_add_signed(delta).min(max);
        // Keep the cursor inside the viewport.
        self.cursor = self
            .cursor
            .clamp(self.scroll, self.scroll + page - 1)
            .min(n.saturating_sub(1));
    }

    /// Click selects the row under the pointer; clicking the selected expandable row toggles it.
    fn click(&mut self, y: u16) {
        if y < self.body.y || y >= self.body.y + self.body.height {
            return;
        }
        let idx = self.scroll + (y - self.body.y) as usize;
        if idx >= self.nodes.len() {
            return;
        }
        if idx == self.cursor {
            self.set_current_open(None);
        } else {
            self.set_cursor(idx);
        }
    }

    pub fn handle_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Snapshot(s) => {
                let now = tree::now_secs();
                if let Some(prev) = self.snapshot.as_ref().filter(|p| p.repo == s.repo) {
                    let mut fresh = activity::diff(prev, &s, now);
                    for e in &fresh {
                        for id in target_nodes(&e.target) {
                            self.recent.insert(id, now);
                        }
                    }
                    fresh.reverse();
                    fresh.append(&mut self.events);
                    self.events = fresh;
                    self.events.truncate(MAX_EVENTS);
                } else if self.snapshot.as_ref().is_some_and(|p| p.repo != s.repo) {
                    self.events.clear();
                    self.recent.clear();
                }
                self.recent
                    .retain(|_, t| now.saturating_sub(*t) <= RECENT_WINDOW_SECS);
                self.snapshot = Some(*s);
                self.status = Status::Ok;
            }
            Msg::NoRepo(cwd) => {
                self.snapshot = None;
                self.status = Status::NoRepo(cwd);
            }
            Msg::Error { repo, message } => {
                if self.snapshot.as_ref().is_some_and(|s| s.repo != repo) {
                    self.snapshot = None;
                }
                self.status = Status::Error(message);
            }
            Msg::Agents(agents) => self.agents = agents,
        }
        self.rebuild();
    }
}

pub fn run() -> Result<()> {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".to_string());
    let (cmd_tx, msg_rx) = poll::spawn(cwd, poll::POLL_INTERVAL);
    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let mut app = App::new(Some(cmd_tx.clone()));
    let result = event_loop(&mut terminal, &mut app, &msg_rx);
    let _ = cmd_tx.send(Cmd::Quit);
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    msg_rx: &Receiver<Msg>,
) -> Result<()> {
    while !app.should_quit {
        while let Ok(msg) = msg_rx.try_recv() {
            app.handle_msg(msg);
        }
        terminal.draw(|f| draw(f, app))?;
        if event::poll(Duration::from_millis(250))? {
            app.handle_event(event::read()?);
        }
    }
    Ok(())
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let [top, body, bottom] = Layout::vertical([
        Constraint::Length(header::ROWS),
        Constraint::Min(0),
        Constraint::Length(FOOTER_ROWS),
    ])
    .areas(f.area());
    if app.body != body {
        app.body = body;
        app.clamp();
    }
    f.render_widget(Paragraph::new(header::lines(app, top.width as usize)), top);
    f.render_widget(Paragraph::new(body_lines(app, body.width as usize)), body);
    f.render_widget(
        Paragraph::new(footer_line(app, bottom.width as usize)),
        bottom,
    );
}

pub fn body_lines(app: &App, w: usize) -> Vec<Line<'static>> {
    if app.show_help {
        return help::lines(w);
    }
    let now = tree::now_secs();
    let dim = Style::default().fg(Color::DarkGray);
    if app.snapshot.is_none() {
        let mut lines = vec![Line::from("")];
        match &app.status {
            Status::NoRepo(cwd) => {
                lines.push(Line::from(Span::styled("No GitHub remote in", dim)));
                lines.extend(
                    wrap(cwd, w)
                        .into_iter()
                        .map(|c| Line::from(Span::styled(c, dim))),
                );
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled("Watching for one…", dim)));
            }
            Status::Error(e) => {
                lines.extend(
                    wrap(e, w)
                        .into_iter()
                        .map(|c| Line::from(Span::styled(c, Style::default().fg(Color::Red)))),
                );
            }
            _ => lines.push(Line::from(Span::styled("fetching GitHub…", dim))),
        }
        return lines;
    }
    app.nodes
        .iter()
        .enumerate()
        .skip(app.scroll)
        .take(app.page())
        .map(|(i, node)| {
            let mut line = tree::render(node, w);
            if app.is_recent(&node.id, now) {
                line = line.patch_style(Style::default().fg(Color::Yellow));
            }
            if i == app.cursor {
                line = line.patch_style(Style::default().add_modifier(Modifier::REVERSED));
            }
            line
        })
        .collect()
}

fn footer_line(app: &App, w: usize) -> Line<'static> {
    let n = app.nodes.len();
    let pos = if n > 0 {
        format!("{}/{n}", app.cursor + 1)
    } else {
        String::new()
    };
    let mut spans = vec![" q".bold(), " quit".dim(), " ?".bold(), " help".dim()];
    let left_w: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    spans.push(Span::raw(" ".repeat(w.saturating_sub(left_w + pos.len()))));
    spans.push(Span::styled(pos, Style::default().fg(Color::DarkGray)));
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GitRef, Issue, Milestone, MilestoneRef, PullRequest};
    use crate::repo::RepoRef;
    use crate::ui::tree::Section;
    use crossterm::event::{KeyEvent, KeyEventState, MouseEvent};
    use std::time::SystemTime;

    fn snapshot() -> Snapshot {
        let issue = |n: u64, ms: Option<u64>| Issue {
            number: n,
            title: format!("Issue {n}"),
            state: "open".into(),
            state_reason: None,
            milestone: ms.map(|m| MilestoneRef { number: m }),
            labels: vec![],
            assignees: vec![],
            updated_at: String::new(),
            closed_at: None,
            html_url: format!("https://x/{n}"),
            pull_request: None,
        };
        Snapshot {
            repo: RepoRef {
                owner: "o".into(),
                name: "r".into(),
                branch: Some("main".into()),
                root: "/x".into(),
            },
            milestones: vec![Milestone {
                number: 1,
                title: "Status pane core".into(),
                state: "open".into(),
                open_issues: 5,
                closed_issues: 1,
                due_on: None,
                html_url: "https://m/1".into(),
                updated_at: String::new(),
            }],
            issues: (2..=7)
                .map(|n| issue(n, Some(1)))
                .chain([issue(8, None)])
                .collect(),
            prs: vec![PullRequest {
                number: 11,
                title: "Scaffold the plugin".into(),
                state: "open".into(),
                draft: true,
                merged_at: None,
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
                html_url: "https://p/11".into(),
                body: None,
                extra: Default::default(),
            }],
            runs: vec![],
            checks: Default::default(),
            fetched_at: SystemTime::now(),
            rate_remaining: Some(4999),
            authenticated: true,
        }
    }

    fn app() -> App {
        let mut app = App::new(None);
        app.body = Rect::new(0, 2, 26, 6);
        app.handle_msg(Msg::Snapshot(Box::new(snapshot())));
        app
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn click(row: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 3,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn texts(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn body_shows_a_page_of_rows_and_fits_width() {
        let app = app();
        let text = texts(&body_lines(&app, 26));
        assert_eq!(text.len(), 6, "one page of rows");
        assert!(text.iter().all(|t| t.chars().count() <= 26), "{text:?}");
        assert!(text[0].starts_with("▾ NOW"));
        assert!(text[2].starts_with("▾ MILESTONES"));
        assert!(text[3].contains("Status pane") && text[3].ends_with("1/6"));
        assert!(texts(&[footer_line(&app, 26)])[0].ends_with("1/18"));
    }

    #[test]
    fn cursor_moves_scrolls_and_wraps_sections() {
        let mut app = app();
        let n = app.nodes.len();
        assert_eq!(n, 18);
        for _ in 0..8 {
            app.handle_event(key(KeyCode::Char('j')));
        }
        assert_eq!(app.cursor, 8);
        assert_eq!(app.scroll, 3, "viewport follows the cursor");
        app.handle_event(key(KeyCode::Char('G')));
        assert_eq!(app.cursor, n - 1);
        app.handle_event(key(KeyCode::Char('g')));
        assert_eq!(app.cursor, 0);
        app.handle_event(key(KeyCode::Tab));
        assert!(matches!(
            app.current().unwrap().id,
            NodeId::Section(Section::Milestones)
        ));
        app.handle_event(key(KeyCode::BackTab));
        assert!(matches!(
            app.current().unwrap().id,
            NodeId::Section(Section::Now)
        ));
        app.handle_event(key(KeyCode::BackTab));
        assert!(
            matches!(
                app.current().unwrap().id,
                NodeId::Section(Section::Activity)
            ),
            "wraps around"
        );
        app.handle_event(key(KeyCode::PageUp));
        assert_eq!(app.cursor, 10);
    }

    #[test]
    fn toggle_keys_and_help() {
        let mut app = app();
        for _ in 0..3 {
            app.handle_event(key(KeyCode::Char('j'))); // milestone row
        }
        assert_eq!(app.current_url(), Some("https://m/1"));
        app.handle_event(key(KeyCode::Enter));
        assert_eq!(app.nodes.len(), 12);
        assert!(!app.nodes.iter().any(|n| matches!(n.id, NodeId::Issue(2))));
        app.handle_event(key(KeyCode::Left));
        assert_eq!(
            app.nodes.len(),
            12,
            "left on a collapsed node stays collapsed"
        );
        app.handle_event(key(KeyCode::Right));
        assert_eq!(app.nodes.len(), 18, "right expands");
        app.handle_event(key(KeyCode::Char('G')));
        assert_eq!(
            app.current_url(),
            None,
            "the no-runs placeholder has no link"
        );
        for _ in 0..4 {
            app.handle_event(key(KeyCode::Char('k')));
        }
        assert_eq!(app.current_url(), Some("https://p/11"));
        app.handle_event(key(KeyCode::Char('?')));
        assert!(app.show_help);
        let help = texts(&body_lines(&app, 26));
        assert!(
            help.iter()
                .all(|t| t.chars().count() <= 26 && !t.contains('…')),
            "{help:?}"
        );
        app.handle_event(key(KeyCode::Char('x')));
        assert!(!app.show_help);
    }

    #[test]
    fn click_selects_then_toggles() {
        let mut app = app();
        app.handle_event(click(5)); // body starts at y=2 → row index 3 (milestone)
        assert_eq!(app.cursor, 3);
        app.handle_event(click(5));
        assert_eq!(
            app.nodes.len(),
            12,
            "second click on the selected milestone collapses it"
        );
        app.handle_event(click(40));
        assert_eq!(app.cursor, 3, "clicks outside the body are ignored");
    }

    #[test]
    fn header_keeps_markers_visible_at_26_columns() {
        let mut app = App::new(None);
        let mut s = snapshot();
        s.repo.branch = Some("issue-2-repo-github-fetch".into());
        s.authenticated = false;
        app.snapshot = Some(s);
        app.status = Status::Error("boom".into());
        let text = texts(&header::lines(&app, 26));
        assert_eq!(text.len(), 2);
        assert!(text.iter().all(|t| t.chars().count() <= 26), "{text:?}");
        assert!(text[1].ends_with("no-token !"), "{:?}", text[1]);
    }

    #[test]
    fn no_repo_message() {
        let mut app = App::new(None);
        app.status = Status::NoRepo("/some/very/long/path/that/does/not/fit/in/the/pane".into());
        let text = texts(&body_lines(&app, 26));
        assert!(text.iter().any(|t| t.contains("No GitHub remote in")));
        assert!(text.iter().any(|t| t.contains("Watching for one")));
        assert!(text.iter().all(|t| t.chars().count() <= 26), "{text:?}");
    }

    #[test]
    fn error_for_another_repo_clears_stale_snapshot() {
        let mut app = app();
        let mut other = snapshot().repo;
        other.name = "elsewhere".into();
        app.handle_msg(Msg::Error {
            repo: other,
            message: "404".into(),
        });
        assert!(app.snapshot.is_none());
        assert!(app.nodes.is_empty());
        app.handle_msg(Msg::Snapshot(Box::new(snapshot())));
        app.handle_msg(Msg::Error {
            repo: snapshot().repo,
            message: "flaky".into(),
        });
        assert!(
            app.snapshot.is_some(),
            "same-repo errors keep the last snapshot"
        );
    }

    #[test]
    fn snapshot_diff_feeds_activity_and_highlights() {
        let mut app = app();
        assert!(app.events.is_empty());
        let mut next = snapshot();
        next.issues[0].state = "closed".into(); // #2
        next.prs[0].draft = false; // #11 ready
        app.handle_msg(Msg::Snapshot(Box::new(next)));
        let kinds: Vec<&activity::Kind> = app.events.iter().map(|e| &e.kind).collect();
        assert_eq!(
            kinds,
            vec![&activity::Kind::PrReady, &activity::Kind::IssueClosed],
            "newest first"
        );
        let now = tree::now_secs();
        assert!(app.is_recent(&NodeId::Issue(2), now));
        assert!(app.is_recent(&NodeId::Pr(11), now));
        assert!(!app.is_recent(&NodeId::Issue(3), now));
        assert!(app
            .nodes
            .iter()
            .any(|n| matches!(n.id, NodeId::Section(Section::Activity))));
        assert!(app.nodes.iter().any(|n| matches!(n.id, NodeId::Event(_))));
        // A different repository resets the feed.
        let mut other = snapshot();
        other.repo.name = "elsewhere".into();
        app.handle_msg(Msg::Snapshot(Box::new(other)));
        assert!(app.events.is_empty());
    }

    #[test]
    fn agents_update_rebuilds_now_section() {
        let mut app = app();
        assert!(app.nodes.iter().any(|n| matches!(n.id, NodeId::Idle)));
        app.handle_msg(Msg::Agents(vec![AgentInfo {
            pane_id: "w7:p1".into(),
            agent: "claude".into(),
            status: "working".into(),
            title: None,
        }]));
        assert!(app.nodes.iter().any(|n| matches!(n.id, NodeId::Agent(_))));
        assert!(
            app.nodes.iter().any(|n| matches!(n.id, NodeId::Idle)),
            "no branch work: still nothing in progress"
        );
        let text = texts(&body_lines(&app, 26));
        assert!(text[0].ends_with("1 busy"), "{:?}", text[0]);
    }
}
