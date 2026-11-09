//! What each running chat is doing right now, folded from its events (the same way the UI's
//! `reduce` does), so a caller can ask "is it done?" or wait until it is. The app calls
//! [`Hub::push`] from every chat's event sink.

use std::collections::HashMap;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use oah_agents::event::{ApprovalRequest, BackgroundTask, ChatEvent, ModelOption, PlanItem, QuestionRequest, Usage};
use oah_core::settings::PermissionMode;
use serde::Serialize;

/// The longest text kept for a turn's reply (its end is kept).
const MAX_REPLY: usize = 20_000;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// A turn is running (from the user's message until it finishes).
    pub busy: bool,
    pub approvals: Vec<ApprovalRequest>,
    pub questions: Vec<QuestionRequest>,
    /// The tool running now (its one-line title).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_tool: Option<String>,
    /// What the agent said this turn (or in the last one, once it finished), per text item.
    #[serde(skip)]
    pub texts: Vec<(String, String)>,
    /// The tools run this turn, one line each (`✓ Bash: cargo test`).
    pub tools: Vec<String>,
    /// Files changed this turn: path, lines added, lines removed.
    pub files: Vec<(String, u32, u32)>,
    pub plan: Vec<PlanItem>,
    /// Background work: running, or ended since the user last wrote.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<BackgroundTask>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelOption>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<PermissionMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Context>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_turn: Option<TurnEnd>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Turns finished since the app started.
    pub turns: u32,
    /// Bumped on every event.
    #[serde(skip)]
    pub generation: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    pub used_tokens: u64,
    pub window_tokens: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnEnd {
    pub interrupted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

impl Status {
    /// `needsApproval`, `needsQuestion`, `running` or `idle`.
    pub fn state(&self) -> &'static str {
        if !self.approvals.is_empty() {
            "needsApproval"
        } else if !self.questions.is_empty() {
            "needsQuestion"
        } else if self.busy {
            "running"
        } else {
            "idle"
        }
    }

    pub fn needs_attention(&self) -> bool {
        !self.approvals.is_empty() || !self.questions.is_empty()
    }

    /// Ends a turn: nothing is waiting any more.
    fn settle(&mut self) {
        self.busy = false;
        self.approvals.clear();
        self.questions.clear();
        self.current_tool = None;
    }

    pub fn fold(&mut self, e: &ChatEvent) {
        self.generation += 1;
        match e {
            ChatEvent::Started { model, .. } => {
                if model.is_some() {
                    self.model = model.clone();
                }
            }
            ChatEvent::UserMessage { .. } | ChatEvent::TurnStarted { .. } => {
                if !self.busy {
                    // A new turn: what's reported is about it now.
                    self.texts.clear();
                    self.tools.clear();
                    self.files.clear();
                    self.last_error = None;
                }
                self.busy = true;
            }
            ChatEvent::TextDelta { item_id, text } => match self.texts.iter_mut().find(|t| &t.0 == item_id) {
                Some(t) => t.1.push_str(text),
                None => self.texts.push((item_id.clone(), text.clone())),
            },
            ChatEvent::Text { item_id, text } => match self.texts.iter_mut().find(|t| &t.0 == item_id) {
                Some(t) => t.1 = text.clone(),
                None => self.texts.push((item_id.clone(), text.clone())),
            },
            ChatEvent::ToolStarted { tool, .. } => self.current_tool = Some(format!("{}: {}", tool.name, tool.title)),
            ChatEvent::ToolFinished { ok, diff, .. } => {
                if let Some(t) = self.current_tool.take() {
                    self.tools.push(format!("{} {t}", if *ok { "✓" } else { "✗" }));
                }
                if let Some(d) = diff {
                    match self.files.iter_mut().find(|f| f.0 == d.path) {
                        Some(f) => {
                            f.1 += d.added;
                            f.2 += d.removed;
                        }
                        None => self.files.push((d.path.clone(), d.added, d.removed)),
                    }
                }
            }
            ChatEvent::Approval { request } => self.approvals.push(request.clone()),
            ChatEvent::ApprovalResolved { request_id, .. } => self.approvals.retain(|a| &a.request_id != request_id),
            ChatEvent::Question { request } => self.questions.push(request.clone()),
            ChatEvent::QuestionResolved { request_id, .. } => self.questions.retain(|q| &q.request_id != request_id),
            ChatEvent::Models { models, current } => {
                self.models = models.clone();
                if current.is_some() {
                    self.model = current.clone();
                }
            }
            ChatEvent::PermissionChanged { mode } => self.permission = Some(*mode),
            ChatEvent::Plan { items } => self.plan = items.clone(),
            ChatEvent::Tasks { tasks } => self.tasks = tasks.clone(),
            ChatEvent::TurnFinished { usage, cost_usd, duration_ms, interrupted, .. } => {
                self.settle();
                self.turns += 1;
                self.last_turn = Some(TurnEnd { interrupted: *interrupted, usage: *usage, cost_usd: *cost_usd, duration_ms: *duration_ms });
            }
            ChatEvent::Context { used_tokens, window_tokens, .. } => {
                self.context = Some(Context { used_tokens: *used_tokens, window_tokens: *window_tokens })
            }
            ChatEvent::Error { message } => {
                self.settle();
                self.last_error = Some(message.clone());
            }
            ChatEvent::Exited { code, message } => {
                self.settle();
                if code.is_some_and(|c| c != 0) {
                    self.last_error = Some(message.clone().unwrap_or_else(|| format!("The agent exited (code {}).", code.unwrap())));
                }
            }
            ChatEvent::ReasoningDelta { .. } | ChatEvent::Reasoning { .. } | ChatEvent::Commands { .. } | ChatEvent::Notice { .. } => {}
        }
    }
}

impl Status {
    /// What the agent said this turn, paragraphs in order; a long reply keeps its end.
    pub fn reply(&self) -> String {
        let mut s = self.texts.iter().map(|t| t.1.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join("\n\n");
        if s.len() > MAX_REPLY {
            let mut cut = s.len() - MAX_REPLY;
            while !s.is_char_boundary(cut) {
                cut += 1;
            }
            s.replace_range(..cut, "…");
        }
        s
    }
}

/// When [`Hub::wait`] returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Until {
    /// The turn finished (or the agent stopped).
    Idle,
    /// Idle, or an approval or question is waiting.
    Attention,
    /// Anything happened.
    Change,
}

#[derive(Default)]
pub struct Hub {
    chats: Mutex<HashMap<String, Status>>,
    changed: Condvar,
}

impl Hub {
    pub fn new() -> Hub {
        Hub::default()
    }

    /// Folds one event of `chat_id` in and wakes whoever waits on it.
    pub fn push(&self, chat_id: &str, event: &ChatEvent) {
        self.chats.lock().unwrap().entry(chat_id.to_string()).or_default().fold(event);
        self.changed.notify_all();
    }

    /// The chat's agent was stopped: nothing runs or waits any more.
    pub fn stopped(&self, chat_id: &str) {
        if let Some(s) = self.chats.lock().unwrap().get_mut(chat_id) {
            s.settle();
            s.generation += 1;
        }
        self.changed.notify_all();
    }

    pub fn forget(&self, chat_id: &str) {
        self.chats.lock().unwrap().remove(chat_id);
        self.changed.notify_all();
    }

    /// The chat's status; a chat that hasn't run since the app started is idle.
    pub fn status(&self, chat_id: &str) -> Status {
        self.chats.lock().unwrap().get(chat_id).cloned().unwrap_or_default()
    }

    /// Waits until `until` holds for `chat_id` or `timeout` passes; returns the status and
    /// whether it timed out. `tick` is called on every change and every `tick_every` while
    /// waiting (for progress notifications).
    pub fn wait(
        &self,
        chat_id: &str,
        until: Until,
        timeout: Duration,
        tick_every: Duration,
        mut tick: impl FnMut(&Status),
    ) -> (Status, bool) {
        let deadline = Instant::now() + timeout;
        let mut chats = self.chats.lock().unwrap();
        let start = chats.get(chat_id).map(|s| s.generation).unwrap_or(0);
        let mut seen = start;
        let mut last_tick = Instant::now();
        loop {
            let s = chats.get(chat_id).cloned().unwrap_or_default();
            let done = match until {
                Until::Idle => !s.busy,
                Until::Attention => !s.busy || s.needs_attention(),
                Until::Change => s.generation != start,
            };
            if done {
                return (s, false);
            }
            let now = Instant::now();
            if now >= deadline {
                return (s, true);
            }
            if s.generation != seen || now.duration_since(last_tick) >= tick_every {
                seen = s.generation;
                last_tick = now;
                drop(chats);
                tick(&s);
                chats = self.chats.lock().unwrap();
                continue;
            }
            let wake = (deadline - now).min(tick_every.saturating_sub(now.duration_since(last_tick)).max(Duration::from_millis(10)));
            chats = self.changed.wait_timeout(chats, wake).unwrap().0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oah_agents::event::{ApprovalKind, Decision, FileDiff, ToolCall, ToolKind};
    use std::sync::Arc;

    fn tool(name: &str, title: &str) -> ToolCall {
        ToolCall { kind: ToolKind::Bash, name: name.into(), title: title.into(), input: serde_json::Value::Null, path: None }
    }

    fn approval(id: &str) -> ApprovalRequest {
        ApprovalRequest {
            request_id: id.into(),
            item_id: None,
            kind: ApprovalKind::Command,
            title: "Run a command".into(),
            detail: Some("rm -rf target".into()),
            diff: None,
            can_remember: true,
        }
    }

    fn user(text: &str) -> ChatEvent {
        ChatEvent::UserMessage { id: "u".into(), text: text.into(), images: vec![], files: vec![] }
    }

    fn finished(interrupted: bool) -> ChatEvent {
        ChatEvent::TurnFinished { turn_id: "t".into(), usage: None, cost_usd: Some(0.01), duration_ms: Some(1200), interrupted }
    }

    #[test]
    fn folds_a_turn() {
        let mut s = Status::default();
        s.fold(&user("fix it"));
        assert_eq!(s.state(), "running");
        s.fold(&ChatEvent::TextDelta { item_id: "a".into(), text: "Look".into() });
        s.fold(&ChatEvent::TextDelta { item_id: "a".into(), text: "ing.".into() });
        s.fold(&ChatEvent::Text { item_id: "a".into(), text: "Looking.".into() });
        assert_eq!(s.reply(), "Looking.");
        s.fold(&ChatEvent::ToolStarted { item_id: "b".into(), tool: tool("Bash", "cargo test") });
        assert_eq!(s.current_tool.as_deref(), Some("Bash: cargo test"));
        s.fold(&ChatEvent::ToolFinished {
            item_id: "b".into(),
            ok: true,
            output: String::new(),
            diff: Some(FileDiff::from_patch("src/a.rs", "+x\n+y\n-z\n")),
        });
        s.fold(&ChatEvent::TextDelta { item_id: "c".into(), text: "Done.".into() });
        s.fold(&ChatEvent::Text { item_id: "c".into(), text: "Done.".into() });
        s.fold(&finished(false));
        assert_eq!(s.state(), "idle");
        assert_eq!(s.reply(), "Looking.\n\nDone.");
        assert_eq!(s.tools, vec!["✓ Bash: cargo test"]);
        assert_eq!(s.files, vec![("src/a.rs".to_string(), 2, 1)]);
        assert_eq!(s.turns, 1);
        // The next turn starts clean.
        s.fold(&user("again"));
        assert!(s.reply().is_empty() && s.tools.is_empty() && s.files.is_empty());
    }

    #[test]
    fn approvals_wait_until_answered_or_the_turn_ends() {
        let mut s = Status::default();
        s.fold(&user("go"));
        s.fold(&ChatEvent::Approval { request: approval("r1") });
        s.fold(&ChatEvent::Approval { request: approval("r2") });
        assert_eq!(s.state(), "needsApproval");
        s.fold(&ChatEvent::ApprovalResolved { request_id: "r1".into(), decision: Decision::Allow });
        assert_eq!(s.approvals.len(), 1);
        s.fold(&finished(true));
        assert_eq!(s.state(), "idle");
        assert!(s.last_turn.as_ref().unwrap().interrupted);
    }

    #[test]
    fn exit_with_an_error_is_reported() {
        let mut s = Status::default();
        s.fold(&user("go"));
        s.fold(&ChatEvent::Exited { code: Some(1), message: None });
        assert_eq!(s.state(), "idle");
        assert_eq!(s.last_error.as_deref(), Some("The agent exited (code 1)."));
    }

    #[test]
    fn wait_returns_when_the_turn_needs_attention() {
        let hub = Arc::new(Hub::new());
        hub.push("c", &user("go"));
        let h = hub.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            h.push("c", &ChatEvent::Approval { request: approval("r") });
        });
        let (s, timed_out) = hub.wait("c", Until::Attention, Duration::from_secs(5), Duration::from_secs(5), |_| {});
        t.join().unwrap();
        assert!(!timed_out);
        assert_eq!(s.state(), "needsApproval");
        // Idle still waits: the approval is part of the turn.
        let (_, timed_out) = hub.wait("c", Until::Idle, Duration::from_millis(30), Duration::from_secs(1), |_| {});
        assert!(timed_out);
        hub.stopped("c");
        let (s, timed_out) = hub.wait("c", Until::Idle, Duration::from_millis(30), Duration::from_secs(1), |_| {});
        assert!(!timed_out && s.state() == "idle");
    }

    #[test]
    fn ticks_while_waiting() {
        let hub = Hub::new();
        hub.push("c", &user("go"));
        let mut ticks = 0;
        let (_, timed_out) = hub.wait("c", Until::Idle, Duration::from_millis(120), Duration::from_millis(25), |_| ticks += 1);
        assert!(timed_out);
        assert!(ticks >= 2, "{ticks}");
    }

    #[test]
    fn an_unknown_chat_is_idle() {
        let hub = Hub::new();
        let (s, timed_out) = hub.wait("x", Until::Idle, Duration::from_millis(10), Duration::from_secs(1), |_| {});
        assert!(!timed_out && s.state() == "idle");
    }
}
