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
/// The most of the agent's answer a report carries (its end is kept).
const MAX_ANSWER: usize = 8_000;
/// Tool lines a report lists (the count says how many ran).
const LAST_TOOLS: usize = 8;
/// Files listed per group in get_changes.
const MAX_FILES: usize = 100;
/// A page of get_diff.
const DIFF_PAGE: usize = 20_000;
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
//
// Every client sends this list to its model on every turn, so each thing is said once: what a
// session id is (in the server's instructions), what the permission modes mean (on
// create_session), and so on.

const PERMISSION: &str = "`ask` (asks before edits and commands), `acceptEdits` (edits go through, commands ask), `plan` (read only, proposes a plan), `fullAccess` (never asks; only if oah allows it). Default: the session's own, else the agent's.";
const MODES: [&str; 4] = ["ask", "acceptEdits", "plan", "fullAccess"];

fn tool(name: &str, title: &str, description: &str, props: Value, required: &[&str], read_only: bool, destructive: bool) -> Value {
    let mut schema = json!({ "type": "object", "properties": props });
    if !required.is_empty() {
        schema["required"] = json!(required);
    }
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": schema,
        "annotations": { "readOnlyHint": read_only, "destructiveHint": destructive, "openWorldHint": false },
    })
}

fn string() -> Value {
    json!({"type": "string"})
}

fn session_id() -> Value {
    string()
}

fn mode(described: bool) -> Value {
    if described {
        json!({"type": "string", "enum": MODES, "description": PERMISSION})
    } else {
        json!({"type": "string", "enum": MODES, "description": "As in create_session."})
    }
}

fn place() -> (Value, Value) {
    (
        json!({"type": "string", "description": "Run in this branch's worktree (list_worktrees)."}),
        json!({"type": "string", "description": "A folder in the project or a worktree. Default: the project folder."}),
    )
}

pub fn list(settings: &Settings) -> Vec<Value> {
    let (worktree, cwd) = place();
    let agents: Vec<Value> = AgentKind::ALL.iter().map(|k| json!(k)).collect();
    let agent = json!({"type": "string", "enum": agents, "description": "Default: the project's, else oah's (list_agents)."});
    let wait = json!({"type": "boolean", "description": "Wait until the turn finishes or needs an answer. Default true."});
    let timeout = |default: u64| json!({"type": "integer", "minimum": 1, "maximum": MAX_WAIT, "description": format!("Seconds, default {default}. The session keeps running after.")});
    let mut tools = vec![
        tool("list_projects", "List projects", "Projects open in oah (this computer, WSL or SSH).", json!({}), &[], true, false),
        tool("list_worktrees", "List worktrees", "A project's git worktrees; a session can run in one, so several work on the repo at once.",
            json!({"project_id": string()}), &["project_id"], true, false),
        tool("list_agents", "List agents", "Agent CLIs oah can run: on, installed and signed in where the project runs, and the default.",
            json!({"project_id": {"type": "string", "description": "Check there. Default: this computer."}}), &[], true, false),
        tool("list_models", "List models and efforts", "An agent's models and each one's effort levels.",
            json!({
                "agent": agent.clone(),
                "project_id": string(),
                "session_id": {"type": "string", "description": "Instead: what this session's agent reports."},
                "commands": {"type": "boolean", "description": "Also its slash commands. Default false."},
            }), &[], true, false),
        tool("list_sessions", "List sessions", "Chats, newest first, with their state.",
            json!({
                "project_id": string(),
                "active_only": {"type": "boolean", "description": "Only sessions whose agent runs now."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 200, "description": "Default 15."},
            }), &[], true, false),
        tool("create_session", "Create a session", "A new chat with an agent in a project, shown live in oah. With `prompt` it starts working (and waits, like send_message).",
            json!({
                "project_id": string(),
                "agent": agent,
                "model": {"type": "string", "description": "From list_models. Default: the agent's."},
                "effort": {"type": "string", "description": "One the model offers (list_models)."},
                "permission": mode(true),
                "worktree": {"type": "string", "description": "Run in this branch's worktree; a new branch gets one."},
                "cwd": cwd.clone(),
                "title": string(),
                "prompt": string(),
                "wait": wait.clone(),
                "timeout_s": timeout(300),
            }), &["project_id"], false, false),
        tool("send_message", "Send a message", "Sends a message, starting the agent if needed. During a turn, Claude and Codex take it in; others run it next. Returns a report: the agent's answer, tools run, files changed, or what it waits for.",
            json!({
                "session_id": session_id(),
                "text": string(),
                "images": {"type": "array", "items": {"type": "object", "properties": {"mime": string(), "data": {"type": "string", "description": "Base64."}}, "required": ["mime", "data"]}},
                "permission": mode(false),
                "wait": wait,
                "timeout_s": timeout(300),
            }), &["session_id", "text"], false, false),
        tool("wait_for_session", "Wait for a session", "Waits until the turn finishes (`idle`), needs an answer or finishes (`attention`), or anything happens (`change`); returns a report.",
            json!({
                "session_id": session_id(),
                "until": {"type": "string", "enum": ["attention", "idle", "change"], "description": "Default attention."},
                "timeout_s": timeout(120),
            }), &["session_id"], true, false),
        tool("get_session", "Get a session", "Settings and state now: what waits for an answer, the todo list, context use, the answer so far.",
            json!({"session_id": session_id()}), &["session_id"], true, false),
        tool("get_transcript", "Read a session's transcript", "The conversation as Markdown, one line per tool.",
            json!({
                "session_id": session_id(),
                "last_turns": {"type": "integer", "minimum": 0, "description": "0 is all. Default 3."},
                "tool_output": {"type": "boolean", "description": "Every tool's output, not just failures'."},
                "max_chars": {"type": "integer", "minimum": 1000, "description": "Default 12000; the oldest part is cut."},
            }), &["session_id"], true, false),
        tool("read_events", "Read a session's events", "The raw event log (oah's ChatEvent JSON) from a cursor; `next` continues.",
            json!({
                "session_id": session_id(),
                "after": {"type": "integer", "minimum": 0, "description": "Skip this many. Default 0."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 500, "description": "Default 20."},
                "all": {"type": "boolean", "description": "Also model lists, command lists, context and reasoning. Default false."},
            }), &["session_id"], true, false),
        tool("interrupt_session", "Stop a turn", "Stops the agent's turn; the session stays.",
            json!({"session_id": session_id()}), &["session_id"], false, false),
        tool("answer_approval", "Answer an approval", "Answers a pending approval (a command, an edit, or a plan: allow approves it, deny keeps planning). First answer, here or in oah, wins.",
            json!({
                "session_id": session_id(),
                "request_id": string(),
                "decision": {"type": "string", "enum": ["allow", "allowAlways", "deny"], "description": "allowAlways: this kind, for the rest of the session (when offered)."},
            }), &["session_id", "request_id", "decision"], false, false),
        tool("answer_question", "Answer a question", "Answers the agent's questions: option labels, or typed text where allowed; an empty list skips one.",
            json!({
                "session_id": session_id(),
                "request_id": string(),
                "answers": {"type": "object", "additionalProperties": {"type": "array", "items": string()}, "description": "Question id → answers."},
            }), &["session_id", "request_id", "answers"], false, false),
        tool("set_session_options", "Change model, effort or permissions", "From the next message on (now, where the agent can).",
            json!({"session_id": session_id(), "model": string(), "effort": string(), "permission": mode(false)}), &["session_id"], false, false),
        tool("compact_session", "Compact a session", "The agent's /compact: summarizes the conversation to free context.",
            json!({"session_id": session_id(), "instructions": {"type": "string", "description": "What to keep."}}), &["session_id"], false, false),
        tool("clear_session", "Clear a session's context", "The agent starts fresh from the next message; the transcript stays.",
            json!({"session_id": session_id()}), &["session_id"], false, true),
        tool("rename_session", "Rename a session", "Sets the title oah shows.",
            json!({"session_id": session_id(), "title": string()}), &["session_id", "title"], false, false),
        tool("close_session", "Close a session's agent", "Ends its agent CLI (the next message resumes it). Use when done. Also ends the agent's own CLI when the session is open in it on the computer, which is when messages to it are refused.",
            json!({"session_id": session_id()}), &["session_id"], false, false),
        tool("get_changes", "Uncommitted changes", "git status where the project or a worktree is: branch, and files with line counts.",
            json!({"project_id": string(), "worktree": worktree.clone(), "cwd": cwd.clone()}), &["project_id"], true, false),
        tool("get_diff", "Diff of a file", "One file's uncommitted changes as a unified diff, in pages.",
            json!({
                "project_id": string(),
                "path": {"type": "string", "description": "From the folder."},
                "offset": {"type": "integer", "minimum": 0, "description": "Start here (a page's `next`). Default 0."},
                "worktree": worktree,
                "cwd": cwd,
            }), &["project_id", "path"], true, false),
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

/// A result as compact JSON: the client's model reads every byte of it, and indentation is
/// 20–60% of a pretty-printed result.
fn to_text(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
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
            if !status.tasks.is_empty() {
                r["backgroundTasks"] = json!(status.tasks);
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
                max_chars: Option<usize>,
            }
            let a: A = args(a)?;
            meta(ctx, &a.session_id)?;
            let events = ctx.host.events(&a.session_id)?;
            let max = a.max_chars.unwrap_or(12_000).max(1000);
            let mut md = transcript::render(
                &events,
                &transcript::Options { last_turns: a.last_turns.unwrap_or(3), tool_output: a.tool_output, request_ids: true },
            );
            let status = ctx.hub.status(&a.session_id);
            if status.busy && is_live(ctx, &a.session_id) {
                md.push_str("\n\n_(still running");
                if let Some(t) = &status.current_tool {
                    md.push_str(&format!(": {t}"));
                }
                md.push_str(")_");
                // Its newest words, which aren't in the log yet.
                let reply = status.answer(4000);
                if !reply.is_empty() {
                    md.push_str(&format!("\n\nSo far this turn:\n{reply}"));
                }
            }
            Ok(keep_end(&md, max, "(earlier turns cut: pass last_turns or a larger max_chars)"))
        }
        "read_events" => {
            #[derive(Deserialize)]
            struct A {
                session_id: String,
                after: Option<usize>,
                limit: Option<usize>,
                #[serde(default)]
                all: bool,
            }
            let a: A = args(a)?;
            meta(ctx, &a.session_id)?;
            let events = ctx.host.events(&a.session_id)?;
            let after = a.after.unwrap_or(0).min(events.len());
            let limit = a.limit.unwrap_or(20).clamp(1, 500);
            // Lists and meters that replace each other say little and cost a lot.
            let noise = |e: &Value| {
                matches!(e["type"].as_str(), Some("models" | "commands" | "context" | "reasoning" | "reasoningDelta" | "tasks"))
            };
            let mut next = after;
            let mut page = Vec::new();
            for e in &events[after..] {
                if page.len() == limit {
                    break;
                }
                next += 1;
                if a.all || !noise(e) {
                    page.push(shrink(e.clone()));
                }
            }
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
            check_permission(&settings, Some(ctx.host.permission(&a.session_id)))?;
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
            let mut st = ctx.host.git_status(&a.project_id, cwd.as_deref())?;
            // A repo with no .gitignore can list thousands of untracked files.
            for group in ["staged", "unstaged", "untracked", "conflicted"] {
                if let Some(list) = st.get_mut(group).and_then(Value::as_array_mut) {
                    if list.len() > MAX_FILES {
                        let more = list.len() - MAX_FILES;
                        list.truncate(MAX_FILES);
                        st[format!("{group}More")] = json!(more);
                    }
                }
            }
            Ok(to_text(&st))
        }
        "get_diff" => {
            #[derive(Deserialize)]
            struct A {
                project_id: String,
                path: String,
                offset: Option<usize>,
                worktree: Option<String>,
                cwd: Option<String>,
            }
            let a: A = args(a)?;
            let cwd = folder(ctx, &a.project_id, a.worktree.as_deref(), a.cwd, false)?;
            let d = ctx.host.git_diff(&a.project_id, cwd.as_deref(), &a.path)?;
            if d.trim().is_empty() {
                return Ok("No uncommitted changes to that file.".into());
            }
            Ok(page(&d, a.offset.unwrap_or(0), DIFF_PAGE))
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
        "model": m.model,
        "effort": m.effort,
        "cwd": m.cwd,
        "state": state,
        "updatedAt": m.updated_at,
        "tokens": m.tokens,
        "costUsd": m.cost_usd,
    })
}

/// Refuses Full access unless Settings → MCP allows it. Callers pass the mode the turn will
/// really run with (given, else the chat's own, else the agent's default), not just what the
/// client asked for: a chat set to Full access in the window is Full access here too.
fn check_permission(settings: &Settings, p: Option<PermissionMode>) -> Result<(), String> {
    if p == Some(PermissionMode::FullAccess) && !settings.mcp.allow_full_access {
        return Err("Full access isn't allowed over MCP (Settings → MCP in oah). This would run with Full access; pass permission acceptEdits, ask or plan.".into());
    }
    Ok(())
}

/// `cwd` must be the project's folder, a folder inside it, or one of its worktrees.
fn check_cwd(ctx: &Ctx, project_id: &str, cwd: &str) -> Result<(), String> {
    let settings = ctx.host.settings();
    let project = settings.projects.iter().find(|p| p.id == project_id).ok_or("no such project: list_projects shows them")?;
    let norm = |p: &str| p.trim().trim_end_matches(['/', '\\']).replace('\\', "/");
    let (cwd, root) = (norm(cwd), norm(&project.path));
    if cwd.split('/').any(|c| c == "..") {
        return Err("cwd can't contain `..`".into());
    }
    let inside = |base: &str| cwd == base || cwd.starts_with(&format!("{base}/"));
    if inside(&root) || ctx.host.worktrees(project_id).unwrap_or_default().iter().any(|w| inside(&norm(&w.path))) {
        return Ok(());
    }
    Err("cwd must be the project's folder, a folder inside it, or one of its worktrees (list_worktrees)".into())
}

/// The folder for `worktree` (a branch) or `cwd`; with `create`, a missing branch gets a new
/// worktree.
fn folder(ctx: &Ctx, project_id: &str, worktree: Option<&str>, cwd: Option<String>, create: bool) -> Result<Option<String>, String> {
    let Some(branch) = worktree.map(str::trim).filter(|b| !b.is_empty()) else {
        let cwd = cwd.filter(|c| !c.trim().is_empty());
        if let Some(c) = &cwd {
            check_cwd(ctx, project_id, c)?;
        }
        return Ok(cwd);
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
    let agents: Vec<(AgentKind, &str, bool, String)> = AgentKind::ALL
        .iter()
        .map(|&k| {
            let p = settings.providers.get(k);
            let name = k.label();
            (k, name, p.enabled, p.default_model.clone())
        })
        .collect();
    // Each check runs a CLI (a few seconds); all at once.
    let checks: Vec<_> = std::thread::scope(|s| {
        let handles: Vec<_> = agents
            .iter()
            .map(|(k, _, enabled, _)| {
                let location = &location;
                s.spawn(move || enabled.then(|| ctx.host.check_agent(*k, location)))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or(None)).collect()
    });
    let list: Vec<Value> = agents
        .into_iter()
        .zip(checks)
        .map(|((kind, name, enabled, model), check)| {
            let mut v = json!({"agent": kind, "name": name, "enabled": enabled, "default": kind == settings.providers.default_agent});
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
        project_id: Option<String>,
        session_id: Option<String>,
        #[serde(default)]
        commands: bool,
    }
    let a: A = args(a)?;
    let with_commands = a.commands;
    let shape = move |mut v: Value| {
        if !with_commands {
            if let Some(o) = v.as_object_mut() {
                o.remove("commands");
            }
        }
        to_text(&v)
    };
    if let Some(id) = a.session_id {
        let m = meta(ctx, &id)?;
        let s = ctx.hub.status(&id);
        if !s.models.is_empty() {
            return Ok(to_text(&json!({"agent": m.agent, "current": s.model, "models": s.models})));
        }
        return list_models(ctx, json!({"agent": m.agent, "project_id": m.project_id, "commands": with_commands}));
    }
    let settings = ctx.host.settings();
    let kind = a.agent.unwrap_or_else(|| {
        a.project_id
            .as_deref()
            .and_then(|id| settings.projects.iter().find(|p| p.id == id))
            .and_then(|p| p.default_agent)
            .unwrap_or(settings.providers.default_agent)
    });
    let key = format!("{kind:?}|{}", a.project_id.as_deref().unwrap_or(""));
    if let Some((at, v)) = ctx.catalogs.lock().unwrap().get(&key) {
        if at.elapsed() < CATALOG_TTL {
            return Ok(shape(v.clone()));
        }
    }
    let (models, commands) = ctx.host.catalog(kind, a.project_id.as_deref())?;
    let v = json!({
        "agent": kind,
        "models": models,
        "commands": commands.iter().map(|c| json!({"name": c.name, "description": c.description, "hint": c.hint})).collect::<Vec<_>>(),
    });
    ctx.catalogs.lock().unwrap().insert(key, (Instant::now(), v.clone()));
    Ok(shape(v))
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
        .take(a.limit.unwrap_or(15).clamp(1, 200))
        .map(|c| {
            let state = if running.contains(&c.id) { ctx.hub.status(&c.id).state() } else { "idle" };
            // Enough to pick one; get_session has the rest.
            let mut v = json!({"sessionId": c.id, "title": c.title, "state": state, "agent": c.agent});
            if !c.model.is_empty() {
                v["model"] = json!(c.model);
            }
            if a.project_id.is_none() {
                v["projectId"] = json!(c.project_id);
            }
            v
        })
        .collect();
    Ok(to_text(&json!(list)))
}

fn create_session(ctx: &Ctx, settings: &Settings, a: Value, progress: Progress) -> ToolResult {
    #[derive(Deserialize)]
    struct A {
        project_id: String,
        agent: Option<AgentKind>,
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
    let Some(project) = settings.projects.iter().find(|p| p.id == a.project_id) else {
        return Err("no such project: list_projects shows them".into());
    };
    // What the new chat would run with: the mode given, else its agent's default.
    let agent = a.agent.or(project.default_agent).unwrap_or(settings.providers.default_agent);
    check_permission(settings, Some(a.permission.unwrap_or(settings.providers.get(agent).permission_mode)))?;
    let cwd = folder(ctx, &a.project_id, a.worktree.as_deref(), a.cwd, true)?;
    let mut m = ctx.host.create(NewChat { project_id: a.project_id, agent: a.agent, model: a.model, effort: a.effort, cwd })?;
    if let Some(t) = a.title.filter(|t| !t.trim().is_empty()) {
        m = ctx.host.rename(&m.id, &t)?;
    }
    let prompt = a.prompt.filter(|p| !p.trim().is_empty());
    if let (Some(p), None) = (a.permission, &prompt) {
        ctx.host.set_permission(&m.id, p)?;
    }
    match prompt {
        Some(prompt) => match send(ctx, settings, &m.id, prompt, vec![], a.permission, a.wait.unwrap_or(true), a.timeout_s, progress) {
            Ok(out) => Ok(format!("Session {} created.\n{out}", m.id)),
            // Its agent didn't start (turned off, not installed…): don't leave an empty chat.
            Err(e) => {
                ctx.host.close(&m.id);
                let _ = ctx.host.delete(&m.id);
                Err(e)
            }
        },
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
    check_permission(settings, Some(permission.unwrap_or_else(|| ctx.host.permission(id))))?;
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
    loop {
        let left = timeout.saturating_sub(started.elapsed());
        let (status, timed_out) = ctx.hub.wait(id, until, left.max(Duration::from_millis(1)), TICK, |s| {
            let what = s.current_tool.clone().unwrap_or_else(|| s.state().to_string());
            progress(format!("{what} ({}s)", started.elapsed().as_secs()));
        });
        // One turn ended but the agent has a queued message (this caller's, maybe) about to
        // start: that's not idle yet. A short pause lets its turn begin, then wait again.
        let settled = timed_out || until == Until::Change || status.busy || status.needs_attention() || !ctx.host.busy(id);
        if settled || started.elapsed() >= timeout {
            return (status, timed_out);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// What happened in the session's turn and what it waits for, with a hint of what to do next.
fn report(ctx: &Ctx, id: &str, s: &Status, timed_out: bool) -> Value {
    let state = if is_live(ctx, id) { s.state() } else { "idle" };
    let mut r = json!({"sessionId": id, "state": state});
    if timed_out {
        r["timedOut"] = json!(true);
    }
    // The agent's answer is its last message; what it said between tools is in the transcript.
    let reply = s.answer(MAX_ANSWER);
    if !reply.is_empty() {
        r["reply"] = json!(reply);
    }
    let earlier = s.texts.iter().filter(|t| !t.1.trim().is_empty()).count().saturating_sub(1);
    if earlier > 0 {
        r["earlierMessages"] = json!(earlier);
    }
    if !s.tools.is_empty() {
        r["toolsRun"] = json!(s.tools.len());
        r["lastTools"] = json!(s.tools[s.tools.len().saturating_sub(LAST_TOOLS)..]);
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
    // Only what isn't obvious from `state` (the server's instructions cover the rest).
    match state {
        "needsApproval" => r["next"] = json!("answer_approval with a requestId"),
        "needsQuestion" => r["next"] = json!("answer_question with the requestId and answers by question id"),
        _ => {}
    }
    r
}

/// `text` from byte `offset`, about `size` bytes ending at a line break, with where to go on.
fn page(text: &str, offset: usize, size: usize) -> String {
    let mut start = offset.min(text.len());
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let rest = &text[start..];
    if rest.len() <= size {
        return if start == 0 { rest.to_string() } else { format!("{rest}\n(end of diff)") };
    }
    let mut end = size;
    while !rest.is_char_boundary(end) {
        end -= 1;
    }
    let end = rest[..end].rfind('\n').map(|i| i + 1).filter(|&i| i > 0).unwrap_or(end);
    format!("{}\n(more: offset {} of {} bytes)", &rest[..end], start + end, text.len())
}

/// The end of `text`, at most `max` bytes from a line break, with `note` where the start was.
fn keep_end(text: &str, max: usize, note: &str) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let start = text[start..].find('\n').map(|i| start + i + 1).unwrap_or(start);
    format!("{note}\n\n{}", &text[start..])
}

/// Long tool output and file contents cut down, so a page of events stays readable.
fn shrink(mut v: Value) -> Value {
    fn walk(v: &mut Value, depth: usize) {
        match v {
            Value::String(s) if s.len() > 1000 => *s = cut(s, 1000),
            Value::Array(a) if depth < 6 => a.iter_mut().for_each(|x| walk(x, depth + 1)),
            Value::Object(o) if depth < 6 => o.values_mut().for_each(|x| walk(x, depth + 1)),
            _ => {}
        }
    }
    walk(&mut v, 0);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every client sends the tool list to its model on every turn.
    #[test]
    fn the_tool_list_stays_small() {
        let mut s = Settings::default();
        s.mcp.allow_delete = true;
        let seen: Vec<Value> =
            list(&s).iter().map(|t| json!({"name": t["name"], "description": t["description"], "inputSchema": t["inputSchema"]})).collect();
        let size = serde_json::to_string(&seen).unwrap().len();
        eprintln!("tools/list as the model sees it: {size} bytes");
        assert!(size < 9_500, "{size} bytes (it was 12.6 KB before it was trimmed)");
    }

    #[test]
    fn diffs_come_in_pages_that_end_at_a_line() {
        let diff: String = (0..2000).map(|i| format!("+line {i} é\n")).collect();
        let first = page(&diff, 0, 1000);
        let (body, more) = first.rsplit_once("\n(more: offset ").unwrap();
        assert!(body.ends_with('\n') && body.len() <= 1000);
        let next: usize = more.split(' ').next().unwrap().parse().unwrap();
        assert!(page(&diff, next, 1000).starts_with("+line "));
        assert!(page(&diff, diff.len() - 5, 1000).ends_with("(end of diff)"));
        assert_eq!(page("small\n", 0, 1000), "small\n");
    }

    #[test]
    fn long_transcripts_keep_their_end() {
        let md: String = (0..1000).map(|i| format!("line {i}\n")).collect();
        let kept = keep_end(&md, 200, "(cut)");
        assert!(kept.starts_with("(cut)\n\nline ") && kept.ends_with("line 999\n"), "{kept}");
        assert!(kept.len() < 220);
    }
}
