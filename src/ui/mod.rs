//! Rendering helpers shared by the pane's views.

pub mod header;
pub mod help;
pub mod tree;

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use std::time::SystemTime;

/// A row with `left` spans (occupying `left_width` columns) and a dim, right-aligned count.
pub fn right_count(
    mut left: Vec<Span<'static>>,
    left_width: usize,
    count: String,
    w: usize,
) -> Line<'static> {
    let pad = w.saturating_sub(left_width + count.chars().count());
    left.push(Span::raw(" ".repeat(pad.max(1))));
    left.push(Span::styled(count, Style::default().fg(Color::DarkGray)));
    Line::from(left)
}

pub fn age_string(t: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(t)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        _ => format!("{}h", secs / 3600),
    }
}

/// Compact duration: `45s`, `3m`, `1h20m`, `2d`.
pub fn fmt_duration(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => {
            let m = (secs % 3600) / 60;
            if m == 0 {
                format!("{}h", secs / 3600)
            } else {
                format!("{}h{m}m", secs / 3600)
            }
        }
        _ => format!("{}d", secs / 86_400),
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

/// Pad or truncate `s` to exactly `width` columns.
pub fn fit(s: &str, width: usize) -> String {
    let t = truncate(s, width);
    let n = t.chars().count();
    if n < width {
        t + &" ".repeat(width - n)
    } else {
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_with_ellipsis() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hell…");
        assert_eq!(truncate("héllo", 3), "hé…");
        assert_eq!(truncate("x", 0), "");
        assert_eq!(fit("ab", 4), "ab  ");
        assert_eq!(fit("abcdef", 4), "abc…");
    }

    #[test]
    fn formats_durations() {
        assert_eq!(fmt_duration(5), "5s");
        assert_eq!(fmt_duration(125), "2m");
        assert_eq!(fmt_duration(3600), "1h");
        assert_eq!(fmt_duration(4800), "1h20m");
        assert_eq!(fmt_duration(200_000), "2d");
    }

    #[test]
    fn wraps_words_and_splits_long_tokens() {
        assert_eq!(wrap("a bb ccc dddd", 6), vec!["a bb", "ccc", "dddd"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert!(wrap("", 5).is_empty());
    }
}
