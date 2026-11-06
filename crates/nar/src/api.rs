//! OpenAI Responses API client (streaming) and model listing.
//!
//! Requests always use `store: false` and `stream: true`, resend the full
//! history in `input`, and request `reasoning.encrypted_content` so reasoning
//! can be carried across stateless turns.

use std::time::Duration;

use anyhow::Result;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use crate::auth::{Auth, USAGE_URL};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub cached_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Clone, Debug)]
pub struct ApiError {
    pub status: Option<u16>,
    pub code: Option<String>,
    pub message: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    TextDelta(String),
    ReasoningDelta(String),
    /// A reasoning summary part finished (separates paragraphs).
    ReasoningBreak,
    ToolCallStarted { name: String },
    /// A finished output item (message, reasoning, function_call…), raw JSON.
    ItemDone(Value),
    Completed(Usage),
}

#[derive(Clone)]
pub struct Client {
    pub http: reqwest::Client,
    pub auth: Auth,
    pub base: String,
}

fn user_agent() -> String {
    format!("nar/{} ({}; {})", crate::config::VERSION, std::env::consts::OS, std::env::consts::ARCH)
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(user_agent())
        .connect_timeout(Duration::from_secs(20))
        .build()
        .expect("http client")
}

/// Strip server-assigned ids so stateless (`store: false`) replays are accepted.
pub fn sanitize_input(items: &[Value]) -> Vec<Value> {
    items
        .iter()
        .map(|it| {
            let mut it = it.clone();
            if let Some(obj) = it.as_object_mut() {
                obj.remove("id");
                obj.remove("status");
                if obj.get("type").and_then(|t| t.as_str()) == Some("message") {
                    if let Some(content) = obj.get_mut("content").and_then(|c| c.as_array_mut()) {
                        for part in content {
                            if let Some(p) = part.as_object_mut() {
                                p.remove("annotations");
                                p.remove("logprobs");
                            }
                        }
                    }
                }
            }
            it
        })
        .collect()
}

fn friendly_error(status: Option<u16>, code: Option<&str>, message: &str) -> String {
    match code {
        Some("subscription_sharing_usage_limit_exceeded") => {
            format!("You've reached your ChatGPT plan's usage limit. Manage usage: {USAGE_URL}")
        }
        Some("subscription_sharing_user_not_eligible") => {
            "This ChatGPT account isn't eligible to use its plan in nar. Use an API key via /auth.".into()
        }
        Some("subscription_sharing_invalid_user") => "Your ChatGPT session is no longer valid — run /auth to sign in again.".into(),
        Some("subscription_sharing_unsupported_capability") => format!("Not supported with ChatGPT plan usage: {message}"),
        Some("subscription_sharing_usage_unavailable") | Some("subscription_sharing_user_unavailable") => {
            "ChatGPT plan usage is temporarily unavailable. Try again shortly.".into()
        }
        Some("chatpass_v2_scope_not_authorized") | Some("chatpass_v2_invalid_authorization_context") => {
            "ChatGPT plan permission is missing — run /auth to grant it.".into()
        }
        _ => match status {
            Some(401) => format!("Unauthorized — run /auth. {message}"),
            Some(403) => format!("Forbidden: {message}"),
            Some(429) => format!("Rate limited: {message}"),
            _ => message.to_string(),
        },
    }
}

fn parse_error_body(status: u16, body: &str) -> ApiError {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let err = v.get("error").cloned().unwrap_or(Value::Null);
    let code = err
        .get("code")
        .and_then(|c| c.as_str())
        .or_else(|| err.get("type").and_then(|c| c.as_str()))
        .map(String::from);
    let message = err
        .get("message")
        .and_then(|m| m.as_str())
        .or_else(|| v.get("detail").and_then(|m| m.as_str()))
        .map(String::from)
        .unwrap_or_else(|| if body.is_empty() { format!("HTTP {status}") } else { body.chars().take(400).collect() });
    ApiError { status: Some(status), message: friendly_error(Some(status), code.as_deref(), &message), code }
}

fn usage_from(v: &Value) -> Usage {
    let u = &v["usage"];
    Usage {
        input_tokens: u["input_tokens"].as_u64().unwrap_or(0),
        cached_tokens: u["input_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0),
        output_tokens: u["output_tokens"].as_u64().unwrap_or(0),
        reasoning_tokens: u["output_tokens_details"]["reasoning_tokens"].as_u64().unwrap_or(0),
        total_tokens: u["total_tokens"].as_u64().unwrap_or(0),
    }
}

impl Client {
    pub fn new(http: reqwest::Client, auth: Auth, base: String) -> Client {
        Client { http, auth, base: base.trim_end_matches('/').to_string() }
    }

    /// Stream one Responses API turn. Returns when the response completes,
    /// fails, or `cancel` fires (Ok(false) when cancelled).
    pub async fn stream(&self, body: Value, tx: &UnboundedSender<StreamEvent>, cancel: &CancellationToken) -> Result<bool, ApiError> {
        let url = format!("{}/responses", self.base);
        let mut attempt = 0u32;
        let mut refreshed = false;
        let resp = loop {
            attempt += 1;
            let token = match self.auth.bearer().await {
                Ok(t) => t,
                Err(e) => return Err(ApiError { status: None, code: None, message: e.to_string() }),
            };
            let req = self
                .http
                .post(&url)
                .bearer_auth(&token)
                .header("Accept", "text/event-stream")
                .json(&body)
                .send();
            let r = tokio::select! {
                _ = cancel.cancelled() => return Ok(false),
                r = req => r,
            };
            match r {
                Ok(r) if r.status().is_success() => break r,
                Ok(r) => {
                    let status = r.status().as_u16();
                    let text = r.text().await.unwrap_or_default();
                    let err = parse_error_body(status, &text);
                    if status == 401 && !refreshed && err.code.as_deref() != Some("subscription_sharing_invalid_user") {
                        refreshed = true;
                        if self.auth.force_refresh().await.is_ok() {
                            continue;
                        }
                    }
                    let retryable = matches!(status, 500 | 502 | 503 | 504)
                        || (status == 429 && err.code.as_deref() != Some("subscription_sharing_usage_limit_exceeded"));
                    if retryable && attempt < 4 {
                        let wait = Duration::from_millis(800 * 2u64.pow(attempt));
                        tokio::select! { _ = cancel.cancelled() => return Ok(false), _ = tokio::time::sleep(wait) => {} }
                        continue;
                    }
                    return Err(err);
                }
                Err(e) => {
                    if attempt < 4 {
                        let wait = Duration::from_millis(800 * 2u64.pow(attempt));
                        tokio::select! { _ = cancel.cancelled() => return Ok(false), _ = tokio::time::sleep(wait) => {} }
                        continue;
                    }
                    return Err(ApiError { status: None, code: None, message: format!("network error: {e}") });
                }
            }
        };

        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        let idle = Duration::from_secs(300);
        loop {
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Ok(false),
                c = tokio::time::timeout(idle, stream.next()) => c,
            };
            let chunk = match chunk {
                Err(_) => return Err(ApiError { status: None, code: None, message: "stream timed out".into() }),
                Ok(None) => {
                    return Err(ApiError {
                        status: None,
                        code: None,
                        message: "stream ended before response.completed".into(),
                    })
                }
                Ok(Some(Err(e))) => return Err(ApiError { status: None, code: None, message: format!("stream error: {e}") }),
                Ok(Some(Ok(c))) => c,
            };
            buf.extend_from_slice(&chunk);
            while let Some(pos) = find_event_end(&buf) {
                let raw: Vec<u8> = buf.drain(..pos.0 + pos.1).collect();
                let text = String::from_utf8_lossy(&raw[..pos.0]);
                let mut data = String::new();
                for line in text.lines() {
                    if let Some(d) = line.strip_prefix("data:") {
                        if !data.is_empty() {
                            data.push('\n');
                        }
                        data.push_str(d.strip_prefix(' ').unwrap_or(d));
                    }
                }
                if data.is_empty() || data == "[DONE]" {
                    continue;
                }
                let v: Value = match serde_json::from_str(&data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(done) = self.handle_event(&v, tx)? {
                    return Ok(done);
                }
            }
        }
    }

    fn handle_event(&self, v: &Value, tx: &UnboundedSender<StreamEvent>) -> Result<Option<bool>, ApiError> {
        let ty = v["type"].as_str().unwrap_or("");
        match ty {
            "response.output_text.delta" => {
                if let Some(d) = v["delta"].as_str() {
                    let _ = tx.send(StreamEvent::TextDelta(d.to_string()));
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                if let Some(d) = v["delta"].as_str() {
                    let _ = tx.send(StreamEvent::ReasoningDelta(d.to_string()));
                }
            }
            "response.reasoning_summary_part.done" => {
                let _ = tx.send(StreamEvent::ReasoningBreak);
            }
            "response.output_item.added" => {
                let item = &v["item"];
                if matches!(item["type"].as_str(), Some("function_call") | Some("custom_tool_call")) {
                    let _ = tx.send(StreamEvent::ToolCallStarted { name: item["name"].as_str().unwrap_or("").to_string() });
                }
            }
            "response.output_item.done" => {
                let _ = tx.send(StreamEvent::ItemDone(v["item"].clone()));
            }
            "response.completed" => {
                let _ = tx.send(StreamEvent::Completed(usage_from(&v["response"])));
                return Ok(Some(true));
            }
            "response.incomplete" => {
                let reason = v["response"]["incomplete_details"]["reason"].as_str().unwrap_or("unknown");
                let _ = tx.send(StreamEvent::Completed(usage_from(&v["response"])));
                if reason == "max_output_tokens" {
                    return Ok(Some(true));
                }
                return Err(ApiError {
                    status: None,
                    code: Some("incomplete".into()),
                    message: format!("response incomplete: {reason}"),
                });
            }
            "response.failed" => {
                let e = &v["response"]["error"];
                let code = e["code"].as_str().map(String::from);
                let msg = e["message"].as_str().unwrap_or("response failed");
                return Err(ApiError { status: None, message: friendly_error(None, code.as_deref(), msg), code });
            }
            "error" => {
                let e = if v["error"].is_object() { &v["error"] } else { v };
                let code = e["code"].as_str().map(String::from);
                let msg = e["message"].as_str().unwrap_or("stream error");
                return Err(ApiError { status: None, message: friendly_error(None, code.as_deref(), msg), code });
            }
            _ => {}
        }
        Ok(None)
    }

    /// One non-streaming-to-the-UI completion (used for compaction): collects text.
    pub async fn complete_text(&self, body: Value, cancel: &CancellationToken) -> Result<(String, Usage), ApiError> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let ok = self.stream(body, &tx, cancel).await?;
        drop(tx);
        if !ok {
            return Err(ApiError { status: None, code: None, message: "cancelled".into() });
        }
        let mut text = String::new();
        let mut usage = Usage::default();
        while let Some(ev) = rx.recv().await {
            match ev {
                StreamEvent::TextDelta(d) => text.push_str(&d),
                StreamEvent::Completed(u) => usage = u,
                _ => {}
            }
        }
        Ok((text, usage))
    }

    pub async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let token = self.auth.bearer().await?;
        let r = self.http.get(format!("{}/models", self.base)).bearer_auth(token).send().await?;
        let status = r.status();
        let v: Value = r.json().await?;
        if !status.is_success() {
            anyhow::bail!("listing models failed: {}", parse_error_body(status.as_u16(), &v.to_string()).message);
        }
        let mut out = Vec::new();
        if let Some(models) = v["models"].as_array() {
            // ChatGPT plan listing: keep server order, only `visibility == "list"`.
            for m in models {
                if m["visibility"].as_str().map(|s| s == "list").unwrap_or(true) {
                    if let Some(slug) = m["slug"].as_str() {
                        out.push(ModelInfo {
                            slug: slug.into(),
                            display_name: m["display_name"].as_str().unwrap_or(slug).into(),
                            context_window: m["context_window"].as_u64().or_else(|| m["max_context_window"].as_u64()),
                            efforts: m["supported_reasoning_levels"]
                                .as_array()
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|e| e["effort"].as_str().or_else(|| e.as_str()).map(String::from))
                                        .collect()
                                })
                                .unwrap_or_default(),
                        });
                    }
                }
            }
        } else if let Some(data) = v["data"].as_array() {
            let mut ids: Vec<String> = data.iter().filter_map(|m| m["id"].as_str().map(String::from)).collect();
            ids.retain(|id| {
                let id = id.as_str();
                (id.starts_with("gpt-") || id.starts_with('o') || id.contains("codex"))
                    && !id.contains("audio")
                    && !id.contains("realtime")
                    && !id.contains("tts")
                    && !id.contains("transcribe")
                    && !id.contains("image")
                    && !id.contains("embedding")
                    && !id.contains("search")
            });
            ids.sort();
            ids.reverse();
            for id in ids {
                out.push(ModelInfo { display_name: id.clone(), slug: id, context_window: None, efforts: vec![] });
            }
        }
        Ok(out)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelInfo {
    pub slug: String,
    pub display_name: String,
    pub context_window: Option<u64>,
    pub efforts: Vec<String>,
}

fn find_event_end(buf: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i + 1 < buf.len() {
        if buf[i] == b'\n' && buf[i + 1] == b'\n' {
            return Some((i, 2));
        }
        if i + 3 < buf.len() && &buf[i..i + 4] == b"\r\n\r\n" {
            return Some((i, 4));
        }
        i += 1;
    }
    None
}

/// Build a Responses API request body.
pub fn request_body(model: &str, instructions: &str, input: &[Value], tools: &[Value], effort: &str, reasoning: bool) -> Value {
    let mut body = json!({
        "model": model,
        "instructions": instructions,
        "input": sanitize_input(input),
        "store": false,
        "stream": true,
        "parallel_tool_calls": true,
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.to_vec());
        body["tool_choice"] = json!("auto");
    }
    if reasoning {
        body["reasoning"] = json!({ "effort": effort, "summary": "auto" });
        body["include"] = json!(["reasoning.encrypted_content"]);
    }
    body
}

pub fn supports_reasoning(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    m.starts_with("gpt-5") || m.starts_with("gpt-6") || m.starts_with('o') || m.contains("codex")
}
