use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const KEYRING_SERVICE: &str = "TextGlow";
pub const KEYRING_USER: &str = "default";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Provider preset id (see `providers::PRESETS`).
    pub provider: String,
    /// OpenAI-compatible API base URL.
    pub base_url: String,
    /// Model id sent to the provider.
    pub model: String,
    /// Reasoning effort: "off" | "low" | "medium" | "high" (provider-mapped).
    pub thinking: String,
    pub temperature: f32,
    /// Empty = built-in glow-up system prompt.
    pub system_prompt: String,
    /// How many past runs (source + result) to keep. 0 disables history.
    pub history_size: usize,
    /// UI font size in points for body/button text.
    pub font_size: f32,
    /// Set once the first-run wizard is closed (finished or skipped).
    pub wizard_done: bool,
    /// Extra pass that strips AI tells from every rewrite (popup toggle).
    pub de_slop: bool,
    /// Per-run override: skip reasoning entirely (popup Fast toggle).
    pub fast_no_thinking: bool,
    /// Hotkey parts, e.g. "SUPER" + "F8" = Win+F8.
    pub hotkey_modifiers: String,
    pub hotkey_key: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            provider: "openrouter".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            model: String::new(),
            thinking: "medium".into(),
            temperature: 0.7,
            system_prompt: String::new(),
            history_size: 25,
            font_size: 15.0,
            wizard_done: false,
            de_slop: true,
            fast_no_thinking: false,
            hotkey_modifiers: "SUPER".into(),
            hotkey_key: "F8".into(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("textglow")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn config_file_exists() -> bool {
    config_path().exists()
}

pub fn load() -> Config {
    load_from(&config_path())
}

/// Write to a sibling temp file and rename over the target, so a crash
/// mid-write can never leave a truncated config/history file behind.
pub fn write_atomic(path: &Path, data: &str) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let tmp = path.with_file_name(format!("{file_name}.tmp"));
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Move an unreadable file aside (`.corrupt`) so its contents survive for
/// inspection instead of being silently destroyed by the next save.
pub fn quarantine_corrupt(path: &Path) {
    if let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) {
        let dest = dir.join(format!("{name}.corrupt"));
        let _ = std::fs::remove_file(&dest);
        if std::fs::rename(path, &dest).is_ok() {
            crate::logging::error(&format!("unreadable file moved aside: {}", path.display()));
        }
    }
}

pub fn load_from(path: &Path) -> Config {
    match std::fs::read_to_string(path) {
        Ok(s) => match toml::from_str(&s) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("textglow: invalid config at {}: {e}", path.display());
                crate::logging::error(&format!("invalid config: {e}"));
                quarantine_corrupt(path);
                Config::default()
            }
        },
        Err(_) => Config::default(),
    }
}

pub fn save(cfg: &Config) -> anyhow::Result<()> {
    save_to(&config_path(), cfg)
}

pub fn save_to(path: &Path, cfg: &Config) -> anyhow::Result<()> {
    write_atomic(path, &toml::to_string_pretty(cfg)?)
}

/// The stored API key (Windows Credential Manager via keyring), if any.
pub fn load_api_key() -> Option<String> {
    let entry = keyring::v1::Entry::new(KEYRING_SERVICE, KEYRING_USER).ok()?;
    entry.get_password().ok()
}

/// Store (or remove, when empty) the API key.
pub fn store_api_key(key: &str) -> anyhow::Result<()> {
    let entry = keyring::v1::Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
    if key.is_empty() {
        let _ = entry.delete_credential();
    } else {
        entry.set_password(key)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip() {
        let dir = std::env::temp_dir().join(format!("textglow-test-{}", std::process::id()));
        let path = dir.join("config.toml");
        let cfg = Config {
            provider: "zai".into(),
            base_url: "https://api.z.ai/api/paas/v4".into(),
            model: "glm-4.6".into(),
            thinking: "high".into(),
            temperature: 0.3,
            system_prompt: "custom".into(),
            history_size: 10,
            font_size: 15.0,
            wizard_done: true,
            de_slop: true,
            fast_no_thinking: false,
            hotkey_modifiers: "CONTROL|SHIFT".into(),
            hotkey_key: "J".into(),
        };
        save_to(&path, &cfg).unwrap();
        assert_eq!(load_from(&path), cfg);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn default_config_loads_from_empty_file() {
        let dir = std::env::temp_dir().join(format!("textglow-empty-{}", std::process::id()));
        let path = dir.join("config.toml");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "").unwrap();
        assert_eq!(load_from(&path), Config::default());
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod compat_tests {
    use super::*;

    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let dir = std::env::temp_dir().join(format!("textglow-compat-{}", std::process::id()));
        let path = dir.join("config.toml");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "provider = \"openai\"\nsome_future_setting = 42\n").unwrap();
        let cfg = load_from(&path);
        assert_eq!(cfg.provider, "openai");
        assert_eq!(cfg.model, "", "missing fields fall back to defaults");
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod corrupt_tests {
    use super::*;

    #[test]
    fn corrupt_config_is_quarantined_and_defaulted() {
        let dir = std::env::temp_dir().join(format!("textglow-corrupt-{}", std::process::id()));
        let path = dir.join("config.toml");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "this is not [ valid toml {{{").unwrap();
        let cfg = load_from(&path);
        assert_eq!(cfg, Config::default());
        let corrupt = dir.join("config.toml.corrupt");
        assert_eq!(
            std::fs::read_to_string(&corrupt).unwrap(),
            "this is not [ valid toml {{{",
            "original content must survive for inspection"
        );
        assert!(!path.exists(), "config path is free for the next save");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn atomic_save_leaves_no_temp_file() {
        let dir = std::env::temp_dir().join(format!("textglow-atomic-{}", std::process::id()));
        let path = dir.join("config.toml");
        save_to(&path, &Config::default()).unwrap();
        assert!(path.exists());
        assert!(!dir.join("config.toml.tmp").exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
