//! ETag cache for conditional requests: a 304 lets us reuse the cached body, and GitHub
//! does not count 304s against the rate limit. Persisted in the plugin state dir so a
//! restarted pane starts warm.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Upper bound on the persisted cache file; larger caches stay in memory only.
pub const MAX_PERSIST_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub etag: String,
    pub body: String,
}

#[derive(Debug, Default)]
pub struct EtagCache {
    entries: HashMap<String, Entry>,
    path: Option<PathBuf>,
    dirty: bool,
    /// URLs used since `begin()`; `sweep()` evicts the rest.
    touched: std::collections::HashSet<String>,
}

impl EtagCache {
    /// In-memory cache, optionally backed by `dir/etag-cache.json`.
    pub fn open(dir: Option<&Path>) -> Self {
        let path = dir.map(|d| d.join("etag-cache.json"));
        let entries = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            entries,
            path,
            dirty: false,
            touched: Default::default(),
        }
    }

    pub fn etag(&mut self, url: &str) -> Option<&str> {
        self.touched.insert(url.to_string());
        self.entries.get(url).map(|e| e.etag.as_str())
    }

    /// Forget one entry (a cached body that no longer decodes).
    pub fn remove(&mut self, url: &str) {
        if self.entries.remove(url).is_some() {
            self.dirty = true;
        }
    }

    /// Start a fetch cycle: entries not looked up before the next `sweep()` are stale.
    pub fn begin(&mut self) {
        self.touched.clear();
    }

    /// Drop entries not used during this cycle (old check-run SHAs, previous repos).
    pub fn sweep(&mut self) {
        let before = self.entries.len();
        let touched = std::mem::take(&mut self.touched);
        self.entries.retain(|url, _| touched.contains(url));
        if self.entries.len() != before {
            self.dirty = true;
        }
    }

    pub fn body(&self, url: &str) -> Option<&str> {
        self.entries.get(url).map(|e| e.body.as_str())
    }

    pub fn store(&mut self, url: &str, etag: String, body: String) {
        let changed = self.entries.get(url).is_none_or(|e| e.etag != etag);
        if changed {
            self.entries.insert(url.to_string(), Entry { etag, body });
            self.dirty = true;
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Write the cache if it changed and is small enough; errors are ignored (the cache
    /// is an optimization, never a requirement).
    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(path) = &self.path else { return };
        if let Ok(json) = serde_json::to_string(&self.entries) {
            if json.len() <= MAX_PERSIST_BYTES {
                let tmp = path.with_extension("json.tmp");
                if std::fs::write(&tmp, json)
                    .and_then(|_| std::fs::rename(&tmp, path))
                    .is_ok()
                {
                    self.dirty = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_reuses_and_persists() {
        let dir = std::env::temp_dir().join(format!("hgs-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = EtagCache::open(Some(&dir));
        assert!(c.is_empty());
        c.store("u1", "e1".into(), "b1".into());
        assert_eq!(c.etag("u1"), Some("e1"));
        assert_eq!(c.body("u1"), Some("b1"));
        c.save();
        let again = EtagCache::open(Some(&dir));
        assert_eq!(again.len(), 1);
        assert_eq!(again.body("u1"), Some("b1"));
        // Same etag again is not a change; a new etag is.
        let mut again = again;
        again.store("u1", "e1".into(), "ignored".into());
        assert_eq!(again.body("u1"), Some("b1"));
        again.store("u1", "e2".into(), "b2".into());
        assert_eq!(again.body("u1"), Some("b2"));
        // A cycle that only touches u2 evicts u1.
        again.store("u2", "e".into(), "b".into());
        again.begin();
        let _ = again.etag("u2");
        again.sweep();
        assert_eq!(again.len(), 1);
        assert!(again.body("u1").is_none());
        again.remove("u2");
        assert!(again.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn works_without_a_directory() {
        let mut c = EtagCache::open(None);
        c.store("u", "e".into(), "b".into());
        c.save();
        assert_eq!(c.body("u"), Some("b"));
    }
}
