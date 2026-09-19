//! User configuration: `config.toml` in the plugin's config directory. Every key is
//! optional and validated on its own, so one bad value costs only that setting: it falls
//! back to its default and produces a warning the pane shows in its header.

use crate::ui::tree::{Section, ViewOptions};
use crate::PLUGIN_ID;
use std::path::PathBuf;
use std::time::Duration;

pub const FILE_NAME: &str = "config.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    /// Match herdr's left sidebar.
    Sidebar,
    Columns(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Seconds between full refreshes.
    pub poll_interval: Duration,
    /// Seconds between run/check refreshes while a workflow run is queued or in progress.
    pub active_poll_interval: Duration,
    pub width: Width,
    /// Whether the auto-dock hook opens the pane by itself.
    pub auto_open: bool,
    /// How long a changed row stays highlighted.
    pub recent_window: Duration,
    /// Workflow runs fetched and shown.
    pub runs_limit: usize,
    pub view: ViewOptions,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            poll_interval: crate::poll::POLL_INTERVAL,
            active_poll_interval: crate::poll::ACTIVE_INTERVAL,
            width: Width::Sidebar,
            auto_open: true,
            recent_window: Duration::from_secs(crate::app::RECENT_WINDOW_SECS),
            runs_limit: crate::github::RUNS_LIMIT,
            view: ViewOptions::default(),
        }
    }
}

/// A configuration plus what was wrong with the file it came from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Loaded {
    pub config: Config,
    pub warnings: Vec<String>,
}

/// `HERDR_PLUGIN_CONFIG_DIR`, else what `herdr plugin config-dir` reports.
pub fn dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("HERDR_PLUGIN_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    crate::util::stdout(
        &crate::herdr::bin(),
        &["plugin", "config-dir", PLUGIN_ID],
        None,
    )
    .map(|out| PathBuf::from(out.trim()))
    .filter(|p| p.is_absolute())
}

/// Load the user's configuration. A missing file is simply the defaults.
pub fn load() -> Loaded {
    let Some(path) = dir().map(|d| d.join(FILE_NAME)) else {
        return Loaded::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Loaded::default(),
        Err(err) => Loaded {
            warnings: vec![format!("cannot read {FILE_NAME}: {err}")],
            ..Loaded::default()
        },
    }
}

pub fn parse(text: &str) -> Loaded {
    let mut loaded = Loaded::default();
    let table = match text.parse::<toml::Table>() {
        Ok(table) => table,
        Err(err) => {
            // The parser's first line says what and where; the rest is a source excerpt.
            let first = err
                .to_string()
                .lines()
                .next()
                .unwrap_or_default()
                .to_string();
            loaded.warnings.push(format!("{FILE_NAME}: {first}"));
            return loaded;
        }
    };
    let Loaded { config, warnings } = &mut loaded;
    for (key, value) in &table {
        let result = match key.as_str() {
            "poll_interval_secs" => {
                int_in(value, 5, 3600).map(|n| config.poll_interval = Duration::from_secs(n))
            }
            "active_poll_interval_secs" => {
                int_in(value, 2, 3600).map(|n| config.active_poll_interval = Duration::from_secs(n))
            }
            "width" => width(value).map(|w| config.width = w),
            "auto_open" => value
                .as_bool()
                .ok_or_else(|| "must be true or false".to_string())
                .map(|b| config.auto_open = b),
            "sections" => sections(value).map(|s| config.view.sections = s),
            "recent_window_minutes" => int_in(value, 0, 24 * 60)
                .map(|n| config.recent_window = Duration::from_secs(n * 60)),
            "recent_closed_hours" => {
                int_in(value, 0, 24 * 365).map(|n| config.view.recent_closed_secs = n * 3600)
            }
            "runs_limit" => int_in(value, 1, 100).map(|n| config.runs_limit = n as usize),
            _ => Err("unknown option".to_string()),
        };
        if let Err(why) = result {
            warnings.push(format!("{key}: {why}"));
        }
    }
    loaded
}

fn int_in(value: &toml::Value, min: u64, max: u64) -> Result<u64, String> {
    value
        .as_integer()
        .and_then(|n| u64::try_from(n).ok())
        .filter(|n| (min..=max).contains(n))
        .ok_or_else(|| format!("must be a whole number from {min} to {max}"))
}

fn width(value: &toml::Value) -> Result<Width, String> {
    match value {
        toml::Value::String(s) if s == "sidebar" => Ok(Width::Sidebar),
        toml::Value::Integer(_) => int_in(value, 16, 200).map(|n| Width::Columns(n as u32)),
        _ => Err("must be \"sidebar\" or a column count from 16 to 200".to_string()),
    }
}

/// Listed sections are shown in the listed order; the rest are hidden.
fn sections(value: &toml::Value) -> Result<Vec<Section>, String> {
    let names = value
        .as_array()
        .ok_or_else(|| "must be a list of section names".to_string())?;
    let mut out = Vec::new();
    for name in names {
        let section = name.as_str().and_then(Section::from_key).ok_or_else(|| {
            let known: Vec<&str> = Section::ALL.iter().map(|s| s.key()).collect();
            format!(
                "unknown section {}; use {}",
                name.as_str().unwrap_or("(not a string)"),
                known.join(", ")
            )
        })?;
        if !out.contains(&section) {
            out.push(section);
        }
    }
    if out.is_empty() {
        return Err("lists no sections".to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_is_the_defaults() {
        assert_eq!(parse(""), Loaded::default());
        let c = Config::default();
        assert_eq!(c.poll_interval, Duration::from_secs(10));
        assert_eq!(c.active_poll_interval, Duration::from_secs(5));
        assert_eq!(c.width, Width::Sidebar);
        assert!(c.auto_open);
        assert_eq!(c.view.sections, Section::ALL.to_vec());
        assert_eq!(c.view.recent_closed_secs, 24 * 3600);
        assert_eq!(c.recent_window, Duration::from_secs(120));
        assert_eq!(c.runs_limit, 15);
    }

    #[test]
    fn every_option_parses() {
        let l = parse(
            r#"
            poll_interval_secs = 30
            active_poll_interval_secs = 3
            width = 40
            auto_open = false
            sections = ["now", "pull_requests", "actions"]
            recent_window_minutes = 10
            recent_closed_hours = 48
            runs_limit = 5
            "#,
        );
        assert_eq!(l.warnings, Vec::<String>::new());
        let c = l.config;
        assert_eq!(c.poll_interval, Duration::from_secs(30));
        assert_eq!(c.active_poll_interval, Duration::from_secs(3));
        assert_eq!(c.width, Width::Columns(40));
        assert!(!c.auto_open);
        assert_eq!(
            c.view.sections,
            vec![Section::Now, Section::PullRequests, Section::Actions]
        );
        assert_eq!(c.recent_window, Duration::from_secs(600));
        assert_eq!(c.view.recent_closed_secs, 48 * 3600);
        assert_eq!(c.runs_limit, 5);
        assert_eq!(parse(r#"width = "sidebar""#).config.width, Width::Sidebar);
    }

    #[test]
    fn a_bad_value_warns_and_keeps_only_that_default() {
        let l = parse(
            r#"
            poll_interval_secs = 1
            width = "wide"
            auto_open = "yes"
            sections = ["now", "nope"]
            runs_limit = 5
            colour = "blue"
            "#,
        );
        assert_eq!(l.config.poll_interval, Duration::from_secs(10));
        assert_eq!(l.config.width, Width::Sidebar);
        assert!(l.config.auto_open);
        assert_eq!(l.config.view.sections, Section::ALL.to_vec());
        assert_eq!(l.config.runs_limit, 5, "valid keys still apply");
        assert_eq!(l.warnings.len(), 5);
        assert!(l.warnings.iter().any(|w| w == "colour: unknown option"));
        assert!(l
            .warnings
            .iter()
            .any(|w| w.starts_with("poll_interval_secs: must be a whole number from 5")));
    }

    #[test]
    fn unparseable_file_is_one_warning_and_the_defaults() {
        let l = parse("width = = 3");
        assert_eq!(l.config, Config::default());
        assert_eq!(l.warnings.len(), 1);
        assert!(l.warnings[0].starts_with("config.toml: "));
    }

    #[test]
    fn sections_dedupe_and_reject_empty() {
        let l = parse(r#"sections = ["actions", "actions", "now"]"#);
        assert_eq!(l.config.view.sections, vec![Section::Actions, Section::Now]);
        assert_eq!(parse("sections = []").warnings.len(), 1);
    }
}
