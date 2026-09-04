//! The two-line header: badge + repository, then branch, refresh age, and markers.

use crate::app::{App, Status};
use crate::ui::{age_string, truncate};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub const ROWS: u16 = 2;

fn badge() -> Span<'static> {
    Span::styled(
        " status ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
}

pub fn lines(app: &App, w: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    match &app.snapshot {
        Some(s) => {
            let name = truncate(&s.repo.full_name(), w.saturating_sub(9));
            lines.push(Line::from(vec![
                badge(),
                Span::raw(" "),
                Span::styled(name, Style::default().add_modifier(Modifier::BOLD)),
            ]));
            let branch = s.repo.branch.clone().unwrap_or_else(|| "detached".into());
            // Trailing markers are reserved first so they are always visible.
            let mut suffix = vec![Span::styled(
                age_string(s.fetched_at),
                Style::default().fg(Color::DarkGray),
            )];
            if !s.authenticated {
                suffix.push(Span::styled(
                    " no-token",
                    Style::default().fg(Color::Yellow),
                ));
            }
            if let Status::Error(_) = app.status {
                suffix.push(Span::styled(
                    " !",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ));
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
            lines.push(Line::from(Span::styled(
                truncate(&msg, w),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }
    lines
}
