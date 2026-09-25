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
            font_size: 14.0,
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

pub fn load() -> Config {
    load_from(&config_path())
}

pub fn load_from(path: &Path) -> Config {
    match std::fs::read_to_string(path) {
        Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
            eprintln!("textglow: ignoring invalid config at {}: {e}", path.display());
            Config::default()
        }),
        Err(_) => Config::default(),
    }
}

pub fn save(cfg: &Config) -> anyhow::Result<()> {
    save_to(&config_path(), cfg)
}

pub fn save_to(path: &Path, cfg: &Config) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, toml::to_string_pretty(cfg)?)?;
    Ok(())
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
