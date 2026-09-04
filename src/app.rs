//! The status TUI. This scaffold renders the pane frame (header, body, footer) and
//! handles quit/resize; data sections arrive in later issues.

use anyhow::Result;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use std::time::Duration;

pub struct App {
    pub cwd: String,
    pub should_quit: bool,
}

impl App {
    fn new() -> Self {
        let cwd = std::env::current_dir()
            .ok()
            .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()))
            .unwrap_or_default();
        Self { cwd, should_quit: false }
    }

    fn handle_event(&mut self, ev: Event) {
        if let Event::Key(key) = ev {
            if key.kind != KeyEventKind::Press {
                return;
            }
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Char('q' | 'Q') => self.should_quit = true,
                KeyCode::Char('c' | 'C') if ctrl => self.should_quit = true,
                _ => {}
            }
        }
    }
}

pub fn run() -> Result<()> {
    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let mut app = App::new();
    let result = (|| -> Result<()> {
        while !app.should_quit {
            terminal.draw(|f| draw(f, &app))?;
            if event::poll(Duration::from_millis(250))? {
                app.handle_event(event::read()?);
            }
        }
        Ok(())
    })();
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

pub fn draw(f: &mut Frame, app: &App) {
    let [header, body, footer] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)]).areas(f.area());
    draw_header(f, header, app);
    draw_body(f, body, app);
    draw_footer(f, footer);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let title = Span::styled(" status ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD));
    let name = truncate(&app.cwd, area.width.saturating_sub(9) as usize);
    let line = Line::from(vec![title, Span::raw(" "), Span::styled(name, Style::default().add_modifier(Modifier::BOLD))]);
    f.render_widget(Paragraph::new(line), area);
}

fn draw_body(f: &mut Frame, area: Rect, _app: &App) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled("waiting for data…", Style::default().fg(Color::DarkGray))),
    ];
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_footer(f: &mut Frame, area: Rect) {
    let line = Line::from(vec![" q".bold(), " quit".dim(), "  ^q".bold(), " close".dim()]);
    f.render_widget(Paragraph::new(line), area);
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
    use super::truncate;

    #[test]
    fn truncates_with_ellipsis() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hell…");
        assert_eq!(truncate("héllo", 3), "hé…");
        assert_eq!(truncate("x", 0), "");
    }
}
