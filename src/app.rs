//! The status TUI: owns the UI state, consumes poll messages, and renders the pane.

use crate::model::Snapshot;
use crate::poll::{self, Cmd, Msg};
use anyhow::Result;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, SystemTime};

pub const POLL_INTERVAL: Duration = Duration::from_secs(10);

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
    cmd: Option<Sender<Cmd>>,
}

impl App {
    pub fn new(cmd: Option<Sender<Cmd>>) -> Self {
        Self { snapshot: None, status: Status::Loading, should_quit: false, cmd }
    }

    pub fn handle_event(&mut self, ev: Event) {
        if let Event::Key(key) = ev {
            if key.kind != KeyEventKind::Press {
                return;
            }
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Char('q' | 'Q') => self.should_quit = true,
                KeyCode::Char('c' | 'C') if ctrl => self.should_quit = true,
                KeyCode::Char('r' | 'R') => {
                    if let Some(cmd) = &self.cmd {
                        let _ = cmd.send(Cmd::Refresh);
                    }
                }
                _ => {}
            }
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
            Msg::Error(e) => self.status = Status::Error(e),
        }
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

pub fn draw(f: &mut Frame, app: &App) {
    let [header, body, footer] =
        Layout::vertical([Constraint::Length(2), Constraint::Min(0), Constraint::Length(1)]).areas(f.area());
    draw_header(f, header, app);
    draw_body(f, body, app);
    draw_footer(f, footer);
}

fn badge() -> Span<'static> {
    Span::styled(" status ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD))
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let w = area.width as usize;
    let mut lines = Vec::new();
    match &app.snapshot {
        Some(s) => {
            let name = truncate(&s.repo.full_name(), w.saturating_sub(9));
            lines.push(Line::from(vec![badge(), Span::raw(" "), Span::styled(name, Style::default().add_modifier(Modifier::BOLD))]));
            let branch = s.repo.branch.clone().unwrap_or_else(|| "detached".into());
            let age = age_string(s.fetched_at);
            let mut meta = vec![
                Span::styled("⎇ ", Style::default().fg(Color::Magenta)),
                Span::raw(truncate(&branch, w.saturating_sub(age.len() + 4))),
                Span::raw(" "),
                Span::styled(age, Style::default().fg(Color::DarkGray)),
            ];
            if !s.authenticated {
                meta.push(Span::styled(" no-token", Style::default().fg(Color::Yellow)));
            }
            if let Status::Error(_) = app.status {
                meta.push(Span::styled(" !", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)));
            }
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
    f.render_widget(Paragraph::new(lines), area);
}

fn section(title: &str, count: String, w: usize) -> Line<'static> {
    let title = title.to_uppercase();
    let pad = w.saturating_sub(title.len() + count.len() + 1);
    Line::from(vec![
        Span::styled(title, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" ".repeat(pad.max(1))),
        Span::styled(count, Style::default().fg(Color::DarkGray)),
    ])
}

pub fn body_lines(app: &App, w: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let Some(s) = &app.snapshot else {
        match &app.status {
            Status::NoRepo(cwd) => {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled("No GitHub remote in", Style::default().fg(Color::DarkGray))));
                lines.push(Line::from(Span::styled(truncate(cwd, w), Style::default().fg(Color::DarkGray))));
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
    let open_ms: Vec<_> = s.milestones.iter().filter(|m| m.state == "open").collect();
    lines.push(section("milestones", open_ms.len().to_string(), w));
    for m in &open_ms {
        let count = format!("{}/{}", m.closed_issues, m.total());
        let title = truncate(&m.title, w.saturating_sub(count.len() + 2));
        let pad = w.saturating_sub(title.chars().count() + count.len() + 1);
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::raw(title),
            Span::raw(" ".repeat(pad.max(1))),
            Span::styled(count, Style::default().fg(Color::DarkGray)),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(section("issues", format!("{} open", s.open_issues()), w));
    for i in s.issues.iter().filter(|i| i.is_open()) {
        let num = format!("#{}", i.number);
        lines.push(Line::from(vec![
            Span::styled(" ● ", Style::default().fg(Color::Green)),
            Span::styled(num.clone(), Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::raw(truncate(&i.title, w.saturating_sub(num.len() + 4))),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(section("pull requests", format!("{} open", s.open_prs()), w));
    for p in s.prs.iter().filter(|p| p.is_open()) {
        let num = format!("#{}", p.number);
        let icon = if p.draft { " ◌ " } else { " ⇄ " };
        lines.push(Line::from(vec![
            Span::styled(icon, Style::default().fg(if p.draft { Color::DarkGray } else { Color::Green })),
            Span::styled(num.clone(), Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::raw(truncate(&p.title, w.saturating_sub(num.len() + 4))),
        ]));
    }
    lines
}

fn draw_body(f: &mut Frame, area: Rect, app: &App) {
    let lines = body_lines(app, area.width as usize);
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_footer(f: &mut Frame, area: Rect) {
    let line = Line::from(vec![" q".bold(), " quit".dim(), "  r".bold(), " refresh".dim()]);
    f.render_widget(Paragraph::new(line), area);
}

pub fn age_string(t: SystemTime) -> String {
    let secs = SystemTime::now().duration_since(t).map(|d| d.as_secs()).unwrap_or(0);
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        _ => format!("{}h", secs / 3600),
    }
}

/// Greedy word wrap to `max` columns (chars).
pub fn wrap(s: &str, max: usize) -> Vec<String> {
    let max = max.max(1);
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > max {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        if word.chars().count() > max {
            for chunk in word.chars().collect::<Vec<_>>().chunks(max) {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                cur = chunk.iter().collect();
            }
        } else {
            cur.push_str(word);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Truncate to `max` display columns with an ellipsis (ASCII-safe approximation: chars).
pub fn truncate(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GitRef, Issue, Milestone, PullRequest};
    use crate::repo::RepoRef;

    #[test]
    fn truncates_with_ellipsis() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hell…");
        assert_eq!(truncate("héllo", 3), "hé…");
        assert_eq!(truncate("x", 0), "");
    }

    #[test]
    fn wraps_words_and_splits_long_tokens() {
        assert_eq!(wrap("a bb ccc dddd", 6), vec!["a bb", "ccc", "dddd"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert!(wrap("", 5).is_empty());
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            repo: RepoRef { owner: "o".into(), name: "r".into(), branch: Some("main".into()), root: "/x".into() },
            milestones: vec![Milestone {
                number: 1, title: "Status pane core".into(), state: "open".into(), open_issues: 5, closed_issues: 1,
                due_on: None, html_url: String::new(), updated_at: String::new(),
            }],
            issues: vec![
                Issue { number: 2, title: "Detect the workspace repository".into(), state: "open".into(), state_reason: None,
                    milestone: None, labels: vec![], assignees: vec![], updated_at: String::new(), closed_at: None,
                    html_url: String::new(), pull_request: None },
                Issue { number: 1, title: "Scaffold".into(), state: "closed".into(), state_reason: Some("completed".into()),
                    milestone: None, labels: vec![], assignees: vec![], updated_at: String::new(), closed_at: None,
                    html_url: String::new(), pull_request: None },
            ],
            prs: vec![PullRequest { number: 11, title: "Scaffold the plugin".into(), state: "open".into(), draft: true,
                merged_at: None, head: GitRef { name: "b".into(), sha: String::new() }, base: GitRef { name: "main".into(), sha: String::new() },
                user: None, updated_at: String::new(), html_url: String::new(), body: None }],
            fetched_at: SystemTime::now(),
            rate_remaining: Some(4999),
            authenticated: true,
        }
    }

    #[test]
    fn body_lines_fit_26_columns() {
        let mut app = App::new(None);
        app.snapshot = Some(snapshot());
        app.status = Status::Ok;
        let lines = body_lines(&app, 26);
        let text: Vec<String> = lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>()).collect();
        assert!(text.iter().all(|t| t.chars().count() <= 26), "{text:?}");
        assert!(text[0].starts_with("MILESTONES"));
        assert!(text[1].contains("1/6"));
        assert!(text.iter().any(|t| t.contains("#2")));
        assert!(!text.iter().any(|t| t.contains("#1 ")), "closed issue should be hidden");
        assert!(text.iter().any(|t| t.contains("◌ #11")));
    }

    #[test]
    fn no_repo_message() {
        let mut app = App::new(None);
        app.status = Status::NoRepo("/tmp/somewhere".into());
        let lines = body_lines(&app, 26);
        assert!(lines.len() >= 3);
    }
}
