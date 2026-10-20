/// Provider presets. Every preset speaks the OpenAI Chat Completions protocol,
/// so one client covers them all.
pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
    pub base_url: &'static str,
    pub default_model: &'static str,
    pub needs_key: bool,
}

pub const PRESETS: &[Preset] = &[
    Preset {
        id: "openrouter",
        label: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        default_model: "openai/gpt-4o-mini",
        needs_key: true,
    },
    Preset {
        id: "deepseek",
        label: "DeepSeek",
        base_url: "https://api.deepseek.com/v1",
        default_model: "deepseek-chat",
        needs_key: true,
    },
    Preset {
        id: "zai",
        label: "Z.ai (GLM)",
        base_url: "https://api.z.ai/api/paas/v4",
        default_model: "glm-4.6",
        needs_key: true,
    },
    Preset {
        id: "openai",
        label: "OpenAI",
        base_url: "https://api.openai.com/v1",
        default_model: "gpt-4o-mini",
        needs_key: true,
    },
    Preset {
        id: "groq",
        label: "Groq",
        base_url: "https://api.groq.com/openai/v1",
        default_model: "llama-3.3-70b-versatile",
        needs_key: true,
    },
    Preset {
        id: "opencode-go",
        label: "OpenCode Go",
        base_url: "https://opencode.ai/zen/go/v1",
        default_model: "glm-5.3-flash",
        needs_key: true,
    },
    Preset {
        id: "anthropic",
        label: "Anthropic (OpenAI-compat)",
        base_url: "https://api.anthropic.com/v1",
        default_model: "claude-sonnet-4-5",
        needs_key: true,
    },
    Preset {
        id: "gemini",
        label: "Google Gemini (OpenAI-compat)",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        default_model: "gemini-2.5-flash",
        needs_key: true,
    },
    Preset {
        id: "ollama",
        label: "Ollama (local)",
        base_url: "http://localhost:11434/v1",
        default_model: "",
        needs_key: false,
    },
    Preset {
        id: "lmstudio",
        label: "LM Studio (local)",
        base_url: "http://localhost:1234/v1",
        default_model: "",
        needs_key: false,
    },
    Preset {
        id: "custom",
        label: "Custom (OpenAI-compatible)",
        base_url: "",
        default_model: "",
        needs_key: false,
    },
];

pub fn find(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}

/// Providers that reject chat requests without a stable per-conversation
/// session id. OpenCode Go (https://opencode.ai/docs/go) requires
/// `x-opencode-session` on every chat completion and asks clients to keep the
/// value stable per conversation for routing and prompt caching.
pub fn needs_session_header(provider: &str) -> bool {
    provider == "opencode-go"
}

fn trimmed_base(base: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    // Tolerate a pasted full endpoint.
    b.strip_suffix("/chat/completions").unwrap_or(b).to_string()
}

pub fn chat_url(base: &str) -> String {
    format!("{}/chat/completions", trimmed_base(base))
}

pub fn models_url(base: &str) -> String {
    format!("{}/models", trimmed_base(base))
}

/// Extra request-body field that turns the configured thinking preference into
/// something the given provider understands. There is no universal
/// OpenAI-compatible field, so this maps per provider:
///
/// - OpenRouter: `reasoning: {effort}` / `{enabled: false}`
/// - Z.ai (GLM): `thinking: {type: enabled|disabled}` (no effort levels)
/// - OpenAI, Groq, Gemini-compat, Anthropic-compat: `reasoning_effort`
///   (omitted for "off": no portable way to force-disable there)
/// - everything else (DeepSeek, OpenCode Go, Ollama, LM Studio, custom): nothing sent
pub fn thinking_field(provider: &str, level: &str) -> Option<(&'static str, serde_json::Value)> {
    let level = level.trim().to_ascii_lowercase();
    if level.is_empty() {
        return None;
    }
    match provider {
        "openrouter" => match level.as_str() {
            "off" => Some(("reasoning", serde_json::json!({ "enabled": false }))),
            "low" | "medium" | "high" => {
                Some(("reasoning", serde_json::json!({ "effort": level })))
            }
            _ => None,
        },
        "zai" => match level.as_str() {
            "off" => Some(("thinking", serde_json::json!({ "type": "disabled" }))),
            "low" | "medium" | "high" => {
                Some(("thinking", serde_json::json!({ "type": "enabled" })))
            }
            _ => None,
        },
        "openai" | "groq" | "gemini" | "anthropic" => match level.as_str() {
            "low" | "medium" | "high" => {
                Some(("reasoning_effort", serde_json::Value::String(level)))
            }
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_join_correctly() {
        assert_eq!(
            chat_url("https://api.deepseek.com/v1"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            chat_url("https://api.deepseek.com/v1/"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            chat_url("https://openrouter.ai/api/v1/chat/completions"),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            models_url("https://openrouter.ai/api/v1/"),
            "https://openrouter.ai/api/v1/models"
        );
    }

    #[test]
    fn presets_have_unique_ids_and_valid_urls() {
        let mut ids: Vec<_> = PRESETS.iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), PRESETS.len());
        for p in PRESETS {
            if !p.base_url.is_empty() {
                assert!(p.base_url.starts_with("http"), "{} base_url", p.id);
            }
        }
    }

    #[test]
    fn opencode_go_is_the_only_session_header_provider() {
        for p in PRESETS {
            assert_eq!(needs_session_header(p.id), p.id == "opencode-go", "{}", p.id);
        }
        assert!(!needs_session_header("custom"));
        assert!(!needs_session_header(""));
    }

    #[test]
    fn thinking_fields_map_per_provider() {
        let effort = |p: &str, l: &str| thinking_field(p, l).map(|(k, v)| (k, v.to_string()));

        assert_eq!(effort("opencode-go", "medium"), None);
        assert_eq!(
            effort("openrouter", "medium"),
            Some(("reasoning", r#"{"effort":"medium"}"#.to_string()))
        );
        assert_eq!(
            effort("openrouter", "off"),
            Some(("reasoning", r#"{"enabled":false}"#.to_string()))
        );
        assert_eq!(
            effort("zai", "medium"),
            Some(("thinking", r#"{"type":"enabled"}"#.to_string()))
        );
        assert_eq!(
            effort("zai", "off"),
            Some(("thinking", r#"{"type":"disabled"}"#.to_string()))
        );
        assert_eq!(
            effort("openai", "high"),
            Some(("reasoning_effort", r#""high""#.to_string()))
        );
        assert_eq!(effort("openai", "off"), None, "no portable off");
        assert_eq!(effort("deepseek", "medium"), None);
        assert_eq!(effort("ollama", "high"), None);
        assert_eq!(effort("custom", "low"), None);
        assert_eq!(effort("openrouter", ""), None);
        assert_eq!(effort("openrouter", "bogus"), None);
    }
}

#[cfg(test)]
mod url_edge_tests {
    use super::*;

    #[test]
    fn whitespace_around_base_url_is_trimmed() {
        assert_eq!(
            chat_url("  https://api.x.com/v1/  "),
            "https://api.x.com/v1/chat/completions"
        );
    }
}
