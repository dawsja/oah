//! The MCP tools: their schemas (written for a model to read) and what each does.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oah_agents::driver::{Image, Prompt};
use oah_agents::event::{ApprovalKind, Decision};
use oah_core::chats::ChatMeta;
use oah_core::settings::{AgentKind, Location, PermissionMode, Settings};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::host::{Host, NewChat};
use crate::hub::{Hub, Status, Until};
use crate::transcript::{self, cut};

/// How long a catalog (models, commands) is reused.
const CATALOG_TTL: Duration = Duration::from_secs(300);
/// The longest a call waits for a turn.
const MAX_WAIT: u64 = 1800;
/// Progress is reported at least this often while waiting.
const TICK: Duration = Duration::from_secs(10);

pub struct Ctx {
    pub host: Arc<dyn Host>,
    pub hub: Arc<Hub>,
    catalogs: Mutex<HashMap<String, (Instant, Value)>>,
}

impl Ctx {
    pub fn new(host: Arc<dyn Host>, hub: Arc<Hub>) -> Ctx {
        Ctx { host, hub, catalogs: Mutex::new(HashMap::new()) }
    }
}

/// Reports progress while a tool waits (a sentence about what's happening).
pub type Progress<'a> = &'a mut dyn FnMut(String);

/// What a tool returns: text for the model, or an error it can read and act on.
pub type ToolResult = Result<String, String>;

// ---- Schemas ---------------------------------------------------------------------------------

const PERMISSION: &str = "How much the agent may do without asking: `ask` (asks before edits and commands), `acceptEdits` (edits go through, commands ask), `plan` (read only, proposes a plan to approve), `fullAccess` (never asks; only if oah allows it in Settings → MCP).";

fn tool(name: &str, title: &str, description: &str, props: Value, required: &[&str], read_only: bool, destructive: bool) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": { "type": "object", "properties": props, "required": required, "additionalProperties": false },
        "annotations": { "title": title, "readOnlyHint": read_only, "destructiveHint": destructive, "openWorldHint": false },
    })
}

fn session_id() -> Value {
    json!({"type": "string", "description": "The session id (from create_session or list_sessions)."})
}

fn place() -> (Value, Value) {
    (
        json!({"type": "string", "description": "A worktree of the project by branch name (see list_worktrees)."}),
        json!({"type": "string", "description": "A folder inside the project (a worktree path). Default: the project folder."}),
    )
}

pub fn list(settings: &Settings) -> Vec<Value> {
    let (worktree, cwd) = place();
    let wait = json!({"type": "boolean", "description": "Wait for the turn to finish or to need you (an approval or a question) before returning. Default true."});
    let timeout = json!({"type": "integer", "minimum": 1, "maximum": MAX_WAIT, "description": "Longest wait, in seconds (default 300). On timeout the session keeps running: call wait_for_session."});
    let agent = json!({"type": "string", "enum": ["claude", "codex", "opencode", "acp"], "description": "The agent CLI: Claude Code, Codex, OpenCode, or an added ACP agent (then also acp_agent). Default: the project's, else oah's default."});
    let mut tools = vec![
        tool("list_projects", "List projects", "The projects open in oah (on this computer, in WSL or over SSH). Sessions run in one of them.", json!({}), &[], true, false),
        tool("list_worktrees", "List worktrees", "A project's git worktrees (one branch checked out per folder). A session can run in one so several work on the repo at once.",
            json!({"project_id": {"type": "string"}}), &["project_id"], true, false),
        tool("list_agents", "List agents", "The agent CLIs oah can run, whether each is turned on, installed and signed in where the project runs, and the default.",
            json!({"project_id": {"type": "string", "description": "Check where this project runs (default: this computer)."}}), &[], true, false),
        tool("list_models", "List models and efforts", "The models an agent offers and each one's reasoning effort levels (low … max), plus its slash commands. Pass session_id for what a running session's agent reports.",
            json!({
                "agent": agent.clone(),
                "acp_agent": {"type": "string", "description": "The added ACP agent's id (list_agents)."},
                "project_id": {"type": "string", "description": "Ask the CLI where this project runs."},
                "session_id": {"type": "string", "description": "Instead: the models a started session's agent reported."},
            }), &[], true, false),
        tool("list_sessions", "List sessions", "oah's chats (sessions), newest first, with their state: `running`, `needsApproval`, `needsQuestion` or `idle`.",
            json!({
                "project_id": {"type": "string"},
                "active_only": {"type": "boolean", "description": "Only sessions whose agent is running now."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 200, "description": "Default 30."},
            }), &[], true, false),
        tool("create_session", "Create a session", "Starts a new chat with a coding agent in a project. It shows in oah's sidebar, live. With `prompt` it starts working right away (and by default waits like send_message).",
            json!({
                "project_id": {"type": "string"},
                "agent": agent,
                "acp_agent": {"type": "string"},
                "model": {"type": "string", "description": "A model id from list_models. Default: the agent's."},
                "effort": {"type": "string", "description": "A reasoning effort the model offers (list_models). Default: the model's."},
                "permission": {"type": "string", "enum": ["ask", "acceptEdits", "plan", "fullAccess"], "description": PERMISSION},
                "worktree": {"type": "string", "description": "Run in this branch's worktree; a new branch gets a new worktree."},
                "cwd": cwd.clone(),
                "title": {"type": "string", "description": "Default: from the first message."},
                "prompt": {"type": "string", "description": "The first message."},
                "wait": wait.clone(),
                "timeout_s": timeout.clone(),
            }), &["project_id"], false, false),
        tool("send_message", "Send a message", "Sends a message to a session (starting its agent if needed). Sent while a turn runs, Claude and Codex take it into the turn and other agents run it next. Returns what the agent said, the tools it ran and files it changed, or what it's waiting for.",
            json!({
                "session_id": session_id(),
                "text": {"type": "string"},
                "images": {"type": "array", "items": {"type": "object", "properties": {"mime": {"type": "string"}, "data": {"type": "string", "description": "Base64."}}, "required": ["mime", "data"]}},
                "permission": {"type": "string", "enum": ["ask", "acceptEdits", "plan", "fullAccess"], "description": PERMISSION},
                "wait": wait,
                "timeout_s": timeout,
            }), &["session_id", "text"], false, false),
        tool("wait_for_session", "Wait for a session", "Waits until a session's turn finishes (`idle`), needs you (`attention`: an approval or a question, or finished), or anything happens (`change`). Returns the same report as send_message.",
            json!({
                "session_id": session_id(),
                "until": {"type": "string", "enum": ["attention", "idle", "change"], "description": "Default attention."},
                "timeout_s": {"type": "integer", "minimum": 1, "maximum": MAX_WAIT, "description": "Default 120."},
            }), &["session_id"], true, false),
        tool("get_session", "Get a session", "A session's settings and state now: what's waiting for an answer (with the full plan or command), the todo list, context use, and the reply so far.",
            json!({"session_id": session_id()}), &["session_id"], true, false),
        tool("get_transcript", "Read a session's transcript", "The conversation as Markdown: messages, replies, one line per tool, approvals and answers.",
            json!({
                "session_id": session_id(),
                "last_turns": {"type": "integer", "minimum": 0, "description": "Only the last this many turns; 0 is all. Default 3."},
                "tool_output": {"type": "boolean", "description": "Include tools' output (otherwise only failures'). Default false."},
            }), &["session_id"], true, false),
        tool("read_events", "Read a session's events", "The session's raw event log (oah's ChatEvent JSON) from a cursor, for exact detail. Use the returned `next` as `after` to continue.",
            json!({
                "session_id": session_id(),
                "after": {"type": "integer", "minimum": 0, "description": "Skip this many events. Default 0."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 500, "description": "Default 50."},
            }), &["session_id"], true, false),
        tool("interrupt_session", "Stop a turn", "Stops what the session's agent is doing. The session stays; send another message to go on.",
            json!({"session_id": session_id()}), &["session_id"], false, false),
        tool("answer_approval", "Answer an approval", "Answers what the agent asked permission for (a command, an edit, or a plan: allow approves the plan, deny keeps it planning). Whoever answers first, here or in oah, wins.",
            json!({
                "session_id": session_id(),
                "request_id": {"type": "string", "description": "From the report's pending approvals."},
                "decision": {"type": "string", "enum": ["allow", "allowAlways", "deny"], "description": "allowAlways: this and the same kind for the rest of the session (when offered)."},
            }), &["session_id", "request_id", "decision"], false, false),
        tool("answer_question", "Answer a question", "Answers questions the agent asked: option labels (several when multi-select), or typed text when allowed. An empty list skips a question.",
            json!({
                "session_id": session_id(),
                "request_id": {"type": "string"},
                "answers": {"type": "object", "additionalProperties": {"type": "array", "items": {"type": "string"}}, "description": "Question id → answers."},
            }), &["session_id", "request_id", "answers"], false, false),
        tool("set_session_options", "Change model, effort or permissions", "Switches a session's model, reasoning effort or permission mode, from the next message on (or now, where the agent can).",
            json!({
                "session_id": session_id(),
                "model": {"type": "string"},
                "effort": {"type": "string"},
                "permission": {"type": "string", "enum": ["ask", "acceptEdits", "plan", "fullAccess"], "description": PERMISSION},
            }), &["session_id"], false, false),
        tool("compact_session", "Compact a session", "Summarizes the session's conversation to free context (the agent's /compact).",
            json!({"session_id": session_id(), "instructions": {"type": "string", "description": "What the summary should keep."}}), &["session_id"], false, false),
        tool("clear_session", "Clear a session's context", "The agent starts fresh from the next message (the transcript stays).",
            json!({"session_id": session_id()}), &["session_id"], false, true),
        tool("rename_session", "Rename a session", "Sets the title oah shows.",
            json!({"session_id": session_id(), "title": {"type": "string"}}), &["session_id", "title"], false, false),
        tool("close_session", "Close a session's agent", "Ends the session's agent CLI (it resumes the conversation on the next message). Use when done with a session.",
            json!({"session_id": session_id()}), &["session_id"], false, false),
        tool("get_changes", "Uncommitted changes", "git status where the project (or a worktree) is: branch, and files staged, changed, untracked and conflicted with line counts.",
            json!({"project_id": {"type": "string"}, "worktree": worktree.clone(), "cwd": cwd.clone()}), &["project_id"], true, false),
        tool("get_diff", "Diff of a file", "The uncommitted changes to one file, as a unified diff.",
            json!({"project_id": {"type": "string"}, "path": {"type": "string", "description": "Relative to the folder."}, "worktree": worktree, "cwd": cwd}), &["project_id", "path"], true, false),
    ];
    if settings.mcp.allow_delete {
        tools.push(tool(
            "delete_session",
            "Delete a session",
            "Deletes a session and its transcript for good.",
            json!({"session_id": session_id()}),
            &["session_id"],
            false,
            true,
        ));
    }
    tools
}

// ---- Calls -----------------------------------------------------------------------------------

fn args<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, String> {
    serde_json::from_value(if v.is_null() { json!({}) } else { v }).map_err(|e| format!("bad arguments: {e}"))
}

fn to_text(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

pub fn call(ctx: &Ctx, name: &str, a: Value, progress: Progress) -> ToolResult {
    let settings = ctx.host.settings();
    match name {
        "list_projects" => Ok(to_text(&json!(settings
            .projects
            .iter()
            .map(|p| json!({
                "id": p.id,
                "name": p.name,
                "location": location_text(&p.location),
                "path": p.path,
                "defaultAgent": p.default_agent.unwrap_or(settings.providers.default_agent),
                "defaultModel": p.default_model,
            }))
            .collect::<Vec<_>>()))),
        "list_worktrees" => {
            #[derive(Deserialize)]
            struct A {
                project_id: String,
            }
            let a: A = args(a)?;
            Ok(to_text(&json!(ctx.host.worktrees(&a.project_id)?)))
        }
        "list_agents" => list_agents(ctx, &settings, args(a)?),
        "list_models" => list_models(ctx, args(a)?),
        "list_sessions" => list_sessions(ctx, args(a)?),
        "create_session" => create_session(ctx, &settings, args(a)?, progress),
        "send_message" => {
            #[derive(Deserialize)]
            struct A {
                session_id: String,
                text: String,
                #[serde(default)]
                images: Vec<Image>,
                permission: Option<PermissionMode>,
                wait: Option<bool>,
                timeout_s: Option<u64>,
            }
            let a: A = args(a)?;
            meta(ctx, &a.session_id)?;
            send(ctx, &settings, &a.session_id, a.text, a.images, a.permission, a.wait.unwrap_or(true), a.timeout_s, progress)
        }
        "wait_for_session" => {
            #[derive(Deserialize)]
            struct A {
                session_id: String,
                until: Option<String>,
                timeout_s: Option<u64>,
            }
            let a: A = args(a)?;
            meta(ctx, &a.session_id)?;
            let until = match a.until.as_deref().unwrap_or("attention") {
                "idle" => Until::Idle,
                "change" => Until::Change,
                "attention" => Until::Attention,
                other => return Err(format!("until is attention, idle or change, not {other}")),
            };
            let (status, timed_out) = wait(ctx, &a.session_id, until, a.timeout_s.unwrap_or(120), progress);
            Ok(to_text(&report(ctx, &a.session_id, &status, timed_out)))
        }
        "get_session" => {
            let a: Sid = args(a)?;
            let m = meta(ctx, &a.session_id)?;
            let status = ctx.hub.status(&a.session_id);
            let mut r = report(ctx, &a.session_id, &status, false);
            r["session"] = session_json(&m, live_state(ctx, &m.id, &status));
            if !status.plan.is_empty() {
                r["todo"] = json!(status.plan);
            }
            if let Some(c) = &status.context {
                r["context"] = json!(c);
            }
            if let Some(p) = status.permission {
                r["permission"] = json!(p);
            }
            if let Some(t) = &status.current_tool {
                r["currentTool"] = json!(t);
            }
            Ok(to_text(&r))
        }
        "get_transcript" => {
            #[derive(Deserialize)]
            struct A {
                session_id: String,
                last_turns: Option<usize>,
                #[serde(default)]
                tool_output: bool,
            }
            let a: A = args(a)?;
            meta(ctx, &a.session_id)?;
            let events = ctx.host.events(&a.session_id)?;
            let mut md =
                transcript::render(&events, &transcript::Options { last_turns: a.last_turns.unwrap_or(3), tool_output: a.tool_output });
            let status = ctx.hub.status(&a.session_id);
            if status.busy && is_live(ctx, &a.session_id) {
                md.push_str("\n\n_(still running");
                if let Some(t) = &status.current_tool {
                    md.push_str(&format!(": {t}"));
                }
                md.push_str(")_");
                let reply = status.reply();
                if !reply.is_empty() {
                    md.push_str(&format!("\n\nSo far this turn:\n{}", cut(&reply, 4000)));
                }
            }
            Ok(md)
        }
        "read_events" => {
            #[derive(Deserialize)]
            struct A {
                session_id: String,
                after: Option<usize>,
                limit: Option<usize>,
            }
            let a: A = args(a)?;
            meta(ctx, &a.session_id)?;
            let events = ctx.host.events(&a.session_id)?;
            let after = a.after.unwrap_or(0).min(events.len());
            let page: Vec<Value> = events[after..].iter().take(a.limit.unwrap_or(50).clamp(1, 500)).cloned().map(shrink).collect();
            let next = after + page.len();
            Ok(to_text(&json!({"events": page, "next": next, "total": events.len()})))
        }
        "interrupt_session" => {
            let a: Sid = args(a)?;
            meta(ctx, &a.session_id)?;
            ctx.host.interrupt(&a.session_id)?;
            Ok("Stopped. Send a message to go on.".into())
        }
        "answer_approval" => {
            #[derive(Deserialize)]
            struct A {
                session_id: String,
                request_id: String,
                decision: Decision,
            }
            let a: A = args(a)?;
            ctx.host.answer(&a.session_id, &a.request_id, a.decision)?;
            Ok("Answered. wait_for_session to follow the turn.".into())
        }
        "answer_question" => {
            #[derive(Deserialize)]
            struct A {
                session_id: String,
                request_id: String,
                answers: BTreeMap<String, Vec<String>>,
            }
            let a: A = args(a)?;
            ctx.host.answer_question(&a.session_id, &a.request_id, a.answers)?;
            Ok("Answered. wait_for_session to follow the turn.".into())
        }
        "set_session_options" => {
            #[derive(Deserialize)]
            struct A {
                session_id: String,
                model: Option<String>,
                effort: Option<String>,
                permission: Option<PermissionMode>,
            }
            let a: A = args(a)?;
            meta(ctx, &a.session_id)?;
            check_permission(&settings, a.permission)?;
            if let Some(m) = &a.model {
                ctx.host.set_model(&a.session_id, m)?;
            }
            if let Some(e) = &a.effort {
                ctx.host.set_effort(&a.session_id, e)?;
            }
            if let Some(p) = a.permission {
                ctx.host.set_permission(&a.session_id, p)?;
            }
            let m = meta(ctx, &a.session_id)?;
            Ok(to_text(&session_json(&m, live_state(ctx, &m.id, &ctx.hub.status(&m.id)))))
        }
        "compact_session" => {
            #[derive(Deserialize)]
            struct A {
                session_id: String,
                #[serde(default)]
                instructions: String,
            }
            let a: A = args(a)?;
            meta(ctx, &a.session_id)?;
            ctx.host.compact(&a.session_id, &a.instructions)?;
            Ok("Compacting. wait_for_session to know when it's done.".into())
        }
        "clear_session" => {
            let a: Sid = args(a)?;
            meta(ctx, &a.session_id)?;
            ctx.host.clear(&a.session_id)?;
            Ok("Cleared: the agent starts fresh from the next message.".into())
        }
        "rename_session" => {
            #[derive(Deserialize)]
            struct A {
                session_id: String,
                title: String,
            }
            let a: A = args(a)?;
            let m = ctx.host.rename(&a.session_id, &a.title)?;
            Ok(to_text(&session_json(&m, live_state(ctx, &m.id, &ctx.hub.status(&m.id)))))
        }
        "close_session" => {
            let a: Sid = args(a)?;
            meta(ctx, &a.session_id)?;
            ctx.host.close(&a.session_id);
            Ok("Closed. The next message starts the agent again, resuming the conversation.".into())
        }
        "delete_session" if settings.mcp.allow_delete => {
            let a: Sid = args(a)?;
            meta(ctx, &a.session_id)?;
            ctx.host.delete(&a.session_id)?;
            Ok("Deleted.".into())
        }
        "get_changes" => {
            #[derive(Deserialize)]
            struct A {
                project_id: String,
                worktree: Option<String>,
                cwd: Option<String>,
            }
            let a: A = args(a)?;
            let cwd = folder(ctx, &a.project_id, a.worktree.as_deref(), a.cwd, false)?;
            Ok(to_text(&ctx.host.git_status(&a.project_id, cwd.as_deref())?))
        }
        "get_diff" => {
            #[derive(Deserialize)]
            struct A {
                project_id: String,
                path: String,
                worktree: Option<String>,
                cwd: Option<String>,
            }
            let a: A = args(a)?;
            let cwd = folder(ctx, &a.project_id, a.worktree.as_deref(), a.cwd, false)?;
            let d = ctx.host.git_diff(&a.project_id, cwd.as_deref(), &a.path)?;
            Ok(if d.trim().is_empty() { "No uncommitted changes to that file.".into() } else { cut(&d, 60_000) })
        }
        _ => Err(format!("no tool named {name}")),
    }
}

#[derive(Deserialize)]
struct Sid {
    session_id: String,
}

fn location_text(l: &Location) -> String {
    match l {
        Location::Local => "this computer".into(),
        Location::Ssh { host } => format!("ssh:{host}"),
        Location::Wsl { distro } => format!("wsl:{distro}"),
    }
}

fn meta(ctx: &Ctx, id: &str) -> Result<ChatMeta, String> {
    ctx.host.chats().into_iter().find(|c| c.id == id).ok_or_else(|| format!("no session {id}: list_sessions shows them"))
}

fn is_live(ctx: &Ctx, id: &str) -> bool {
    ctx.host.running().iter().any(|r| r == id)
}

/// The state, minding that a stopped agent runs nothing whatever its last events said.
fn live_state(ctx: &Ctx, id: &str, s: &Status) -> &'static str {
    if is_live(ctx, id) {
        s.state()
    } else {
        "idle"
    }
}

fn session_json(m: &ChatMeta, state: &str) -> Value {
    json!({
        "sessionId": m.id,
        "title": m.title,
        "projectId": m.project_id,
        "agent": m.agent,
        "acpAgent": m.acp_agent,
        "model": m.model,
        "effort": m.effort,
        "cwd": m.cwd,
        "state": state,
        "updatedAt": m.updated_at,
        "tokens": m.tokens,
        "costUsd": m.cost_usd,
    })
}

fn check_permission(settings: &Settings, p: Option<PermissionMode>) -> Result<(), String> {
    if p == Some(PermissionMode::FullAccess) && !settings.mcp.allow_full_access {
        return Err("Full access isn't allowed over MCP (Settings → MCP in oah). Use acceptEdits, ask or plan.".into());
    }
    Ok(())
}

/// The folder for `worktree` (a branch) or `cwd`; with `create`, a missing branch gets a new
/// worktree.
fn folder(ctx: &Ctx, project_id: &str, worktree: Option<&str>, cwd: Option<String>, create: bool) -> Result<Option<String>, String> {
    let Some(branch) = worktree.map(str::trim).filter(|b| !b.is_empty()) else {
        return Ok(cwd.filter(|c| !c.trim().is_empty()));
    };
    let found = ctx.host.worktrees(project_id)?.into_iter().find(|w| w.branch.as_deref() == Some(branch));
    match found {
        Some(w) => Ok(Some(w.path)),
        None if create => Ok(Some(ctx.host.create_worktree(project_id, branch)?.path)),
        None => Err(format!("no worktree on branch {branch}: list_worktrees shows them")),
    }
}

fn list_agents(ctx: &Ctx, settings: &Settings, a: Value) -> ToolResult {
    #[derive(Deserialize)]
    struct A {
        project_id: Option<String>,
    }
    let a: A = args(a)?;
    let location = match a.project_id {
        Some(id) => settings.projects.iter().find(|p| p.id == id).map(|p| p.location.clone()).ok_or("no such project")?,
        None => Location::Local,
    };
    let mut agents: Vec<(AgentKind, Option<String>, String, bool, String)> = AgentKind::ALL
        .iter()
        .map(|&k| {
            let p = settings.providers.get(k);
            let name = match k {
                AgentKind::Claude => "Claude Code",
                AgentKind::Codex => "Codex",
                AgentKind::Opencode => "OpenCode",
                AgentKind::Acp => "ACP",
            };
            (k, None, name.to_string(), p.enabled, p.default_model.clone())
        })
        .collect();
    for acp in &settings.providers.acp {
        agents.push((AgentKind::Acp, Some(acp.id.clone()), acp.name.clone(), acp.provider.enabled, acp.provider.default_model.clone()));
    }
    // Each check runs a CLI (a few seconds); all at once.
    let checks: Vec<_> = std::thread::scope(|s| {
        let handles: Vec<_> = agents
            .iter()
            .map(|(k, acp, _, enabled, _)| {
                let location = &location;
                s.spawn(move || enabled.then(|| ctx.host.check_agent(*k, acp.as_deref(), location)))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or(None)).collect()
    });
    let list: Vec<Value> = agents
        .into_iter()
        .zip(checks)
        .map(|((kind, acp, name, enabled, model), check)| {
            let mut v = json!({"agent": kind, "name": name, "enabled": enabled, "default": acp.is_none() && kind == settings.providers.default_agent});
            if let Some(id) = acp {
                v["acpAgent"] = json!(id);
            }
            if !model.is_empty() {
                v["defaultModel"] = json!(model);
            }
            if let Some(c) = check {
                v["check"] = json!(c);
            }
            v
        })
        .collect();
    Ok(to_text(&json!({"where": location_text(&location), "agents": list})))
}

fn list_models(ctx: &Ctx, a: Value) -> ToolResult {
    #[derive(Deserialize)]
    struct A {
        agent: Option<AgentKind>,
        acp_agent: Option<String>,
        project_id: Option<String>,
        session_id: Option<String>,
    }
    let a: A = args(a)?;
    if let Some(id) = a.session_id {
        let m = meta(ctx, &id)?;
        let s = ctx.hub.status(&id);
        if !s.models.is_empty() {
            return Ok(to_text(&json!({"agent": m.agent, "current": s.model, "models": s.models})));
        }
        if m.agent == AgentKind::Acp {
            return Err("this session's agent hasn't listed its models yet: send it a message first".into());
        }
        return list_models(ctx, json!({"agent": m.agent, "project_id": m.project_id}));
    }
    let settings = ctx.host.settings();
    let kind = a.agent.unwrap_or_else(|| {
        a.project_id
            .as_deref()
            .and_then(|id| settings.projects.iter().find(|p| p.id == id))
            .and_then(|p| p.default_agent)
            .unwrap_or(settings.providers.default_agent)
    });
    if kind == AgentKind::Acp {
        let _ = a.acp_agent;
        return Err(
            "an ACP agent lists its models once a session runs: create_session, send a message, then list_models with its session_id"
                .into(),
        );
    }
    let key = format!("{kind:?}|{}", a.project_id.as_deref().unwrap_or(""));
    if let Some((at, v)) = ctx.catalogs.lock().unwrap().get(&key) {
        if at.elapsed() < CATALOG_TTL {
            return Ok(to_text(v));
        }
    }
    let (models, commands) = ctx.host.catalog(kind, a.project_id.as_deref())?;
    let v = json!({
        "agent": kind,
        "models": models,
        "commands": commands.iter().map(|c| json!({"name": c.name, "description": c.description, "hint": c.hint})).collect::<Vec<_>>(),
    });
    ctx.catalogs.lock().unwrap().insert(key, (Instant::now(), v.clone()));
    Ok(to_text(&v))
}

fn list_sessions(ctx: &Ctx, a: Value) -> ToolResult {
    #[derive(Deserialize)]
    struct A {
        project_id: Option<String>,
        #[serde(default)]
        active_only: bool,
        limit: Option<usize>,
    }
    let a: A = args(a)?;
    let running = ctx.host.running();
    let list: Vec<Value> = ctx
        .host
        .chats()
        .into_iter()
        .filter(|c| a.project_id.as_ref().is_none_or(|p| &c.project_id == p))
        .filter(|c| !a.active_only || running.contains(&c.id))
        .take(a.limit.unwrap_or(30).clamp(1, 200))
        .map(|c| {
            let state = if running.contains(&c.id) { ctx.hub.status(&c.id).state() } else { "idle" };
            session_json(&c, state)
        })
        .collect();
    Ok(to_text(&json!(list)))
}

fn create_session(ctx: &Ctx, settings: &Settings, a: Value, progress: Progress) -> ToolResult {
    #[derive(Deserialize)]
    struct A {
        project_id: String,
        agent: Option<AgentKind>,
        acp_agent: Option<String>,
        model: Option<String>,
        effort: Option<String>,
        permission: Option<PermissionMode>,
        worktree: Option<String>,
        cwd: Option<String>,
        title: Option<String>,
        prompt: Option<String>,
        wait: Option<bool>,
        timeout_s: Option<u64>,
    }
    let a: A = args(a)?;
    check_permission(settings, a.permission)?;
    if !settings.projects.iter().any(|p| p.id == a.project_id) {
        return Err("no such project: list_projects shows them".into());
    }
    let cwd = folder(ctx, &a.project_id, a.worktree.as_deref(), a.cwd, true)?;
    let mut m = ctx.host.create(NewChat {
        project_id: a.project_id,
        agent: a.agent,
        acp_agent: a.acp_agent,
        model: a.model,
        effort: a.effort,
        cwd,
    })?;
    if let Some(t) = a.title.filter(|t| !t.trim().is_empty()) {
        m = ctx.host.rename(&m.id, &t)?;
    }
    let prompt = a.prompt.filter(|p| !p.trim().is_empty());
    if let (Some(p), None) = (a.permission, &prompt) {
        ctx.host.set_permission(&m.id, p)?;
    }
    match prompt {
        Some(prompt) => {
            let out = send(ctx, settings, &m.id, prompt, vec![], a.permission, a.wait.unwrap_or(true), a.timeout_s, progress)?;
            Ok(format!("Session {} created.\n{out}", m.id))
        }
        None => Ok(to_text(&session_json(&m, "idle"))),
    }
}

#[allow(clippy::too_many_arguments)]
fn send(
    ctx: &Ctx,
    settings: &Settings,
    id: &str,
    text: String,
    images: Vec<Image>,
    permission: Option<PermissionMode>,
    wait_for_it: bool,
    timeout_s: Option<u64>,
    progress: Progress,
) -> ToolResult {
    check_permission(settings, permission)?;
    if text.trim().is_empty() && images.is_empty() {
        return Err("the message is empty".into());
    }
    // Announced, so the window sends with the same mode when someone types in the chat.
    if let Some(p) = permission {
        ctx.host.set_permission(id, p)?;
    }
    let prompt = Prompt { id: uuid::Uuid::new_v4().to_string(), text, images, files: vec![] };
    ctx.host.send(id, prompt, permission)?;
    if !wait_for_it {
        return Ok(to_text(&json!({"sessionId": id, "state": "running", "next": "wait_for_session to follow it"})));
    }
    let (status, timed_out) = wait(ctx, id, Until::Attention, timeout_s.unwrap_or(300), progress);
    Ok(to_text(&report(ctx, id, &status, timed_out)))
}

fn wait(ctx: &Ctx, id: &str, until: Until, timeout_s: u64, progress: Progress) -> (Status, bool) {
    let timeout = Duration::from_secs(timeout_s.clamp(1, MAX_WAIT));
    let started = Instant::now();
    ctx.hub.wait(id, until, timeout, TICK, |s| {
        let what = s.current_tool.clone().unwrap_or_else(|| s.state().to_string());
        progress(format!("{what} ({}s)", started.elapsed().as_secs()));
    })
}

/// What happened in the session's turn and what it waits for, with a hint of what to do next.
fn report(ctx: &Ctx, id: &str, s: &Status, timed_out: bool) -> Value {
    let state = if is_live(ctx, id) { s.state() } else { "idle" };
    let mut r = json!({"sessionId": id, "state": state});
    if timed_out {
        r["timedOut"] = json!(true);
    }
    let reply = s.reply();
    if !reply.is_empty() {
        r["reply"] = json!(reply);
    }
    if !s.tools.is_empty() {
        r["tools"] = json!(s.tools);
    }
    if !s.files.is_empty() {
        r["filesChanged"] = json!(s.files.iter().map(|f| json!({"path": f.0, "added": f.1, "removed": f.2})).collect::<Vec<_>>());
    }
    if !s.approvals.is_empty() {
        r["approvals"] = json!(s
            .approvals
            .iter()
            .map(|a| {
                let mut v = json!({"requestId": a.request_id, "kind": a.kind, "title": a.title, "canAllowAlways": a.can_remember});
                if let Some(d) = &a.detail {
                    // A plan is Markdown worth reading whole; a command is one line.
                    v[if a.kind == ApprovalKind::Plan { "plan" } else { "detail" }] = json!(cut(d, 20_000));
                }
                if let Some(d) = &a.diff {
                    v["diff"] = json!(cut(&d.patch, 8000));
                }
                v
            })
            .collect::<Vec<_>>());
    }
    if !s.questions.is_empty() {
        r["questions"] = json!(s.questions);
    }
    if let Some(t) = &s.last_turn {
        if !s.busy {
            r["lastTurn"] = json!(t);
        }
    }
    if let Some(e) = &s.last_error {
        r["error"] = json!(e);
    }
    r["next"] = json!(match state {
        "needsApproval" => "answer_approval with a requestId",
        "needsQuestion" => "answer_question with the requestId and answers by question id",
        "running" => "wait_for_session, send_message to steer, or interrupt_session",
        _ => "send_message to go on, get_changes to see what changed, or close_session when done",
    });
    r
}

/// Long tool output and file contents cut down, so a page of events stays readable.
fn shrink(mut v: Value) -> Value {
    fn walk(v: &mut Value, depth: usize) {
        match v {
            Value::String(s) if s.len() > 4000 => *s = cut(s, 4000),
            Value::Array(a) if depth < 6 => a.iter_mut().for_each(|x| walk(x, depth + 1)),
            Value::Object(o) if depth < 6 => o.values_mut().for_each(|x| walk(x, depth + 1)),
            _ => {}
        }
    }
    walk(&mut v, 0);
    v
}
