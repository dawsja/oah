//! Codex over `codex app-server` (JSON-RPC 2.0, one JSON object per line).
//!
//! We send `initialize` (+ `initialized`), then `thread/start` or `thread/resume`, and
//! `turn/start` per message. Codex streams `item/started` / `item/completed` for messages,
//! reasoning, commands, file changes and tool calls, deltas for text and command output,
//! `turn/plan/updated`, `thread/tokenUsage/updated` and `turn/completed`. It asks before acting
//! with server requests (`item/commandExecution/requestApproval`,
//! `item/fileChange/requestApproval`), which we answer with `{decision}`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use oah_core::settings::{AgentKind, PermissionMode};
use oah_transport::Spec;
use serde_json::{json, Value};

use crate::driver::{Driver, DriverResult, Prompt, Sink, StartOptions};
use crate::event::*;
use crate::jsonrpc::Rpc;

pub struct CodexDriver {
    rpc: Rpc,
    child: Arc<Mutex<Option<Child>>>,
    state: Arc<Mutex<State>>,
    sink: Sink,
    permission: PermissionMode,
    model: Option<String>,
}

#[derive(Default)]
struct State {
    thread_id: Option<String>,
    turn_id: Option<String>,
    /// Approval requests waiting: our request id → (JSON-RPC id, kind).
    pending: HashMap<String, (Value, ApprovalKind)>,
    /// Questions waiting: our request id → the JSON-RPC id.
    questions: HashMap<String, Value>,
    /// File changes by item id, for the approval card's diff.
    changes: HashMap<String, Vec<FileDiff>>,
    usage: Option<Usage>,
    alive: bool,
}

/// Codex's approval policy and sandbox for each permission mode.
pub fn policy(mode: PermissionMode) -> (&'static str, &'static str) {
    match mode {
        PermissionMode::Ask => ("untrusted", "workspace-write"),
        PermissionMode::AcceptEdits => ("on-request", "workspace-write"),
        PermissionMode::Plan => ("on-request", "read-only"),
        PermissionMode::FullAccess => ("never", "danger-full-access"),
    }
}

impl CodexDriver {
    pub fn start(opts: StartOptions, sink: Sink) -> DriverResult<CodexDriver> {
        let program = if opts.binary.trim().is_empty() { AgentKind::Codex.default_binary().to_string() } else { opts.binary.clone() };
        let mut cmd = oah_transport::command(&Spec {
            location: &opts.location,
            cwd: &opts.cwd,
            program: &program,
            args: vec!["app-server".into()],
            env: &opts.env,
        })?;
        let mut child = cmd.spawn().map_err(|e| format!("couldn't start Codex: {e}"))?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = child.stderr.take().ok_or("no stderr")?;
        let rpc = Rpc::new(child.stdin.take(), "Codex");
        let child = Arc::new(Mutex::new(Some(child)));
        let state = Arc::new(Mutex::new(State { alive: true, ..Default::default() }));

        let tail = Arc::new(Mutex::new(String::new()));
        {
            let tail = tail.clone();
            thread::spawn(move || {
                let mut buf = String::new();
                let _ = BufReader::new(stderr).read_to_string(&mut buf);
                let keep: Vec<&str> = buf.lines().filter(|l| !l.trim().is_empty()).collect();
                *tail.lock().unwrap() = keep[keep.len().saturating_sub(6)..].join("\n");
            });
        }
        {
            let (sink, state, rpc, child) = (sink.clone(), state.clone(), rpc.clone(), child.clone());
            thread::spawn(move || {
                let mut tr = Translator { sink: sink.clone(), state: state.clone(), rpc: rpc.clone() };
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if let Ok(v) = serde_json::from_str::<Value>(&line) {
                        tr.handle(&v);
                    }
                }
                state.lock().unwrap().alive = false;
                rpc.fail_all();
                let code = child.lock().unwrap().as_mut().and_then(|c| c.wait().ok()).and_then(|s| s.code());
                thread::sleep(Duration::from_millis(50));
                if let Some(turn_id) = state.lock().unwrap().turn_id.take() {
                    sink(ChatEvent::TurnFinished { turn_id, usage: None, cost_usd: None, duration_ms: None, interrupted: true });
                }
                // 127: the shell couldn't find the program (on an SSH host, usually not installed).
                let message = if code == Some(127) {
                    Some("Codex isn't installed there, or isn't on that machine's PATH.".to_string())
                } else {
                    Some(tail.lock().unwrap().clone()).filter(|m| !m.is_empty())
                };
                sink(ChatEvent::Exited { code, message });
            });
        }

        let d = CodexDriver { rpc, child, state, sink, permission: opts.permission, model: opts.model.clone() };
        d.rpc.call(
            "initialize",
            json!({"clientInfo": {"name": "oah", "title": "oah", "version": env!("CARGO_PKG_VERSION")}, "capabilities": null}),
            Duration::from_secs(30),
        )?;
        d.rpc.notify("initialized", None)?;
        let (approval, sandbox) = policy(opts.permission);
        let mut params = json!({"cwd": opts.cwd, "approvalPolicy": approval, "sandbox": sandbox});
        if let Some(m) = &opts.model {
            params["model"] = json!(m);
        }
        let result = match &opts.resume {
            Some(id) => {
                params["threadId"] = json!(id);
                params["excludeTurns"] = json!(true);
                d.rpc.call("thread/resume", params, Duration::from_secs(60))
            }
            None => d.rpc.call("thread/start", params, Duration::from_secs(60)),
        }?;
        let thread = &result["thread"];
        let thread_id = thread["id"].as_str().ok_or("Codex didn't start a thread")?.to_string();
        d.state.lock().unwrap().thread_id = Some(thread_id.clone());
        let cwd = thread["environments"][0]["cwd"].as_str().map(String::from).or(Some(opts.cwd.clone()));
        let current = result["model"].as_str().map(String::from).or(opts.model.clone());
        (d.sink)(ChatEvent::Started { session_id: Some(thread_id), model: current.clone(), cwd });
        // The model picker and the skills for the / menu: Codex's own lists.
        let (rpc, sink, folder) = (d.rpc.clone(), d.sink.clone(), opts.cwd.clone());
        thread::spawn(move || {
            if let Ok(list) = rpc.call("model/list", json!({"limit": 100, "includeHidden": false}), Duration::from_secs(30)) {
                sink(ChatEvent::Models { models: parse_models(&list), current });
            }
            if let Ok(list) = rpc.call("skills/list", json!({"cwds": [folder]}), Duration::from_secs(30)) {
                sink(ChatEvent::Commands { commands: parse_skills(&list) });
            }
        });
        Ok(d)
    }

    fn thread_id(&self) -> DriverResult<String> {
        let s = self.state.lock().unwrap();
        if !s.alive {
            return Err("Codex isn't running".into());
        }
        s.thread_id.clone().ok_or_else(|| "Codex has no thread".into())
    }
}

impl Driver for CodexDriver {
    fn send(&mut self, prompt: Prompt) -> DriverResult<()> {
        let thread_id = self.thread_id()?;
        let mut input = vec![json!({"type": "text", "text": prompt.text, "text_elements": []})];
        for img in &prompt.images {
            input.push(json!({"type": "image", "url": format!("data:{};base64,{}", img.mime, img.data)}));
        }
        (self.sink)(ChatEvent::UserMessage {
            id: prompt.id,
            text: prompt.text,
            images: prompt.images.iter().map(|i| i.mime.clone()).collect(),
        });
        let (approval, _) = policy(self.permission);
        let mut params = json!({"threadId": thread_id, "input": input, "approvalPolicy": approval});
        if let Some(m) = &self.model {
            params["model"] = json!(m);
        }
        // A turn already running takes the message as steering input (turn/steer); if that's
        // refused, it starts a new turn.
        let running = self.state.lock().unwrap().turn_id.clone();
        let rpc = self.rpc.clone();
        let sink = self.sink.clone();
        thread::spawn(move || {
            if let Some(turn_id) = running {
                let steer = json!({"threadId": params["threadId"], "expectedTurnId": turn_id, "input": params["input"]});
                if rpc.call("turn/steer", steer, Duration::from_secs(60)).is_ok() {
                    return;
                }
            }
            if let Err(e) = rpc.call("turn/start", params, Duration::from_secs(120)) {
                sink(ChatEvent::Error { message: e });
            }
        });
        Ok(())
    }

    fn interrupt(&mut self) -> DriverResult<()> {
        let thread_id = self.thread_id()?;
        let Some(turn_id) = self.state.lock().unwrap().turn_id.clone() else { return Ok(()) };
        let rpc = self.rpc.clone();
        thread::spawn(move || {
            let _ = rpc.call("turn/interrupt", json!({"threadId": thread_id, "turnId": turn_id}), Duration::from_secs(30));
        });
        Ok(())
    }

    fn answer(&mut self, request_id: &str, decision: Decision) -> DriverResult<()> {
        let (rpc_id, _kind) = self.state.lock().unwrap().pending.remove(request_id).ok_or("that question was already answered")?;
        let d = match decision {
            Decision::Allow => "accept",
            Decision::AllowAlways => "acceptForSession",
            Decision::Deny => "decline",
        };
        (self.sink)(ChatEvent::ApprovalResolved { request_id: request_id.into(), decision });
        self.rpc.reply(&rpc_id, json!({"decision": d}))
    }

    fn answer_question(&mut self, request_id: &str, answers: std::collections::BTreeMap<String, Vec<String>>) -> DriverResult<()> {
        let rpc_id = self.state.lock().unwrap().questions.remove(request_id).ok_or("that question was already answered")?;
        let body: serde_json::Map<String, Value> = answers.iter().map(|(k, v)| (k.clone(), json!({"answers": v}))).collect();
        (self.sink)(ChatEvent::QuestionResolved { request_id: request_id.into(), answers });
        self.rpc.reply(&rpc_id, json!({"answers": body}))
    }

    fn set_model(&mut self, model: &str) -> DriverResult<()> {
        // Applied with the next turn/start.
        self.model = Some(model.to_string()).filter(|m| !m.is_empty());
        Ok(())
    }

    fn set_permission(&mut self, mode: PermissionMode) -> DriverResult<()> {
        // Applied with the next turn/start.
        self.permission = mode;
        Ok(())
    }

    fn compact(&mut self, instructions: &str) -> DriverResult<bool> {
        if !instructions.trim().is_empty() {
            return Err("Codex's /compact doesn't take instructions".into());
        }
        let thread_id = self.thread_id()?;
        let (rpc, sink) = (self.rpc.clone(), self.sink.clone());
        thread::spawn(move || {
            if let Err(e) = rpc.call("thread/compact/start", json!({"threadId": thread_id}), Duration::from_secs(60)) {
                sink(ChatEvent::Error { message: format!("Couldn't compact: {e}") });
            }
        });
        Ok(true)
    }

    fn stop(&mut self) {
        self.rpc.stdin.lock().unwrap().take();
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for CodexDriver {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---- Translation -----------------------------------------------------------------------------

struct Translator {
    sink: Sink,
    state: Arc<Mutex<State>>,
    rpc: Rpc,
}

const MAX_OUTPUT: usize = 16 * 1024;

impl Translator {
    fn emit(&self, e: ChatEvent) {
        (self.sink)(e)
    }

    fn handle(&mut self, v: &Value) {
        // A reply to one of our requests.
        if self.rpc.take_reply(v) || v.get("method").is_none() {
            return;
        }
        let method = v["method"].as_str().unwrap_or("");
        let p = &v["params"];
        // A request from Codex (it has an id): an approval, or something we can't answer.
        if let Some(id) = v.get("id") {
            match method {
                "item/commandExecution/requestApproval"
                | "item/fileChange/requestApproval"
                | "execCommandApproval"
                | "applyPatchApproval" => self.approval(id, method, p),
                "item/tool/requestUserInput" => self.question(id, p),
                _ => {
                    let _ = self.rpc.write(
                        &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "oah doesn't support this request"}}),
                    );
                }
            }
            return;
        }
        match method {
            "turn/started" => {
                let id = p["turn"]["id"].as_str().unwrap_or("").to_string();
                self.state.lock().unwrap().turn_id = Some(id.clone());
                self.emit(ChatEvent::TurnStarted { turn_id: id });
            }
            "item/started" => self.item(&p["item"], false),
            "item/completed" => self.item(&p["item"], true),
            "item/agentMessage/delta" => self.emit(ChatEvent::TextDelta { item_id: s(&p["itemId"]), text: s(&p["delta"]) }),
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                self.emit(ChatEvent::ReasoningDelta { item_id: s(&p["itemId"]), text: s(&p["delta"]) })
            }
            "turn/plan/updated" => self.emit(ChatEvent::Plan {
                items: p["plan"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|st| PlanItem {
                        text: s(&st["step"]),
                        status: match st["status"].as_str() {
                            Some("completed") => PlanStatus::Completed,
                            Some("inProgress") | Some("in_progress") => PlanStatus::InProgress,
                            _ => PlanStatus::Pending,
                        },
                    })
                    .collect(),
            }),
            "thread/tokenUsage/updated" => {
                let u = &p["tokenUsage"];
                let last = &u["last"];
                let input = last["inputTokens"].as_u64().unwrap_or(0);
                let cached = last["cachedInputTokens"].as_u64().unwrap_or(0);
                let usage = Usage {
                    input_tokens: input.saturating_sub(cached),
                    output_tokens: last["outputTokens"].as_u64().unwrap_or(0),
                    cache_read_tokens: cached,
                    cache_write_tokens: last["cacheWriteInputTokens"].as_u64().unwrap_or(0),
                };
                {
                    let mut st = self.state.lock().unwrap();
                    let acc = st.usage.get_or_insert_with(Usage::default);
                    acc.input_tokens += usage.input_tokens;
                    acc.output_tokens += usage.output_tokens;
                    acc.cache_read_tokens += usage.cache_read_tokens;
                    acc.cache_write_tokens += usage.cache_write_tokens;
                }
                if let Some(window) = u["modelContextWindow"].as_u64() {
                    self.emit(ChatEvent::Context { used_tokens: last["totalTokens"].as_u64().unwrap_or(input), window_tokens: window });
                }
            }
            "turn/completed" => {
                let turn = &p["turn"];
                let (turn_id, usage) = {
                    let mut st = self.state.lock().unwrap();
                    st.pending.clear();
                    st.questions.clear();
                    st.turn_id = None;
                    (s(&turn["id"]), st.usage.take())
                };
                let status = turn["status"].as_str().unwrap_or("completed");
                if status == "failed" {
                    let msg = turn["error"]["message"].as_str().unwrap_or("The turn failed.").to_string();
                    self.emit(ChatEvent::Error { message: msg });
                }
                self.emit(ChatEvent::TurnFinished {
                    turn_id,
                    usage,
                    cost_usd: None,
                    duration_ms: turn["durationMs"].as_u64(),
                    interrupted: status == "interrupted",
                });
            }
            "error" => {
                if p["willRetry"].as_bool() != Some(true) {
                    let msg = p["error"]["message"].as_str().or(p["message"].as_str()).unwrap_or("Codex reported an error").to_string();
                    self.emit(ChatEvent::Error { message: msg });
                }
            }
            "account/rateLimits/updated" if p["rateLimits"]["rateLimitReachedType"].is_string() => {
                self.emit(ChatEvent::Notice { level: NoticeLevel::Warn, text: "You've hit a Codex usage limit.".into() });
            }
            _ => {}
        }
    }

    fn item(&mut self, item: &Value, done: bool) {
        let id = s(&item["id"]);
        match item["type"].as_str().unwrap_or("") {
            "agentMessage" if done => self.emit(ChatEvent::Text { item_id: id, text: s(&item["text"]) }),
            "contextCompaction" if done => {
                self.emit(ChatEvent::Notice { level: NoticeLevel::Info, text: "Compacted the conversation so far.".into() })
            }
            "reasoning" if done => {
                let parts: Vec<String> = item["summary"]
                    .as_array()
                    .or(item["content"].as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect();
                if !parts.is_empty() {
                    self.emit(ChatEvent::Reasoning { item_id: id, text: parts.join("\n\n") });
                }
            }
            "commandExecution" => {
                let command = s(&item["command"]);
                if !done {
                    let kind = command_kind(item);
                    self.emit(ChatEvent::ToolStarted {
                        item_id: id,
                        tool: ToolCall {
                            kind,
                            name: "shell".into(),
                            title: strip_shell(&command),
                            input: json!({"command": command}),
                            path: None,
                        },
                    });
                } else {
                    let ok = item["status"] == "completed" && item["exitCode"].as_i64().unwrap_or(0) == 0;
                    self.emit(ChatEvent::ToolFinished { item_id: id, ok, output: clip(s(&item["aggregatedOutput"])), diff: None });
                }
            }
            "fileChange" => {
                let diffs: Vec<FileDiff> =
                    item["changes"].as_array().into_iter().flatten().map(|c| FileDiff::from_patch(s(&c["path"]), s(&c["diff"]))).collect();
                if !done {
                    let title = diffs.iter().map(|d| d.path.clone()).collect::<Vec<_>>().join(", ");
                    let path = diffs.first().map(|d| d.path.clone());
                    let kind = if item["changes"][0]["kind"]["type"] == "add" { ToolKind::Write } else { ToolKind::Edit };
                    self.state.lock().unwrap().changes.insert(id.clone(), diffs);
                    self.emit(ChatEvent::ToolStarted {
                        item_id: id,
                        tool: ToolCall { kind, name: "apply_patch".into(), title, input: item["changes"].clone(), path },
                    });
                } else {
                    let ok = item["status"] == "completed";
                    self.emit(ChatEvent::ToolFinished { item_id: id, ok, output: String::new(), diff: merge(diffs) });
                }
            }
            "mcpToolCall" | "dynamicToolCall" | "collabAgentToolCall" => {
                let name = item["tool"].as_str().or(item["name"].as_str()).unwrap_or("tool").to_string();
                if !done {
                    let title = format!("{}{}", item["server"].as_str().map(|s| format!("{s} · ")).unwrap_or_default(), name);
                    self.emit(ChatEvent::ToolStarted {
                        item_id: id,
                        tool: ToolCall { kind: ToolKind::Other, name, title, input: item["arguments"].clone(), path: None },
                    });
                } else {
                    let ok = item["status"] != "failed" && item["error"].is_null();
                    let output = if ok { clip(item["result"].to_string()) } else { s(&item["error"]["message"]) };
                    self.emit(ChatEvent::ToolFinished { item_id: id, ok, output, diff: None });
                }
            }
            "webSearch" => {
                if !done {
                    self.emit(ChatEvent::ToolStarted {
                        item_id: id,
                        tool: ToolCall {
                            kind: ToolKind::Web,
                            name: "web_search".into(),
                            title: s(&item["query"]),
                            input: item.clone(),
                            path: None,
                        },
                    });
                } else {
                    self.emit(ChatEvent::ToolFinished { item_id: id, ok: true, output: String::new(), diff: None });
                }
            }
            _ => {}
        }
    }

    fn question(&mut self, rpc_id: &Value, p: &Value) {
        let request_id = format!("cxq-{rpc_id}");
        let questions = p["questions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|q| Question {
                id: s(&q["id"]),
                header: s(&q["header"]),
                question: s(&q["question"]),
                options: q["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|o| QuestionOption { label: s(&o["label"]), description: s(&o["description"]) })
                    .collect(),
                multi: false,
                other: q["isOther"].as_bool().unwrap_or(false),
                secret: q["isSecret"].as_bool().unwrap_or(false),
            })
            .collect();
        self.state.lock().unwrap().questions.insert(request_id.clone(), rpc_id.clone());
        self.emit(ChatEvent::Question { request: QuestionRequest { request_id, questions } });
    }

    fn approval(&mut self, rpc_id: &Value, method: &str, p: &Value) {
        let item_id = p["itemId"].as_str().or(p["callId"].as_str()).map(String::from);
        let request_id = format!("cx-{}", rpc_id);
        let (kind, title, detail, diff) = if method.contains("ommand") {
            let cmd = p["command"]
                .as_str()
                .map(strip_shell)
                .or_else(|| p["command"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" ")));
            (ApprovalKind::Command, "Run a command".to_string(), cmd.or_else(|| p["reason"].as_str().map(String::from)), None)
        } else {
            let diffs = item_id.as_ref().and_then(|i| self.state.lock().unwrap().changes.get(i).cloned()).unwrap_or_default();
            let title = match diffs.as_slice() {
                [one] => format!("Edit {}", one.path),
                [] => "Edit files".into(),
                many => format!("Edit {} files", many.len()),
            };
            (ApprovalKind::Edit, title, p["reason"].as_str().map(String::from), merge(diffs))
        };
        self.state.lock().unwrap().pending.insert(request_id.clone(), (rpc_id.clone(), kind));
        self.emit(ChatEvent::Approval { request: ApprovalRequest { request_id, item_id, kind, title, detail, diff, can_remember: true } });
    }
}

/// `model/list` → the picker's options (hidden models left out).
/// `skills/list` → the / menu's skills: `{data: [{cwd, skills: [{name, description,
/// shortDescription, interface: {shortDescription}, enabled}]}]}`, turned-off ones left out.
pub fn parse_skills(v: &Value) -> Vec<SlashCommand> {
    let mut seen = std::collections::HashSet::new();
    v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|e| e["skills"].as_array().into_iter().flatten())
        .filter(|k| k["enabled"].as_bool() != Some(false))
        .filter_map(|k| {
            let name = k["name"].as_str()?.to_string();
            let description = [&k["interface"]["shortDescription"], &k["shortDescription"], &k["description"]]
                .into_iter()
                .find_map(|d| d.as_str().filter(|d| !d.trim().is_empty()))
                .unwrap_or("")
                .to_string();
            seen.insert(name.clone()).then_some(SlashCommand { name, description, hint: String::new(), skill: true })
        })
        .collect()
}

pub fn parse_models(v: &Value) -> Vec<ModelOption> {
    v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| !m["hidden"].as_bool().unwrap_or(false))
        .map(|m| ModelOption {
            id: m["model"].as_str().or(m["id"].as_str()).unwrap_or("").into(),
            name: m["displayName"].as_str().or(m["model"].as_str()).unwrap_or("").into(),
            description: m["description"].as_str().unwrap_or("").into(),
        })
        .collect()
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

fn clip(mut out: String) -> String {
    if out.len() > MAX_OUTPUT {
        let mut cut = MAX_OUTPUT;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out.push_str("\n…");
    }
    out
}

/// One diff for the card: a single file as is, several concatenated under the first's path.
fn merge(diffs: Vec<FileDiff>) -> Option<FileDiff> {
    match diffs.len() {
        0 => None,
        1 => diffs.into_iter().next(),
        _ => {
            let path = format!("{} (+{} more)", diffs[0].path, diffs.len() - 1);
            let patch = diffs.iter().map(|d| format!("@@ {} @@\n{}", d.path, d.patch)).collect::<String>();
            Some(FileDiff::from_patch(path, patch))
        }
    }
}

/// `"pwsh.exe" -Command 'Get-Content a.txt'` / `bash -lc 'ls'` → the inner command.
pub fn strip_shell(cmd: &str) -> String {
    for marker in [" -Command ", " -lc ", " -c "] {
        if let Some((_, rest)) = cmd.split_once(marker) {
            let rest = rest.trim();
            let unq = rest
                .strip_prefix('\'')
                .and_then(|r| r.strip_suffix('\''))
                .or_else(|| rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')))
                .unwrap_or(rest);
            return unq.replace("'\\''", "'");
        }
    }
    cmd.to_string()
}

/// A command that only reads or searches shows as such.
fn command_kind(item: &Value) -> ToolKind {
    let actions = item["commandActions"].as_array();
    match actions.map(|a| a.iter().map(|x| x["type"].as_str().unwrap_or("")).collect::<Vec<_>>()) {
        Some(types) if !types.is_empty() && types.iter().all(|t| *t == "read") => ToolKind::Read,
        Some(types) if !types.is_empty() && types.iter().all(|t| *t == "search" || *t == "listFiles") => ToolKind::Search,
        _ => ToolKind::Bash,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(lines: &[Value]) -> (Vec<ChatEvent>, Arc<Mutex<State>>) {
        let out = Arc::new(Mutex::new(Vec::new()));
        let o = out.clone();
        let sink: Sink = Arc::new(move |e| o.lock().unwrap().push(e));
        let state = Arc::new(Mutex::new(State { alive: true, ..Default::default() }));
        let rpc = Rpc::new(None, "Codex");
        let mut tr = Translator { sink, state: state.clone(), rpc };
        for l in lines {
            tr.handle(l);
        }
        let v = out.lock().unwrap().clone();
        (v, state)
    }

    #[test]
    fn translates_a_turn() {
        let (ev, state) = run(&[
            json!({"method":"turn/started","params":{"turn":{"id":"T1"}}}),
            json!({"method":"item/agentMessage/delta","params":{"itemId":"m1","delta":"I'll"}}),
            json!({"method":"item/completed","params":{"item":{"type":"agentMessage","id":"m1","text":"I'll edit it."}}}),
            json!({"method":"item/started","params":{"item":{"type":"commandExecution","id":"c1","command":"\"pwsh.exe\" -Command 'Get-Content -LiteralPath a.txt'","commandActions":[{"type":"read"}],"status":"inProgress"}}}),
            json!({"method":"item/completed","params":{"item":{"type":"commandExecution","id":"c1","command":"x","status":"completed","exitCode":0,"aggregatedOutput":"hello\r\n"}}}),
            json!({"method":"item/started","params":{"item":{"type":"fileChange","id":"f1","changes":[{"path":"a.txt","kind":{"type":"update"},"diff":"@@ -1 +1 @@\n-hello\n+hello world\n"}],"status":"inProgress"}}}),
            json!({"method":"item/fileChange/requestApproval","id":7,"params":{"itemId":"f1","threadId":"t","turnId":"T1"}}),
            json!({"method":"thread/tokenUsage/updated","params":{"tokenUsage":{"last":{"totalTokens":20184,"inputTokens":20130,"cachedInputTokens":12416,"cacheWriteInputTokens":0,"outputTokens":54},"modelContextWindow":258400}}}),
            json!({"method":"turn/completed","params":{"turn":{"id":"T1","status":"completed","durationMs":9000}}}),
        ]);
        assert!(matches!(&ev[0], ChatEvent::TurnStarted { turn_id } if turn_id == "T1"));
        assert!(matches!(&ev[1], ChatEvent::TextDelta { item_id, .. } if item_id == "m1"));
        assert!(matches!(&ev[2], ChatEvent::Text { text, .. } if text == "I'll edit it."));
        assert!(
            matches!(&ev[3], ChatEvent::ToolStarted { tool, .. } if tool.kind == ToolKind::Read && tool.title == "Get-Content -LiteralPath a.txt")
        );
        assert!(matches!(&ev[4], ChatEvent::ToolFinished { ok: true, .. }));
        assert!(matches!(&ev[5], ChatEvent::ToolStarted { tool, .. } if tool.kind == ToolKind::Edit && tool.title == "a.txt"));
        match &ev[6] {
            ChatEvent::Approval { request } => {
                assert_eq!(request.request_id, "cx-7");
                assert_eq!(request.title, "Edit a.txt");
                assert_eq!(request.diff.as_ref().unwrap().added, 1);
            }
            e => panic!("{e:?}"),
        }
        assert!(state.lock().unwrap().pending.is_empty(), "the turn ending drops unanswered questions");
        assert!(matches!(&ev[7], ChatEvent::Context { used_tokens: 20184, window_tokens: 258400 }));
        match &ev[8] {
            ChatEvent::TurnFinished { usage: Some(u), duration_ms: Some(9000), interrupted: false, .. } => {
                assert_eq!((u.input_tokens, u.cache_read_tokens, u.output_tokens), (7714, 12416, 54));
            }
            e => panic!("{e:?}"),
        }
    }

    #[test]
    fn asks_questions_and_lists_models() {
        let (ev, state) = run(&[
            json!({"method":"item/tool/requestUserInput","id":9,"params":{"questions":[{"id":"env","header":"Env","question":"Which environment?","isOther":true,"isSecret":false,"options":[{"label":"staging","description":""}]}]}}),
        ]);
        match &ev[0] {
            ChatEvent::Question { request } => {
                assert_eq!(request.request_id, "cxq-9");
                assert_eq!(request.questions[0].id, "env");
                assert!(request.questions[0].other);
            }
            e => panic!("{e:?}"),
        }
        assert!(state.lock().unwrap().questions.contains_key("cxq-9"));
        let m = parse_models(
            &json!({"data":[{"id":"a","model":"gpt-6","displayName":"GPT-6","description":"x","hidden":false},{"id":"b","model":"old","displayName":"Old","hidden":true}]}),
        );
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].id, "gpt-6");
    }

    #[test]
    fn strips_shell_wrappers() {
        assert_eq!(strip_shell("bash -lc 'ls -la'"), "ls -la");
        assert_eq!(strip_shell("\"C:\\pwsh.exe\" -Command 'Get-Content a.txt'"), "Get-Content a.txt");
        assert_eq!(strip_shell("git status"), "git status");
    }
}
