//! The `?` key overlay.

use crate::ui::{truncate, wrap};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub const KEYS: &[(&str, &str)] = &[
    ("j/k ↑/↓", "move"),
    ("⏎ space", "expand/collapse"),
    ("← / →", "collapse / expand"),
    ("tab ⇧tab", "next/prev section"),
    ("g/G ⇱/⇲", "top / bottom"),
    ("^d / ^u", "half page"),
    ("pg↑/pg↓", "page"),
    ("o", "open in browser"),
    ("r", "refresh now"),
    ("?", "this help"),
    ("q", "close pane"),
];

/// `warnings`: problems in the user's `config.toml`, listed in full (the header only has
/// room for the first).
pub fn lines(w: usize, warnings: &[String]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if !warnings.is_empty() {
        lines.push(Line::from(Span::styled(
            "CONFIG",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
        for warning in warnings {
            for row in wrap(warning, w) {
                lines.push(Line::from(Span::styled(
                    row,
                    Style::default().fg(Color::Yellow),
                )));
            }
        }
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        "KEYS",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    let key_w = KEYS
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(1);
    for (k, v) in KEYS {
        let pad = key_w - k.chars().count();
        lines.push(Line::from(vec![
            Span::styled(
                format!("{k}{}", " ".repeat(pad)),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                truncate(v, w.saturating_sub(key_w + 1)),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "any key to close",
        Style::default().fg(Color::DarkGray),
    )));
    lines
}
