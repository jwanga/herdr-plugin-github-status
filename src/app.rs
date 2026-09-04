//! The status TUI: owns the UI state, consumes poll messages, and renders the pane.

use crate::model::Snapshot;
use crate::poll::{self, Cmd, Msg};
use crate::ui::tree::{self, NodeId, Row, TreeState};
use crate::ui::{age_string, open_url, truncate, wrap};
use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::execute;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

pub const POLL_INTERVAL: Duration = Duration::from_secs(10);
const HEADER_ROWS: u16 = 2;
const FOOTER_ROWS: u16 = 1;

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
    pub cursor: usize,
    pub scroll: usize,
    pub show_help: bool,
    /// Body geometry from the last draw, for mouse hit-testing and scrolling.
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
            cursor: 0,
            scroll: 0,
            show_help: false,
            body: Rect::new(0, HEADER_ROWS, 26, 20),
            cmd,
        }
    }

    pub fn rows(&self) -> Vec<Row> {
        match &self.snapshot {
            Some(s) => tree::rows(s, &self.tree, self.body.width as usize),
            None => Vec::new(),
        }
    }

    fn page(&self) -> usize {
        (self.body.height as usize).max(1)
    }

    /// Keep the cursor inside the row list and the viewport around the cursor.
    pub fn clamp(&mut self, row_count: usize) {
        if row_count == 0 {
            self.cursor = 0;
            self.scroll = 0;
            return;
        }
        self.cursor = self.cursor.min(row_count - 1);
        let page = self.page();
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + page {
            self.scroll = self.cursor + 1 - page;
        }
        self.scroll = self.scroll.min(row_count.saturating_sub(page));
    }

    fn move_cursor(&mut self, delta: isize) {
        let rows = self.rows();
        let n = rows.len();
        if n == 0 {
            return;
        }
        let next = (self.cursor as isize + delta).clamp(0, n as isize - 1) as usize;
        self.cursor = next;
        self.clamp(n);
    }

    fn jump_section(&mut self, forward: bool) {
        let rows = self.rows();
        let sections: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r.id, NodeId::Section(_)))
            .map(|(i, _)| i)
            .collect();
        let target = if forward {
            sections.iter().copied().find(|&i| i > self.cursor).or_else(|| sections.first().copied())
        } else {
            sections.iter().rev().copied().find(|&i| i < self.cursor).or_else(|| sections.last().copied())
        };
        if let Some(t) = target {
            self.cursor = t;
            self.clamp(rows.len());
        }
    }

    fn toggle_current(&mut self) {
        let rows = self.rows();
        if let Some(row) = rows.get(self.cursor) {
            if row.expandable.is_some() {
                self.tree.toggle(&row.id);
                let n = self.rows().len();
                self.clamp(n);
            }
        }
    }

    fn open_current(&self) {
        if let Some(url) = self.rows().get(self.cursor).and_then(|r| r.url.clone()) {
            open_url(&url);
        }
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
                let shift = key.modifiers.contains(KeyModifiers::SHIFT);
                if self.show_help {
                    match key.code {
                        KeyCode::Char('c') if ctrl => self.should_quit = true,
                        KeyCode::Char('q') if ctrl => self.should_quit = true,
                        _ => self.show_help = false,
                    }
                    return;
                }
                match key.code {
                    KeyCode::Char('q' | 'Q') => self.should_quit = true,
                    KeyCode::Char('c' | 'C') if ctrl => self.should_quit = true,
                    KeyCode::Char('r' | 'R') => self.refresh(),
                    KeyCode::Char('?') => self.show_help = true,
                    KeyCode::Char('j') | KeyCode::Down => self.move_cursor(1),
                    KeyCode::Char('k') | KeyCode::Up => self.move_cursor(-1),
                    KeyCode::Char('d') if ctrl => self.move_cursor(self.page() as isize / 2),
                    KeyCode::Char('u') if ctrl => self.move_cursor(-(self.page() as isize / 2)),
                    KeyCode::PageDown => self.move_cursor(self.page() as isize),
                    KeyCode::PageUp => self.move_cursor(-(self.page() as isize)),
                    KeyCode::Char('g') | KeyCode::Home => self.move_cursor(isize::MIN / 2),
                    KeyCode::Char('G') | KeyCode::End => self.move_cursor(isize::MAX / 2),
                    KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Right | KeyCode::Left => self.toggle_current(),
                    KeyCode::Tab => self.jump_section(true),
                    KeyCode::BackTab => self.jump_section(false),
                    KeyCode::Char('o' | 'O') => self.open_current(),
                    KeyCode::Char('h') if !shift => self.jump_section(false),
                    KeyCode::Char('l') => self.jump_section(true),
                    _ => {}
                }
            }
            Event::Mouse(m) => match m.kind {
                MouseEventKind::ScrollDown => self.scroll_by(3),
                MouseEventKind::ScrollUp => self.scroll_by(-3),
                MouseEventKind::Down(MouseButton::Left) => {
                    if self.show_help {
                        self.show_help = false;
                        return;
                    }
                    self.click(m.column, m.row);
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn scroll_by(&mut self, delta: isize) {
        let n = self.rows().len();
        let page = self.page();
        let max = n.saturating_sub(page);
        self.scroll = (self.scroll as isize + delta).clamp(0, max as isize) as usize;
        // Keep the cursor visible.
        self.cursor = self.cursor.clamp(self.scroll, (self.scroll + page).saturating_sub(1).max(self.scroll));
        self.cursor = self.cursor.min(n.saturating_sub(1));
    }

    /// Click selects the row under the pointer; clicking the selected expandable row toggles it.
    fn click(&mut self, _x: u16, y: u16) {
        if y < self.body.y || y >= self.body.y + self.body.height {
            return;
        }
        let idx = self.scroll + (y - self.body.y) as usize;
        let rows = self.rows();
        if idx >= rows.len() {
            return;
        }
        if idx == self.cursor {
            self.toggle_current();
        } else {
            self.cursor = idx;
            self.clamp(rows.len());
        }
    }

    pub fn handle_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Snapshot(s) => {
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
        }
        let n = self.rows().len();
        self.clamp(n);
    }
}

pub fn run() -> Result<()> {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".to_string());
    let (cmd_tx, msg_rx) = poll::spawn(cwd, POLL_INTERVAL);
    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let mut app = App::new(Some(cmd_tx.clone()));
    let result = event_loop(&mut terminal, &mut app, &msg_rx);
    let _ = cmd_tx.send(Cmd::Quit);
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App, msg_rx: &Receiver<Msg>) -> Result<()> {
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
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(HEADER_ROWS),
        Constraint::Min(0),
        Constraint::Length(FOOTER_ROWS),
    ])
    .areas(f.area());
    if app.body != body {
        app.body = body;
        let n = app.rows().len();
        app.clamp(n);
    }
    draw_header(f, header, app);
    draw_body(f, body, app);
    draw_footer(f, footer, app);
}

fn badge() -> Span<'static> {
    Span::styled(" status ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD))
}

pub fn header_lines(app: &App, w: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    match &app.snapshot {
        Some(s) => {
            let name = truncate(&s.repo.full_name(), w.saturating_sub(9));
            lines.push(Line::from(vec![badge(), Span::raw(" "), Span::styled(name, Style::default().add_modifier(Modifier::BOLD))]));
            let branch = s.repo.branch.clone().unwrap_or_else(|| "detached".into());
            // Trailing markers are reserved first so they are always visible.
            let mut suffix = vec![Span::styled(age_string(s.fetched_at), Style::default().fg(Color::DarkGray))];
            if !s.authenticated {
                suffix.push(Span::styled(" no-token", Style::default().fg(Color::Yellow)));
            }
            if let Status::Error(_) = app.status {
                suffix.push(Span::styled(" !", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)));
            }
            let suffix_width: usize = suffix.iter().map(|s| s.content.chars().count()).sum();
            let mut meta = vec![
                Span::styled("⎇ ", Style::default().fg(Color::Magenta)),
                Span::raw(truncate(&branch, w.saturating_sub(suffix_width + 3))),
                Span::raw(" "),
            ];
            meta.extend(suffix);
            lines.push(Line::from(meta));
        }
        None => {
            lines.push(Line::from(vec![badge()]));
            let msg = match &app.status {
                Status::Loading => "loading…".to_string(),
                Status::NoRepo(_) => "no GitHub repository".to_string(),
                Status::Error(e) => format!("error: {e}"),
                Status::Ok => String::new(),
            };
            lines.push(Line::from(Span::styled(truncate(&msg, w), Style::default().fg(Color::DarkGray))));
        }
    }
    lines
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    f.render_widget(Paragraph::new(header_lines(app, area.width as usize)), area);
}

pub const HELP: &[(&str, &str)] = &[
    ("j/k ↑/↓", "move"),
    ("⏎ space", "expand/collapse"),
    ("tab ⇧tab", "next/prev section"),
    ("g / G", "top / bottom"),
    ("^d / ^u", "half page"),
    ("o", "open in browser"),
    ("r", "refresh now"),
    ("?", "this help"),
    ("q", "close pane"),
];

pub fn help_lines(w: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled("KEYS", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)))];
    let key_w = HELP.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(1);
    for (k, v) in HELP {
        let key = format!("{k:<key_w$}");
        lines.push(Line::from(vec![
            Span::styled(key, Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled(truncate(v, w.saturating_sub(key_w + 1)), Style::default().fg(Color::DarkGray)),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("any key to close", Style::default().fg(Color::DarkGray))));
    lines
}

pub fn body_lines(app: &App, w: usize) -> Vec<Line<'static>> {
    if app.show_help {
        return help_lines(w);
    }
    let mut lines = Vec::new();
    let Some(_) = &app.snapshot else {
        match &app.status {
            Status::NoRepo(cwd) => {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled("No GitHub remote in", Style::default().fg(Color::DarkGray))));
                for chunk in wrap(cwd, w) {
                    lines.push(Line::from(Span::styled(chunk, Style::default().fg(Color::DarkGray))));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled("Watching for one…", Style::default().fg(Color::DarkGray))));
            }
            Status::Error(e) => {
                lines.push(Line::from(""));
                for chunk in wrap(e, w) {
                    lines.push(Line::from(Span::styled(chunk, Style::default().fg(Color::Red))));
                }
            }
            _ => {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled("fetching GitHub…", Style::default().fg(Color::DarkGray))));
            }
        }
        return lines;
    };
    let rows = app.rows();
    let page = app.page();
    for (i, row) in rows.iter().enumerate().skip(app.scroll).take(page) {
        let mut line = row.line.clone();
        if i == app.cursor {
            line = line.patch_style(Style::default().add_modifier(Modifier::REVERSED));
        }
        lines.push(line);
    }
    lines
}

fn draw_body(f: &mut Frame, area: Rect, app: &App) {
    let lines = body_lines(app, area.width as usize);
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let n = app.rows().len();
    let pos = if n > 0 && app.snapshot.is_some() { format!("{}/{n}", app.cursor + 1) } else { String::new() };
    let left = vec![" q".bold(), " quit".dim(), " ?".bold(), " help".dim()];
    let left_w: usize = left.iter().map(|s| s.content.chars().count()).sum();
    let pad = (area.width as usize).saturating_sub(left_w + pos.len());
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(pos, Style::default().fg(Color::DarkGray)));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GitRef, Issue, Milestone, MilestoneRef, PullRequest};
    use crate::ui::tree::Section;
    use crate::repo::RepoRef;
    use crossterm::event::{KeyEvent, KeyEventState};
    use std::time::SystemTime;

    fn snapshot() -> Snapshot {
        let issue = |n: u64, state: &str, ms: Option<u64>| Issue {
            number: n, title: format!("Issue {n}"), state: state.into(), state_reason: None,
            milestone: ms.map(|m| MilestoneRef { number: m }), labels: vec![], assignees: vec![], updated_at: String::new(),
            closed_at: None, html_url: format!("https://x/{n}"), pull_request: None,
        };
        Snapshot {
            repo: RepoRef { owner: "o".into(), name: "r".into(), branch: Some("main".into()), root: "/x".into() },
            milestones: vec![Milestone {
                number: 1, title: "Status pane core".into(), state: "open".into(), open_issues: 5, closed_issues: 1,
                due_on: None, html_url: String::new(), updated_at: String::new(),
            }],
            issues: (2..=7).map(|n| issue(n, "open", Some(1))).chain([issue(8, "open", None)]).collect(),
            prs: vec![PullRequest { number: 11, title: "Scaffold the plugin".into(), state: "open".into(), draft: true,
                merged_at: None, head: GitRef { name: "b".into(), sha: String::new() }, base: GitRef { name: "main".into(), sha: String::new() },
                user: None, updated_at: String::new(), html_url: String::new(), body: None }],
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
        Event::Key(KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE })
    }

    fn texts(lines: &[Line<'static>]) -> Vec<String> {
        lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>()).collect()
    }

    #[test]
    fn body_shows_a_page_of_rows_and_fits_width() {
        let app = app();
        let text = texts(&body_lines(&app, 26));
        assert_eq!(text.len(), 6, "one page of rows");
        assert!(text.iter().all(|t| t.chars().count() <= 26), "{text:?}");
        assert!(text[0].starts_with("▾ MILESTONES"));
        assert!(text[1].contains("Status pane") && text[1].ends_with("1/6"));
    }

    #[test]
    fn cursor_moves_scrolls_and_wraps_sections() {
        let mut app = app();
        let n = app.rows().len();
        assert_eq!(n, 12);
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
        assert!(matches!(app.rows()[app.cursor].id, NodeId::Section(Section::Issues)));
        app.handle_event(key(KeyCode::BackTab));
        assert!(matches!(app.rows()[app.cursor].id, NodeId::Section(Section::Milestones)));
        app.handle_event(key(KeyCode::BackTab));
        assert!(matches!(app.rows()[app.cursor].id, NodeId::Section(Section::PullRequests)), "wraps around");
    }

    #[test]
    fn enter_collapses_and_cursor_stays_valid() {
        let mut app = app();
        app.handle_event(key(KeyCode::Char('j'))); // milestone row
        app.handle_event(key(KeyCode::Enter));
        let ids: Vec<NodeId> = app.rows().into_iter().map(|r| r.id).collect();
        assert_eq!(ids.len(), 6);
        assert!(!ids.iter().any(|i| matches!(i, NodeId::Issue(2))));
        app.handle_event(key(KeyCode::Char('G')));
        app.handle_event(key(KeyCode::Char('?')));
        assert!(app.show_help);
        assert!(texts(&body_lines(&app, 26)).iter().all(|t| t.chars().count() <= 26));
        app.handle_event(key(KeyCode::Char('x')));
        assert!(!app.show_help);
    }

    #[test]
    fn click_selects_then_toggles() {
        let mut app = app();
        let click = |row: u16| Event::Mouse(crossterm::event::MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: 3, row, modifiers: KeyModifiers::NONE });
        app.handle_event(click(3)); // body starts at y=2 → row index 1 (milestone)
        assert_eq!(app.cursor, 1);
        app.handle_event(click(3));
        assert_eq!(app.rows().len(), 6, "second click on the selected milestone collapses it");
        app.handle_event(click(40));
        assert_eq!(app.cursor, 1, "clicks outside the body are ignored");
    }

    #[test]
    fn header_keeps_markers_visible_at_26_columns() {
        let mut app = App::new(None);
        let mut s = snapshot();
        s.repo.branch = Some("issue-2-repo-github-fetch".into());
        s.authenticated = false;
        app.snapshot = Some(s);
        app.status = Status::Error("boom".into());
        let text = texts(&header_lines(&app, 26));
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
        app.handle_msg(Msg::Error { repo: other, message: "404".into() });
        assert!(app.snapshot.is_none());
        app.handle_msg(Msg::Snapshot(Box::new(snapshot())));
        app.handle_msg(Msg::Error { repo: snapshot().repo, message: "flaky".into() });
        assert!(app.snapshot.is_some(), "same-repo errors keep the last snapshot");
    }
}
