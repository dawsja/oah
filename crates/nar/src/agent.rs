//! The agent loop: one user turn = repeated Responses API calls until the model
//! stops calling tools. Runs on the tokio runtime and reports to the UI thread
//! through [`AgentEvent`]s; the UI owns all conversation state.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::api::{self, Client, StreamEvent, Usage};
use crate::config::{Mode, Permission};
use crate::mcp::McpManager;
use crate::tools::{self, Risk, ToolCtx, ToolOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionReply {
    Once,
    Always,
    Deny,
}

pub enum AgentEvent {
    TextDelta(String),
    ReasoningDelta(String),
    ReasoningBreak,
    ToolStart { call_id: String, title: String },
    ToolEnd { call_id: String, outcome: ToolOutcome, denied: bool },
    Permission { title: String, kind: String, preview: Vec<String>, reply: oneshot::Sender<PermissionReply> },
    /// Items to append to the conversation, in order.
    Items(Vec<Value>),
    Usage(Usage),
    Todos(Vec<(String, String)>),
    GoalComplete(String),
    /// The model produced a final answer (no more tool calls).
    TurnDone { error: Option<String>, cancelled: bool },
    Compacted { summary: String, error: Option<String> },
}

pub type Sink = Arc<dyn Fn(AgentEvent) + Send + Sync>;

#[derive(Clone)]
pub struct TurnConfig {
    pub model: String,
    pub effort: String,
    pub mode: Mode,
    pub permission: Permission,
    pub instructions: String,
    pub goal_active: bool,
    pub allowed_commands: Vec<String>,
}

#[derive(Clone)]
pub struct Agent {
    pub client: Client,
    pub mcp: McpManager,
    pub ctx: ToolCtx,
    /// Session-scoped "always allow" grants (e.g. `bash:cargo`, `write`, an MCP tool name).
    pub grants: Arc<Mutex<HashSet<String>>>,
}

fn grant_key(name: &str, args: &Value) -> String {
    match name {
        "write_file" | "edit_file" => "write".into(),
        "bash" => {
            let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let first = cmd.split_whitespace().next().unwrap_or("");
            format!("bash:{first}")
        }
        "read_file" | "list_dir" | "glob" | "grep" => "read".into(),
        other => other.to_string(),
    }
}

fn needs_permission(perm: Permission, risk: Risk) -> bool {
    match perm {
        Permission::AlwaysAllow => false,
        Permission::AlwaysAsk => risk != Risk::Internal,
        Permission::Auto => matches!(risk, Risk::ReadOutside | Risk::Write | Risk::Exec | Risk::External),
    }
}

fn kind_label(risk: Risk) -> &'static str {
    match risk {
        Risk::Internal => "internal",
        Risk::Read => "read",
        Risk::ReadOutside => "read outside workspace",
        Risk::Write => "edit files",
        Risk::SafeExec => "run command",
        Risk::Exec => "run command",
        Risk::External => "MCP tool",
    }
}

impl Agent {
    async fn tool_specs(&self, cfg: &TurnConfig, has_skills: bool) -> Vec<Value> {
        let mut specs = tools::builtin_specs(cfg.mode, cfg.goal_active, has_skills);
        if cfg.mode == Mode::Build || cfg.permission != Permission::AlwaysAllow {
            specs.extend(self.mcp.specs().await);
        }
        specs
    }

    /// Run one user turn to completion. `items` already contains the new user message.
    pub async fn run_turn(&self, cfg: TurnConfig, mut items: Vec<Value>, sink: Sink, cancel: CancellationToken) {
        let has_skills = !self.ctx.skills.list.is_empty();
        let tools_spec = self.tool_specs(&cfg, has_skills).await;
        let reasoning = api::supports_reasoning(&cfg.model);
        let mut ctx = self.ctx.clone();
        ctx.cancel = cancel.clone();
        loop {
            let body = api::request_body(&cfg.model, &cfg.instructions, &items, &tools_spec, &cfg.effort, reasoning);
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<StreamEvent>();
            let client = self.client.clone();
            let c2 = cancel.clone();
            let req = tokio::spawn(async move { client.stream(body, &tx, &c2).await });
            let mut outputs: Vec<Value> = Vec::new();
            while let Some(ev) = rx.recv().await {
                match ev {
                    StreamEvent::TextDelta(d) => sink(AgentEvent::TextDelta(d)),
                    StreamEvent::ReasoningDelta(d) => sink(AgentEvent::ReasoningDelta(d)),
                    StreamEvent::ReasoningBreak => sink(AgentEvent::ReasoningBreak),
                    StreamEvent::ToolCallStarted => {}
                    StreamEvent::ItemDone(item) => outputs.push(item),
                    StreamEvent::Completed(u) => sink(AgentEvent::Usage(u)),
                }
            }
            let result = req.await.unwrap_or_else(|e| Err(api::ApiError { status: None, code: None, message: e.to_string() }));
            match result {
                Ok(true) => {}
                Ok(false) => {
                    sink(AgentEvent::TurnDone { error: None, cancelled: true });
                    return;
                }
                Err(e) => {
                    sink(AgentEvent::TurnDone { error: Some(e.message), cancelled: false });
                    return;
                }
            }
            // Keep only items that can be replayed.
            outputs.retain(|o| matches!(o["type"].as_str(), Some("message") | Some("reasoning") | Some("function_call")));
            for o in outputs.iter_mut() {
                if o["type"] == "reasoning" && o.get("encrypted_content").map(|v| v.is_null()).unwrap_or(true) {
                    // Without encrypted content a reasoning item cannot be replayed statelessly.
                    *o = Value::Null;
                }
            }
            outputs.retain(|o| !o.is_null());
            items.extend(outputs.iter().cloned());
            sink(AgentEvent::Items(outputs.clone()));

            let calls: Vec<Value> = outputs.iter().filter(|o| o["type"] == "function_call").cloned().collect();
            if calls.is_empty() {
                sink(AgentEvent::TurnDone { error: None, cancelled: false });
                return;
            }
            let mut results = Vec::new();
            for (i, call) in calls.iter().enumerate() {
                if cancel.is_cancelled() {
                    for rest in &calls[i..] {
                        results.push(json!({"type":"function_call_output","call_id":rest["call_id"],"output":"Interrupted by the user."}));
                    }
                    break;
                }
                let out = self.execute(call, &cfg, &ctx, &sink).await;
                results.push(out);
            }
            items.extend(results.iter().cloned());
            sink(AgentEvent::Items(results));
            if cancel.is_cancelled() {
                sink(AgentEvent::TurnDone { error: None, cancelled: true });
                return;
            }
        }
    }

    async fn execute(&self, call: &Value, cfg: &TurnConfig, ctx: &ToolCtx, sink: &Sink) -> Value {
        let name = call["name"].as_str().unwrap_or("").to_string();
        let call_id = call["call_id"].as_str().unwrap_or("").to_string();
        let raw_args = call["arguments"].as_str().unwrap_or("{}");
        let args: Value = serde_json::from_str(raw_args).unwrap_or(json!({}));
        let title = tools::title(&name, &args, &ctx.cwd);
        sink(AgentEvent::ToolStart { call_id: call_id.clone(), title: title.clone() });

        let risk = tools::risk(&name, &args, &ctx.cwd, &cfg.allowed_commands);
        let key = grant_key(&name, &args);
        let granted = self.grants.lock().unwrap().contains(&key);
        if needs_permission(cfg.permission, risk) && !granted {
            let (tx, rx) = oneshot::channel();
            sink(AgentEvent::Permission { title: title.clone(), kind: kind_label(risk).into(), preview: tools::permission_preview(&name, &args), reply: tx });
            let reply = tokio::select! {
                r = rx => r.unwrap_or(PermissionReply::Deny),
                _ = ctx.cancel.cancelled() => PermissionReply::Deny,
            };
            match reply {
                PermissionReply::Deny => {
                    let outcome = ToolOutcome {
                        output: "The user denied this tool call. Do not retry it; ask how to proceed if needed.".into(),
                        summary: "denied".into(),
                        detail: vec![],
                        ok: false,
                    };
                    sink(AgentEvent::ToolEnd { call_id: call_id.clone(), outcome: outcome.clone(), denied: true });
                    return json!({"type":"function_call_output","call_id":call_id,"output":outcome.output});
                }
                PermissionReply::Always => {
                    self.grants.lock().unwrap().insert(key);
                }
                PermissionReply::Once => {}
            }
        }

        let outcome = if tools::is_builtin(&name) {
            let o = tools::run_builtin(&name, &args, ctx, cfg.mode).await;
            if name == "todo_write" {
                let todos = args["items"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|i| (i["text"].as_str().unwrap_or("").to_string(), i["status"].as_str().unwrap_or("pending").to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                sink(AgentEvent::Todos(todos));
            }
            if name == "goal_complete" {
                sink(AgentEvent::GoalComplete(args["summary"].as_str().unwrap_or("").to_string()));
            }
            o
        } else if name.starts_with("mcp__") {
            let r = tokio::select! {
                r = self.mcp.call(&name, args.clone()) => r,
                _ = ctx.cancel.cancelled() => Err(anyhow::anyhow!("interrupted")),
            };
            match r {
                Ok((text, is_error)) => {
                    let lines = text.lines().count();
                    let preview: Vec<String> = text.lines().take(4).map(|l| tools::one_line(l, 160)).collect();
                    ToolOutcome {
                        output: if text.is_empty() { "(no output)".into() } else { text },
                        summary: if is_error { "error".into() } else { format!("{lines} lines") },
                        detail: preview,
                        ok: !is_error,
                    }
                }
                Err(e) => ToolOutcome { output: format!("Error: {e}"), summary: e.to_string(), detail: vec![], ok: false },
            }
        } else {
            ToolOutcome { output: format!("Error: unknown tool {name}"), summary: "unknown tool".into(), detail: vec![], ok: false }
        };
        sink(AgentEvent::ToolEnd { call_id: call_id.clone(), outcome: outcome.clone(), denied: false });
        json!({"type":"function_call_output","call_id":call_id,"output":outcome.output})
    }

    /// Summarize the conversation so it can continue in a fresh context.
    pub async fn compact(&self, cfg: TurnConfig, items: Vec<Value>, focus: String, sink: Sink, cancel: CancellationToken) {
        let mut input = items;
        let mut ask = String::from(
            "Your context window is being compacted. Write a detailed handoff summary of the conversation so far so that you can \
             continue seamlessly. Include: the user's goals and constraints; key decisions; files read or changed (with paths) and \
             what changed; commands run and their important results; errors encountered and fixes; the current state; and the \
             concrete next steps. Be specific and complete; do not call tools.",
        );
        if !focus.trim().is_empty() {
            ask.push_str(&format!("\nPay special attention to: {}", focus.trim()));
        }
        input.push(user_message(&ask));
        let reasoning = api::supports_reasoning(&cfg.model);
        let body = api::request_body(&cfg.model, &cfg.instructions, &input, &[], "low", reasoning);
        match self.client.complete_text(body, &cancel).await {
            Ok((summary, _)) if !summary.trim().is_empty() => sink(AgentEvent::Compacted { summary, error: None }),
            Ok(_) => sink(AgentEvent::Compacted { summary: String::new(), error: Some("empty summary".into()) }),
            Err(e) => sink(AgentEvent::Compacted { summary: String::new(), error: Some(e.message) }),
        }
    }
}

pub fn user_message(text: &str) -> Value {
    json!({"type":"message","role":"user","content":[{"type":"input_text","text":text}]})
}


pub fn system_prompt(
    cwd: &PathBuf,
    mode: Mode,
    goal: Option<&str>,
    skills: &str,
    memory: &str,
    mcp_servers: &[String],
) -> String {
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let git = cwd.join(".git").exists() || crate::memory::project_root(cwd).join(".git").exists();
    let mut s = format!(
        "You are nar, an expert software engineering agent running in the user's terminal. You help with coding tasks by \
reading and editing files and running commands in the workspace.

# Working style
- Be concise and direct. Use GitHub-flavored Markdown; keep answers short unless detail is requested.
- Explore before changing things: read relevant files, search with grep/glob, and understand conventions.
- Make focused, minimal changes that follow the existing style. Prefer edit_file over rewriting whole files.
- After changes, verify (build, tests, linters) when possible and report results honestly.
- For multi-step work, keep a todo list with todo_write and update it as you go.
- Never invent file contents or command output. If something fails, investigate the cause.
- Do not run destructive commands (rm -rf, git reset --hard, force pushes) unless the user explicitly asks.
- When the user states a durable preference or fact worth keeping across sessions, save it with the memory tool.

# Environment
- Working directory: {}
- Git repository: {}
- Platform: {} ({})
- Date: {}
",
        cwd.display(),
        if git { "yes" } else { "no" },
        std::env::consts::OS,
        std::env::consts::ARCH,
        date
    );
    match mode {
        Mode::Plan => s.push_str(
            "\n# Mode: PLAN\nYou are in plan mode. Do NOT modify files or run commands that change state. Investigate with read-only \
tools, then present a clear, numbered implementation plan (files to change, approach, risks, verification). Ask clarifying \
questions if requirements are ambiguous. The user will switch to build mode to execute.\n",
        ),
        Mode::Build => s.push_str("\n# Mode: BUILD\nYou may edit files and run commands (subject to the user's permission settings).\n"),
    }
    if let Some(g) = goal {
        s.push_str(&format!(
            "\n# Active goal\nThe user set a goal and wants you to keep working autonomously until it is completely done:\n\n{g}\n\n\
Work in small verified steps. Do not stop to ask for confirmation unless you are truly blocked. When — and only when — the goal \
is fully achieved and verified, call the goal_complete tool with a summary. If you end a turn without calling goal_complete you \
will be asked to continue.\n"
        ));
    }
    if !mcp_servers.is_empty() {
        s.push_str(&format!("\n# MCP\nConnected MCP servers: {}. Their tools are prefixed mcp__<server>__.\n", mcp_servers.join(", ")));
    }
    if !skills.is_empty() {
        s.push('\n');
        s.push_str(skills);
    }
    if !memory.is_empty() {
        s.push('\n');
        s.push_str(memory);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_matrix() {
        assert!(!needs_permission(Permission::Auto, Risk::Read));
        assert!(!needs_permission(Permission::Auto, Risk::SafeExec));
        assert!(needs_permission(Permission::Auto, Risk::Write));
        assert!(needs_permission(Permission::Auto, Risk::Exec));
        assert!(needs_permission(Permission::AlwaysAsk, Risk::Read));
        assert!(!needs_permission(Permission::AlwaysAsk, Risk::Internal));
        assert!(!needs_permission(Permission::AlwaysAllow, Risk::Exec));
    }

    #[test]
    fn grant_keys() {
        assert_eq!(grant_key("bash", &json!({"command":"cargo test --all"})), "bash:cargo");
        assert_eq!(grant_key("edit_file", &json!({})), "write");
    }
}
