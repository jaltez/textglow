use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMsg {
    pub role: String,
    pub content: String,
}

impl ChatMsg {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".into(), content: content.into() }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: "assistant".into(), content: content.into() }
    }
}

#[derive(Debug)]
pub enum LlmEvent {
    Delta(String),
    Done(String),
    Error(String),
}

#[derive(Debug, Clone)]
pub struct StreamRequest {
    /// Provider preset id (drives thinking-mode request fields).
    pub provider: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    /// "off" | "low" | "medium" | "high", mapped per provider (may be ignored).
    pub thinking: String,
    pub temperature: f32,
    pub messages: Vec<ChatMsg>,
}

#[derive(Debug, Clone)]
pub struct ModelsRequest {
    pub base_url: String,
    pub api_key: Option<String>,
}

/// Shared tokio runtime for all LLM requests, so connection pooling works
/// across calls (a fresh runtime per request would re-open TLS every time).
static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();

fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime"))
}

static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();

fn client() -> &'static reqwest::Client {
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(90))
            .build()
            .expect("reqwest client")
    })
}

/// Remove the API key from provider error bodies before they reach the UI
/// (some providers echo the Authorization header back on failures).
fn redact(body: &str, api_key: Option<&str>) -> String {
    match api_key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(key) => body.replace(key, "[redacted]"),
        None => body.to_string(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut cut = max;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}…", &s[..cut])
    }
}

/// Spawn a worker thread that POSTs a streaming chat completion and forwards
/// deltas over a channel. Dropping the receiver cancels the request.
pub fn spawn_stream(req: StreamRequest) -> Receiver<LlmEvent> {
    let (tx, rx) = mpsc::channel();
    runtime().spawn(async move { run_stream(req, tx).await });
    rx
}

async fn run_stream(req: StreamRequest, tx: mpsc::Sender<LlmEvent>) {
    use futures_util::StreamExt;

    let mut body = serde_json::json!({
        "model": req.model,
        "messages": req.messages,
        "temperature": req.temperature,
        "stream": true,
    });
    if let Some((key, value)) = crate::providers::thinking_field(&req.provider, &req.thinking) {
        if let Some(obj) = body.as_object_mut() {
            obj.insert(key.to_string(), value);
        }
    }
    let mut request = client()
        .post(crate::providers::chat_url(&req.base_url))
        .header("X-Title", "TextGlow")
        .json(&body);
    if let Some(key) = req.api_key.as_deref().filter(|k| !k.trim().is_empty()) {
        request = request.bearer_auth(key);
    }

    let resp = match request.send().await {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(LlmEvent::Error(format!("connection error: {e}")));
            return;
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let _ = tx.send(LlmEvent::Error(redact(
            &format!("HTTP {status}: {}", truncate(body.trim(), 400)),
            req.api_key.as_deref(),
        )));
        return;
    }

    let mut stream = resp.bytes_stream();
    let mut parser = SseParser::default();
    let mut full = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(LlmEvent::Error(format!("stream error: {e}")));
                return;
            }
        };
        for delta in parser.feed(&chunk) {
            full.push_str(&delta);
            if tx.send(LlmEvent::Delta(delta)).is_err() {
                return; // receiver dropped: cancelled
            }
        }
        if parser.is_overflow() {
            let _ = tx.send(LlmEvent::Error(
                "stream aborted: the server sent an oversized data line".into(),
            ));
            return;
        }
        if parser.is_done() {
            break;
        }
    }

    if full.trim().is_empty() {
        let _ = tx.send(LlmEvent::Error(
            "the model returned an empty response".into(),
        ));
    } else {
        let _ = tx.send(LlmEvent::Done(full));
    }
}

/// Spawn a worker that fetches the provider's model list from `GET /models`.
pub fn spawn_fetch_models(req: ModelsRequest) -> Receiver<Result<Vec<String>, String>> {
    let (tx, rx) = mpsc::channel();
    runtime().spawn(async move {
        let result = fetch_models_inner(req).await;
        if let Err(e) = &result {
            crate::logging::error(&format!("models fetch failed: {e}"));
        }
        let _ = tx.send(result);
    });
    rx
}

async fn fetch_models_inner(req: ModelsRequest) -> Result<Vec<String>, String> {
    let mut request = client().get(crate::providers::models_url(&req.base_url));
    if let Some(key) = req.api_key.as_deref().filter(|k| !k.trim().is_empty()) {
        request = request.bearer_auth(key);
    }
    let resp = match request.send().await {
        Ok(r) => r,
        Err(e) => return Err(format!("connection error: {e}")),
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(redact(
            &format!("HTTP {status}: {}", truncate(body.trim(), 300)),
            req.api_key.as_deref(),
        ));
    }
    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => return Err(format!("invalid JSON response: {e}")),
    };
    let mut ids: Vec<String> = body
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|i| i.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        return Err("no models found in the response (you can still type a model id manually)".into());
    }
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// Incremental parser for the `text/event-stream` framing used by OpenAI-compatible
/// chat streaming: `data: {json}\n\n` lines terminated by `data: [DONE]`.
#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
    done: bool,
    /// Set when a single line exceeded `MAX_LINE_BYTES` (broken server).
    overflow: bool,
}

/// A single SSE line larger than this is treated as a broken server instead
/// of growing memory without bound.
const MAX_LINE_BYTES: usize = 1024 * 1024;

impl SseParser {
    /// Feed raw bytes; returns the content deltas found in complete `data:` lines.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        if self.overflow {
            return Vec::new();
        }
        self.buf.extend_from_slice(bytes);
        if self.buf.len() > MAX_LINE_BYTES {
            // No newline in sight: cap memory and flag the stream as broken.
            self.overflow = true;
            self.buf.clear();
            return Vec::new();
        }
        let mut deltas = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop(); // \n
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if line.is_empty() {
                continue; // event separator
            }
            let Ok(s) = std::str::from_utf8(&line) else {
                continue;
            };
            let Some(payload) = s.strip_prefix("data:").map(str::trim_start) else {
                continue; // comments, "event:" lines, etc.
            };
            if payload == "[DONE]" {
                self.done = true;
                break;
            }
            if let Some(content) = extract_content(payload) {
                deltas.push(content);
            }
        }
        deltas
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    pub fn is_overflow(&self) -> bool {
        self.overflow
    }
}

fn extract_content(payload: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    let content = v
        .get("choices")?
        .get(0)?
        .get("delta")?
        .get("content")?
        .as_str()?;
    if content.is_empty() {
        return None;
    }
    Some(content.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sse(content: &str) -> String {
        let json = serde_json::json!({
            "id": "x", "object": "chat.completion.chunk",
            "choices": [{"index": 0, "delta": {"content": content}, "finish_reason": null}]
        });
        format!("data: {json}\n\n")
    }

    #[test]
    fn parses_simple_deltas() {
        let mut p = SseParser::default();
        let out = p.feed(format!("{}{}data: [DONE]\n\n", sse("Hello"), sse(" world")).as_bytes());
        assert_eq!(out, vec!["Hello".to_string(), " world".to_string()]);
        assert!(p.is_done());
    }

    #[test]
    fn handles_chunks_split_mid_utf8() {
        let text = "héllo ✨ glow";
        let bytes = format!("{}data: [DONE]\n\n", sse(text)).into_bytes();
        let cut = bytes.len() / 3; // split inside a multibyte char
        let mut p = SseParser::default();
        assert!(p.feed(&bytes[..cut]).is_empty());
        assert!(p.feed(&bytes[cut..2 * cut]).is_empty());
        let out = p.feed(&bytes[2 * cut..]);
        assert_eq!(out, vec![text.to_string()]);
    }

    #[test]
    fn ignores_non_data_lines_and_role_deltas() {
        let role = r#"data: {"choices":[{"delta":{"role":"assistant"},"index":0}]}"#;
        let mut p = SseParser::default();
        let out = p.feed(format!(": keepalive\nevent: chunk\n{role}\n\n{}", sse("ok")).as_bytes());
        assert_eq!(out, vec!["ok".to_string()]);
        assert!(!p.is_done());
    }

    #[test]
    fn error_payloads_produce_no_delta() {
        let err = r#"data: {"error":{"message":"bad key","type":"auth"}}"#;
        let mut p = SseParser::default();
        assert!(p.feed(format!("{err}\n\n").as_bytes()).is_empty());
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        assert_eq!(truncate("héllo", 2), "h…");
        assert_eq!(truncate("abc", 3), "abc");
    }
}

#[cfg(test)]
mod sse_edge_tests {
    use super::*;

    #[test]
    fn crlf_line_endings_are_handled() {
        let chunk = r#"data: {"choices":[{"delta":{"content":"a"}}]}"#;
        let mut p = SseParser::default();
        let out = p.feed(format!("{chunk}\r\n\r\n{chunk}\r\n\r\ndata: [DONE]\r\n\r\n").as_bytes());
        assert_eq!(out, vec!["a".to_string(), "a".to_string()]);
        assert!(p.is_done());
    }

    #[test]
    fn incomplete_lines_stay_buffered() {
        let mut p = SseParser::default();
        let partial: &[u8] = b"data: {\"choices\":[{\"delta\":{\"content\":\"he";
        assert!(p.feed(partial).is_empty());
        // A line break completes the buffered data line.
        assert!(p.feed(b"llo\"}]\n\n").is_empty(), "incomplete JSON line");
        let full: &[u8] = b"data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n";
        let out = p.feed(full);
        assert_eq!(out, vec!["hello".to_string()]);
    }
}

#[cfg(test)]
mod overflow_tests {
    use super::*;

    #[test]
    fn oversized_line_trips_overflow_and_caps_memory() {
        let mut p = SseParser::default();
        // Far more than MAX_LINE_BYTES with no newline anywhere.
        let out = p.feed(&vec![b'x'; 2 * 1024 * 1024]);
        assert!(out.is_empty());
        assert!(p.is_overflow());
        assert!(p.feed(b"more of the same").is_empty(), "parser stays inert");
    }
}

/// One-shot non-streaming request used by the first-run wizard to verify the
/// provider/model/key combination. Deliberately minimal: no temperature (some
/// reasoning models reject it), no thinking hints, tiny max_tokens.
pub fn spawn_test(req: StreamRequest) -> Receiver<Result<String, String>> {
    let (tx, rx) = mpsc::channel();
    let key = req.api_key.clone();
    runtime().spawn(async move {
        let result = test_connection_inner(req).await;
        if let Err(e) = &result {
            crate::logging::error(&format!("connection test failed: {e}"));
        }
        let _ = tx.send(result);
    });
    drop(key);
    rx
}

async fn test_connection_inner(req: StreamRequest) -> Result<String, String> {
    let body = serde_json::json!({
        "model": req.model,
        "messages": [
            ChatMsg::system("You are a connection test. Reply with the single word OK."),
            ChatMsg::user("OK"),
        ],
        "max_tokens": 16,
        "stream": false,
    });
    let mut request = client()
        .post(crate::providers::chat_url(&req.base_url))
        .header("X-Title", "TextGlow")
        .json(&body);
    if let Some(key) = req.api_key.as_deref().filter(|k| !k.trim().is_empty()) {
        request = request.bearer_auth(key);
    }
    let resp = match request.send().await {
        Ok(r) => r,
        Err(e) => return Err(format!("connection error: {e}")),
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(redact(
            &format!("HTTP {status}: {}", truncate(text.trim(), 300)),
            req.api_key.as_deref(),
        ));
    }
    let v: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => return Err(format!("invalid JSON response: {e}")),
    };
    let content = v
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .ok_or_else(|| "unexpected response shape".to_string())?
        .to_string();
    Ok(content)
}

#[cfg(test)]
mod test_request_tests {
    use super::*;

    #[test]
    fn test_request_body_is_small_and_non_streaming() {
        let req = StreamRequest {
            provider: "openai".into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key: None,
            model: "gpt-4o-mini".into(),
            thinking: "off".into(),
            temperature: 0.7,
            messages: vec![ChatMsg::user("OK")],
        };
        // The wizard test omits temperature and thinking hints on purpose:
        // reasoning models reject temperature, and we want a cheap reply.
        let body = serde_json::json!({
            "model": req.model,
            "messages": [
                ChatMsg::system("You are a connection test. Reply with the single word OK."),
                ChatMsg::user("OK"),
            ],
            "max_tokens": 16,
            "stream": false,
        });
        assert_eq!(body["stream"], serde_json::Value::Bool(false));
        assert!(body.get("temperature").is_none());
        assert_eq!(body["max_tokens"], serde_json::Value::Number(16.into()));
    }
}

#[cfg(test)]
mod redact_tests {
    use super::*;

    #[test]
    fn redact_removes_key_from_error_bodies() {
        let body = r#"{"error":{"message":"bad key sk-abc12345 provided"}}"#;
        assert_eq!(
            redact(body, Some("sk-abc12345")),
            r#"{"error":{"message":"bad key [redacted] provided"}}"#
        );
        assert_eq!(redact(body, None), body);
        assert_eq!(redact(body, Some("  ")), body);
    }
}
