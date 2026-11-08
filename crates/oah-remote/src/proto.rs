//! The messages inside a session, as JSON.
//!
//! - phone → desktop: [`Request`] `{id, method, params}`
//! - desktop → phone: [`Response`] `{id, ok, result | error}`, or a push:
//!   `{push: "chat", chatId, event}` (an event of a chat the phone opened),
//!   `{push: "chats"}` (the chat list changed: list it again),
//!   `{push: "resync"}` (events were missed: open the chats again).

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub result: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    pub fn from(id: u64, r: Result<Value, String>) -> Response {
        match r {
            Ok(result) => Response { id, ok: true, result, error: None },
            Err(e) => Response { id, ok: false, result: Value::Null, error: Some(e) },
        }
    }
}

/// What the desktop sends: an answer, or a push.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Incoming {
    Response(Response),
    Push(Value),
}

/// Methods the server answers itself.
pub const HELLO: &str = "hello";
/// `{chatId}` → the chat's snapshot (from [`crate::server::Host::open_chat`]); its events push
/// from then on, until `chat.close`.
pub const CHAT_OPEN: &str = "chat.open";
pub const CHAT_CLOSE: &str = "chat.close";

/// Methods the server passes to the host. Anything not here (or above) is refused, so a paired
/// phone can do exactly this much.
pub const HOST_METHODS: &[&str] = &[
    // → [{id, name, location, path}]
    "projects.list",
    // → {agents: [{kind, acpId, name, defaultModel, permissionMode}], defaultAgent}
    "agents.list",
    // → {metas, running, busy}
    "chats.list",
    // {projectId, agent?, acpAgent?, model?, effort?, cwd?} → meta
    "chat.create",
    // {chatId, prompt: {id, text, images: [{mime, data}]}, permission?}
    "chat.send",
    // {chatId}
    "chat.interrupt",
    // {chatId, requestId, decision}
    "chat.answer",
    // {chatId, requestId, answers}
    "chat.answerQuestion",
    // {chatId, model} → meta
    "chat.setModel",
    // {chatId, effort} → meta
    "chat.setEffort",
    // {chatId, mode}
    "chat.setPermission",
    // {chatId, instructions, permission?}
    "chat.compact",
    // {chatId}
    "chat.clear",
    // {chatId, file} → data: URL
    "chat.image",
    // {chatId, title} → meta
    "chat.rename",
    // {kind, projectId?} → {models, commands}
    "provider.catalog",
];

pub fn chat_push(chat_id: &str, event: &Value) -> Value {
    serde_json::json!({ "push": "chat", "chatId": chat_id, "event": event })
}
