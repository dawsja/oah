//! What the MCP tools need from the app. The desktop app implements this over its live chats
//! (`app/src-tauri/src/mcp.rs`); tests use a fake. Every method may block (it can start an agent
//! CLI or run git), and errors are the plain sentences oah shows anyway.

use std::collections::BTreeMap;

use oah_agents::driver::Prompt;
use oah_agents::event::{Decision, ModelOption, SlashCommand};
use oah_core::chats::ChatMeta;
use oah_core::settings::{AgentKind, Location, PermissionMode, Settings};
use serde::Serialize;

pub type HostResult<T> = Result<T, String>;

/// A new chat, as `chat_create` takes it: anything left out comes from the project and
/// Settings → Providers.
#[derive(Debug, Clone, Default)]
pub struct NewChat {
    pub project_id: String,
    pub agent: Option<AgentKind>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub cwd: Option<String>,
}

/// Whether an agent's CLI is there and signed in, where a project runs.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCheck {
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// None when the agent has no way to tell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signed_in: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: Option<String>,
    pub main: bool,
}

pub trait Host: Send + Sync + 'static {
    /// The settings as last saved (projects, providers, `mcp`).
    fn settings(&self) -> Settings;

    fn worktrees(&self, project_id: &str) -> HostResult<Vec<WorktreeInfo>>;
    /// A new worktree on a new branch, from the project's base branch.
    fn create_worktree(&self, project_id: &str, branch: &str) -> HostResult<WorktreeInfo>;

    /// `kind`'s CLI on `location`.
    fn check_agent(&self, kind: AgentKind, location: &Location) -> AgentCheck;
    /// The models (with their effort levels) and slash commands an agent offers where the
    /// project is.
    fn catalog(&self, kind: AgentKind, project_id: Option<&str>) -> HostResult<(Vec<ModelOption>, Vec<SlashCommand>)>;

    /// Every chat, newest first.
    fn chats(&self) -> Vec<ChatMeta>;
    /// Chats whose agent CLI is running.
    fn running(&self) -> Vec<String>;
    /// The chat's agent still has work: a turn running, or a message queued behind one (which
    /// the events alone can't show between one turn's end and the next one's start).
    fn busy(&self, chat_id: &str) -> bool;
    fn create(&self, chat: NewChat) -> HostResult<ChatMeta>;
    /// Sends a message, starting the chat's agent if it isn't running; returns once it's sent.
    fn send(&self, chat_id: &str, prompt: Prompt, permission: Option<PermissionMode>) -> HostResult<()>;
    fn interrupt(&self, chat_id: &str) -> HostResult<()>;
    fn answer(&self, chat_id: &str, request_id: &str, decision: Decision) -> HostResult<()>;
    fn answer_question(&self, chat_id: &str, request_id: &str, answers: BTreeMap<String, Vec<String>>) -> HostResult<()>;
    fn set_model(&self, chat_id: &str, model: &str) -> HostResult<()>;
    fn set_effort(&self, chat_id: &str, effort: &str) -> HostResult<()>;
    fn set_permission(&self, chat_id: &str, mode: PermissionMode) -> HostResult<()>;
    /// The mode the chat's next message runs with when none is given: the one it last ran
    /// with, else its agent's default in Settings → Providers.
    fn permission(&self, chat_id: &str) -> PermissionMode;
    fn compact(&self, chat_id: &str, instructions: &str) -> HostResult<()>;
    fn clear(&self, chat_id: &str) -> HostResult<()>;
    fn rename(&self, chat_id: &str, title: &str) -> HostResult<ChatMeta>;
    fn close(&self, chat_id: &str);
    fn delete(&self, chat_id: &str) -> HostResult<()>;
    /// The chat's logged events (`events.jsonl`), oldest first.
    fn events(&self, chat_id: &str) -> HostResult<Vec<serde_json::Value>>;

    /// `git status` where the project (or a worktree of it) is, as JSON.
    fn git_status(&self, project_id: &str, cwd: Option<&str>) -> HostResult<serde_json::Value>;
    /// The unified diff of one file's uncommitted changes.
    fn git_diff(&self, project_id: &str, cwd: Option<&str>, path: &str) -> HostResult<String>;
}
