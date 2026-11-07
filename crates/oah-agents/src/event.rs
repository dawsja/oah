//! What a chat shows, whichever CLI is behind it. Each driver translates its CLI's protocol into
//! these events; the UI folds them into a transcript, and the app appends them to the chat's
//! `events.jsonl`, so reopening a chat is replaying its events.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ChatEvent {
    /// The CLI is up. `session_id` is the CLI's own id, used to resume the conversation later.
    Started { session_id: Option<String>, model: Option<String>, cwd: Option<String> },
    /// What the user sent (echoed so the transcript and the log hold it).
    UserMessage { id: String, text: String, images: Vec<String> },
    TurnStarted { turn_id: String },
    /// Streaming assistant text for `item_id`, appended.
    TextDelta { item_id: String, text: String },
    /// The whole assistant text for `item_id` (replaces what streamed).
    Text { item_id: String, text: String },
    ReasoningDelta { item_id: String, text: String },
    Reasoning { item_id: String, text: String },
    ToolStarted { item_id: String, tool: ToolCall },
    ToolFinished { item_id: String, ok: bool, output: String, diff: Option<FileDiff> },
    /// The agent asks before doing something.
    Approval { request: ApprovalRequest },
    ApprovalResolved { request_id: String, decision: Decision },
    /// The agent's plan / todo list, replaced whole.
    Plan { items: Vec<PlanItem> },
    TurnFinished { turn_id: String, usage: Option<Usage>, cost_usd: Option<f64>, duration_ms: Option<u64>, interrupted: bool },
    /// Context window fill after a turn, when the CLI reports it.
    Context { used_tokens: u64, window_tokens: u64 },
    Notice { level: NoticeLevel, text: String },
    Error { message: String },
    Exited { code: Option<i32>, message: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolKind {
    Read,
    Edit,
    Write,
    Bash,
    Search,
    Web,
    Task,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub kind: ToolKind,
    /// The CLI's own tool name (`Bash`, `apply_patch`…).
    pub name: String,
    /// One line: the command, the path, the pattern.
    pub title: String,
    /// The raw input, for the expanded view.
    pub input: serde_json::Value,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    /// Unified diff text (`@@` hunks, `+`/`-`/` ` lines).
    pub patch: String,
    pub added: u32,
    pub removed: u32,
}

impl FileDiff {
    /// Builds a diff from a patch, counting its added and removed lines.
    pub fn from_patch(path: impl Into<String>, patch: impl Into<String>) -> FileDiff {
        let patch = patch.into();
        let (mut added, mut removed) = (0, 0);
        for l in patch.lines() {
            if l.starts_with("+++") || l.starts_with("---") {
                continue;
            }
            if l.starts_with('+') {
                added += 1;
            } else if l.starts_with('-') {
                removed += 1;
            }
        }
        FileDiff { path: path.into(), patch, added, removed }
    }

    /// A diff of replacing `old` with `new` in `path` (a whole-hunk diff, no context search).
    pub fn replace(path: impl Into<String>, old: &str, new: &str) -> FileDiff {
        let mut patch = format!("@@ -1,{} +1,{} @@\n", old.lines().count(), new.lines().count());
        for l in old.lines() {
            patch.push('-');
            patch.push_str(l);
            patch.push('\n');
        }
        for l in new.lines() {
            patch.push('+');
            patch.push_str(l);
            patch.push('\n');
        }
        FileDiff::from_patch(path, patch)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalKind {
    Command,
    Edit,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub request_id: String,
    pub item_id: Option<String>,
    pub kind: ApprovalKind,
    /// "Run a command", "Edit src/main.rs".
    pub title: String,
    /// The command line, or why.
    pub detail: Option<String>,
    pub diff: Option<FileDiff>,
    /// Whether "allow for this chat" is offered.
    pub can_remember: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    Allow,
    /// Allow this and the same kind of request for the rest of the chat.
    AllowAlways,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanItem {
    pub text: String,
    pub status: PlanStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoticeLevel {
    Info,
    Warn,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

impl Usage {
    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_with_a_type_tag_in_camel_case() {
        let e = ChatEvent::TextDelta { item_id: "a1".into(), text: "hi".into() };
        assert_eq!(serde_json::to_string(&e).unwrap(), r#"{"type":"textDelta","itemId":"a1","text":"hi"}"#);
        let back: ChatEvent = serde_json::from_str(r#"{"type":"approvalResolved","requestId":"r","decision":"allowAlways"}"#).unwrap();
        assert_eq!(back, ChatEvent::ApprovalResolved { request_id: "r".into(), decision: Decision::AllowAlways });
    }

    #[test]
    fn counts_diff_lines() {
        let d = FileDiff::from_patch("a.rs", "--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@\n-old\n+new\n+more\n same\n");
        assert_eq!((d.added, d.removed), (2, 1));
        let r = FileDiff::replace("b.rs", "x\ny", "z");
        assert_eq!((r.added, r.removed), (1, 2));
    }
}
