//! Minimal file logging. Release builds have no console (`windows_subsystem`),
//! so diagnostics go to `%APPDATA%\textglow\textglow.log`. The log is rotated
//! (kept as `textglow.prev.log`) when it passes ~1 MB. Never log API keys or
//! request bodies: messages are plain descriptions only.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

pub fn path() -> PathBuf {
    crate::config::config_dir().join("textglow.log")
}

/// Rotate an oversized log before the first write of this session.
pub fn init() {
    let path = path();
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 1_000_000 {
            let prev = path.with_file_name("textglow.prev.log");
            let _ = std::fs::remove_file(&prev);
            let _ = std::fs::rename(&path, prev);
        }
    }
}

fn write(level: &str, msg: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path()) {
        let _ = writeln!(file, "[{} {:5}] {}", now_ts(), level, msg);
    }
}

pub fn info(msg: impl AsRef<str>) {
    write("INFO", msg.as_ref());
}

pub fn error(msg: impl AsRef<str>) {
    write("ERROR", msg.as_ref());
}

/// Unix timestamp in seconds (timezone-free; logs and history only).
pub fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_ts_is_plausible() {
        // Any time after 2026-01-01 and before 2100.
        let ts = now_ts();
        assert!(ts > 1_767_225_600 && ts < 4_102_444_800);
    }
}
