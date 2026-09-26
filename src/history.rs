use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One past glow-up run: the source text and what the model produced.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryEntry {
    /// Unix timestamp (seconds).
    pub ts: u64,
    /// Tone label, e.g. "Glow up".
    pub tone: String,
    /// Free-form instruction that accompanied the run, if any.
    pub instruction: String,
    pub source: String,
    pub result: String,
}

pub fn path() -> PathBuf {
    crate::config::config_dir().join("history.json")
}

pub fn load() -> Vec<HistoryEntry> {
    load_from(&path())
}

pub fn load_from(path: &Path) -> Vec<HistoryEntry> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            eprintln!("textglow: ignoring invalid history at {}: {e}", path.display());
            Vec::new()
        }),
        Err(_) => Vec::new(),
    }
}

pub fn save(entries: &[HistoryEntry]) -> anyhow::Result<()> {
    save_to(&path(), entries)
}

pub fn save_to(path: &Path, entries: &[HistoryEntry]) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(entries)?)?;
    Ok(())
}

/// Insert newest-first and cap the list. `max == 0` disables history.
pub fn push(entries: &mut Vec<HistoryEntry>, entry: HistoryEntry, max: usize) {
    if max == 0 {
        entries.clear();
        return;
    }
    entries.insert(0, entry);
    entries.truncate(max);
}

/// Timezone-free relative age, e.g. "just now", "5m ago", "3h ago", "2d ago".
pub fn format_relative(ts: u64, now: u64) -> String {
    let delta = now.saturating_sub(ts);
    let mins = delta / 60;
    let hours = delta / 3600;
    let days = delta / 86400;
    match () {
        _ if delta < 60 => "just now".into(),
        _ if hours < 1 => format!("{mins}m ago"),
        _ if days < 1 => format!("{hours}h ago"),
        _ if days < 30 => format!("{days}d ago"),
        _ => format!("{}d ago", days),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(ts: u64) -> HistoryEntry {
        HistoryEntry {
            ts,
            tone: "Glow up".into(),
            instruction: String::new(),
            source: "src".into(),
            result: "res".into(),
        }
    }

    #[test]
    fn push_caps_and_disables() {
        let mut v = Vec::new();
        for i in 0..5u64 {
            push(&mut v, entry(i), 3);
        }
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].ts, 4, "newest first");
        push(&mut v, entry(9), 0);
        assert!(v.is_empty(), "0 disables history");
    }

    #[test]
    fn history_roundtrip() {
        let dir = std::env::temp_dir().join(format!("textglow-hist-{}", std::process::id()));
        let path = dir.join("history.json");
        let entries = vec![entry(1), entry(2)];
        save_to(&path, &entries).unwrap();
        assert_eq!(load_from(&path), entries);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn relative_formatting() {
        let now = 1_000_000u64;
        assert_eq!(format_relative(now - 10, now), "just now");
        assert_eq!(format_relative(now - 300, now), "5m ago");
        assert_eq!(format_relative(now - 7200, now), "2h ago");
        assert_eq!(format_relative(now - 3 * 86400, now), "3d ago");
        assert_eq!(format_relative(now + 500, now), "just now", "clock skew");
    }
}

#[cfg(test)]
mod edge_tests {
    use super::*;

    fn entry(ts: u64) -> HistoryEntry {
        HistoryEntry {
            ts,
            tone: "Glow up".into(),
            instruction: String::new(),
            source: "src".into(),
            result: "res".into(),
        }
    }

    #[test]
    fn push_max_one_keeps_only_newest() {
        let mut v = Vec::new();
        push(&mut v, entry(1), 1);
        push(&mut v, entry(2), 1);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].ts, 2);
    }
}
