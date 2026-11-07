//! Claude Code over its stream-json protocol:
//!
//! `claude -p --input-format stream-json --output-format stream-json --verbose
//!  --include-partial-messages --permission-prompt-tool stdio --permission-mode <mode>`
//!
//! We write `user` messages and `control_request`s (initialize, interrupt, set_permission_mode)
//! to stdin. Claude writes `system`/`init`, `stream_event`s (text deltas), whole `assistant`
//! messages (text, thinking, tool_use), `user` messages carrying tool results, `control_request`
//! `can_use_tool` (answered with a `control_response`), and a `result` per turn.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin};
use std::sync::{Arc, Mutex};
use std::thread;

use oah_core::settings::{AgentKind, PermissionMode};
use oah_transport::Spec;
use serde_json::{json, Value};

use crate::driver::{Driver, DriverResult, Prompt, Sink, StartOptions};
use crate::event::*;

pub struct ClaudeDriver {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    child: Arc<Mutex<Option<Child>>>,
    state: Arc<Mutex<State>>,
    sink: Sink,
    turn: u32,
}

/// What the reader thread and the driver share.
#[derive(Default)]
struct State {
    /// Approval requests waiting for an answer: request id → (tool input, suggestions).
    pending: HashMap<String, (Value, Value)>,
    /// The turn being run, for `TurnFinished`.
    turn_id: Option<String>,
    /// `total_cost_usd` is per process; turns report the difference.
    cost_so_far: f64,
    /// Messages whose text arrived as stream deltas (their final text isn't sent again).
    streamed: HashSet<String>,
    /// The current streamed message id and its block kinds by index.
    message_id: String,
    blocks: HashMap<u64, &'static str>,
    alive: bool,
}

pub fn permission_mode_flag(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Ask => "default",
        PermissionMode::AcceptEdits => "acceptEdits",
        PermissionMode::Plan => "plan",
        PermissionMode::FullAccess => "bypassPermissions",
    }
}

impl ClaudeDriver {
    pub fn start(opts: StartOptions, sink: Sink) -> DriverResult<ClaudeDriver> {
        let mut args: Vec<String> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--permission-prompt-tool",
            "stdio",
            "--permission-mode",
            permission_mode_flag(opts.permission),
        ]
        .map(String::from)
        .to_vec();
        if opts.permission == PermissionMode::FullAccess {
            args.push("--allow-dangerously-skip-permissions".into());
        }
        if let Some(m) = &opts.model {
            args.extend(["--model".into(), m.clone()]);
        }
        if let Some(r) = &opts.resume {
            args.extend(["--resume".into(), r.clone()]);
        }
        let program = if opts.binary.trim().is_empty() { AgentKind::Claude.default_binary().to_string() } else { opts.binary.clone() };
        let mut cmd = oah_transport::command(&Spec { location: &opts.location, cwd: &opts.cwd, program: &program, args, env: &opts.env })?;
        let mut child = cmd.spawn().map_err(|e| format!("couldn't start Claude Code: {e}"))?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = child.stderr.take().ok_or("no stderr")?;
        let stdin = Arc::new(Mutex::new(child.stdin.take()));
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
            let (sink, state, child) = (sink.clone(), state.clone(), child.clone());
            thread::spawn(move || {
                let reader = BufReader::new(stdout);
                let mut tr = Translator::new(sink.clone(), state.clone());
                for line in reader.lines().map_while(Result::ok) {
                    if let Ok(v) = serde_json::from_str::<Value>(&line) {
                        tr.handle(&v);
                    }
                }
                state.lock().unwrap().alive = false;
                let code = child.lock().unwrap().as_mut().and_then(|c| c.wait().ok()).and_then(|s| s.code());
                std::thread::sleep(std::time::Duration::from_millis(50));
                let message = Some(tail.lock().unwrap().clone()).filter(|m| !m.is_empty());
                if let Some(turn_id) = state.lock().unwrap().turn_id.take() {
                    sink(ChatEvent::TurnFinished { turn_id, usage: None, cost_usd: None, duration_ms: None, interrupted: true });
                }
                sink(ChatEvent::Exited { code, message });
            });
        }

        let d = ClaudeDriver { stdin, child, state, sink, turn: 0 };
        d.write(&json!({"type": "control_request", "request_id": "oah-init", "request": {"subtype": "initialize", "hooks": null}}))?;
        Ok(d)
    }

    fn write(&self, v: &Value) -> DriverResult<()> {
        let mut g = self.stdin.lock().unwrap();
        let stdin = g.as_mut().ok_or("Claude Code isn't running")?;
        let mut line = serde_json::to_vec(v).map_err(|e| e.to_string())?;
        line.push(b'\n');
        stdin.write_all(&line).and_then(|_| stdin.flush()).map_err(|_| "Claude Code isn't running".to_string())
    }
}

impl Driver for ClaudeDriver {
    fn send(&mut self, prompt: Prompt) -> DriverResult<()> {
        if !self.state.lock().unwrap().alive {
            return Err("Claude Code isn't running".into());
        }
        self.turn += 1;
        let turn_id = format!("turn-{}", self.turn);
        let mut content = vec![json!({"type": "text", "text": prompt.text})];
        for img in &prompt.images {
            content.push(json!({"type": "image", "source": {"type": "base64", "media_type": img.mime, "data": img.data}}));
        }
        (self.sink)(ChatEvent::UserMessage { id: prompt.id.clone(), text: prompt.text.clone(), images: prompt.images.iter().map(|i| i.mime.clone()).collect() });
        let started = {
            let mut s = self.state.lock().unwrap();
            if s.turn_id.is_none() {
                s.turn_id = Some(turn_id.clone());
                true
            } else {
                false
            }
        };
        if started {
            (self.sink)(ChatEvent::TurnStarted { turn_id });
        }
        self.write(&json!({"type": "user", "message": {"role": "user", "content": content}}))
    }

    fn interrupt(&mut self) -> DriverResult<()> {
        self.write(&json!({"type": "control_request", "request_id": format!("oah-int-{}", self.turn), "request": {"subtype": "interrupt"}}))
    }

    fn answer(&mut self, request_id: &str, decision: Decision) -> DriverResult<()> {
        let (input, suggestions) = self.state.lock().unwrap().pending.remove(request_id).ok_or("that question was already answered")?;
        let response = match decision {
            Decision::Allow => json!({"behavior": "allow", "updatedInput": input}),
            Decision::AllowAlways => json!({"behavior": "allow", "updatedInput": input, "updatedPermissions": suggestions}),
            Decision::Deny => json!({"behavior": "deny", "message": "The user declined this. Don't try it again; ask what they want instead."}),
        };
        (self.sink)(ChatEvent::ApprovalResolved { request_id: request_id.into(), decision });
        self.write(&json!({"type": "control_response", "response": {"subtype": "success", "request_id": request_id, "response": response}}))
    }

    fn set_permission(&mut self, mode: PermissionMode) -> DriverResult<()> {
        self.write(&json!({"type": "control_request", "request_id": format!("oah-mode-{}", self.turn), "request": {"subtype": "set_permission_mode", "mode": permission_mode_flag(mode)}}))
    }

    fn stop(&mut self) {
        self.stdin.lock().unwrap().take();
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for ClaudeDriver {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---- Translation -----------------------------------------------------------------------------

struct Translator {
    sink: Sink,
    state: Arc<Mutex<State>>,
}

const MAX_OUTPUT: usize = 16 * 1024;

impl Translator {
    fn new(sink: Sink, state: Arc<Mutex<State>>) -> Translator {
        Translator { sink, state }
    }

    fn emit(&self, e: ChatEvent) {
        (self.sink)(e)
    }

    fn handle(&mut self, v: &Value) {
        match v["type"].as_str().unwrap_or("") {
            "system" if v["subtype"] == "init" => self.emit(ChatEvent::Started {
                session_id: v["session_id"].as_str().map(String::from),
                model: v["model"].as_str().map(String::from),
                cwd: v["cwd"].as_str().map(String::from),
            }),
            "stream_event" => self.stream_event(&v["event"]),
            "assistant" => self.assistant(v),
            "user" => self.tool_results(v),
            "control_request" => self.control_request(v),
            "result" => self.result(v),
            "rate_limit_event" => {
                let info = &v["rate_limit_info"];
                if info["status"].as_str().is_some_and(|s| s != "allowed" && s != "allowed_warning") {
                    self.emit(ChatEvent::Notice { level: NoticeLevel::Warn, text: "You've hit a Claude usage limit; requests will fail until it resets.".into() });
                }
            }
            _ => {}
        }
    }

    fn stream_event(&mut self, e: &Value) {
        let mut s = self.state.lock().unwrap();
        match e["type"].as_str().unwrap_or("") {
            "message_start" => {
                s.message_id = e["message"]["id"].as_str().unwrap_or("").to_string();
                s.blocks.clear();
            }
            "content_block_start" => {
                let kind = match e["content_block"]["type"].as_str() {
                    Some("text") => "text",
                    Some("thinking") => "thinking",
                    _ => "other",
                };
                s.blocks.insert(e["index"].as_u64().unwrap_or(0), kind);
            }
            "content_block_delta" => {
                let index = e["index"].as_u64().unwrap_or(0);
                let item_id = format!("{}:{index}", s.message_id);
                let d = &e["delta"];
                match d["type"].as_str() {
                    Some("text_delta") => {
                        let mid = s.message_id.clone();
                        s.streamed.insert(mid);
                        drop(s);
                        self.emit(ChatEvent::TextDelta { item_id, text: d["text"].as_str().unwrap_or("").into() });
                    }
                    Some("thinking_delta") => {
                        let t = d["thinking"].as_str().unwrap_or("");
                        if !t.is_empty() {
                            drop(s);
                            self.emit(ChatEvent::ReasoningDelta { item_id, text: t.into() });
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn assistant(&mut self, v: &Value) {
        let msg = &v["message"];
        let mid = msg["id"].as_str().unwrap_or("").to_string();
        let streamed = self.state.lock().unwrap().streamed.contains(&mid);
        for (i, block) in msg["content"].as_array().into_iter().flatten().enumerate() {
            match block["type"].as_str() {
                Some("text") if !streamed => {
                    self.emit(ChatEvent::Text { item_id: format!("{mid}:t{i}"), text: block["text"].as_str().unwrap_or("").into() });
                }
                Some("thinking") if !streamed => {
                    let t = block["thinking"].as_str().unwrap_or("");
                    if !t.is_empty() {
                        self.emit(ChatEvent::Reasoning { item_id: format!("{mid}:r{i}"), text: t.into() });
                    }
                }
                Some("tool_use") => {
                    let id = block["id"].as_str().unwrap_or("").to_string();
                    let name = block["name"].as_str().unwrap_or("");
                    let input = &block["input"];
                    if name == "TodoWrite" {
                        self.emit(ChatEvent::Plan { items: todos(input) });
                    }
                    self.emit(ChatEvent::ToolStarted { item_id: id, tool: tool_call(name, input) });
                }
                _ => {}
            }
        }
    }

    fn tool_results(&mut self, v: &Value) {
        let Some(content) = v["message"]["content"].as_array() else { return };
        for block in content.iter().filter(|b| b["type"] == "tool_result") {
            let id = block["tool_use_id"].as_str().unwrap_or("").to_string();
            let ok = !block["is_error"].as_bool().unwrap_or(false);
            let mut output = result_text(&block["content"]);
            if output.len() > MAX_OUTPUT {
                let mut cut = MAX_OUTPUT;
                while !output.is_char_boundary(cut) {
                    cut -= 1;
                }
                output.truncate(cut);
                output.push_str("\n…");
            }
            let diff = if ok { patch_of(&v["tool_use_result"]) } else { None };
            self.emit(ChatEvent::ToolFinished { item_id: id, ok, output, diff });
        }
    }

    fn control_request(&mut self, v: &Value) {
        let req = &v["request"];
        if req["subtype"] != "can_use_tool" {
            return;
        }
        let request_id = v["request_id"].as_str().unwrap_or("").to_string();
        let name = req["tool_name"].as_str().unwrap_or("");
        let input = req["input"].clone();
        let call = tool_call(name, &input);
        let (kind, title, detail, diff) = match call.kind {
            ToolKind::Bash => (ApprovalKind::Command, "Run a command".to_string(), input["command"].as_str().map(String::from), None),
            ToolKind::Edit | ToolKind::Write => (ApprovalKind::Edit, format!("{} {}", if call.kind == ToolKind::Write { "Write" } else { "Edit" }, call.title), None, proposed_diff(name, &input)),
            _ => (ApprovalKind::Other, format!("Use {name}"), Some(call.title.clone()).filter(|t| !t.is_empty()), None),
        };
        let detail = detail.or_else(|| req["decision_reason"].as_str().map(String::from));
        let suggestions = req["permission_suggestions"].clone();
        let can_remember = suggestions.as_array().is_some_and(|a| !a.is_empty());
        self.state.lock().unwrap().pending.insert(request_id.clone(), (input, suggestions));
        self.emit(ChatEvent::Approval {
            request: ApprovalRequest { request_id, item_id: req["tool_use_id"].as_str().map(String::from), kind, title, detail, diff, can_remember },
        });
    }

    fn result(&mut self, v: &Value) {
        let (turn_id, cost) = {
            let mut s = self.state.lock().unwrap();
            let total = v["total_cost_usd"].as_f64().unwrap_or(s.cost_so_far);
            let cost = (total - s.cost_so_far).max(0.0);
            s.cost_so_far = total;
            s.pending.clear();
            (s.turn_id.take(), cost)
        };
        let u = &v["usage"];
        let usage = Usage {
            input_tokens: u["input_tokens"].as_u64().unwrap_or(0),
            output_tokens: u["output_tokens"].as_u64().unwrap_or(0),
            cache_read_tokens: u["cache_read_input_tokens"].as_u64().unwrap_or(0),
            cache_write_tokens: u["cache_creation_input_tokens"].as_u64().unwrap_or(0),
        };
        if v["is_error"].as_bool() == Some(true) {
            let msg = v["result"].as_str().filter(|r| !r.is_empty()).map(String::from).unwrap_or_else(|| format!("Claude Code stopped: {}", v["subtype"].as_str().unwrap_or("error")));
            if v["subtype"] != "error_during_execution" {
                self.emit(ChatEvent::Error { message: msg });
            }
        }
        let interrupted = v["subtype"] == "error_during_execution" || v["terminal_reason"].as_str().is_some_and(|r| r.contains("interrupt"));
        self.emit(ChatEvent::TurnFinished {
            turn_id: turn_id.unwrap_or_else(|| "turn".into()),
            usage: Some(usage),
            cost_usd: Some(cost),
            duration_ms: v["duration_ms"].as_u64(),
            interrupted,
        });
        // Context fill: the last request's prompt size against the model's window.
        if let Some(last) = u["iterations"].as_array().and_then(|a| a.last()) {
            let used = ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"].iter().map(|k| last[*k].as_u64().unwrap_or(0)).sum::<u64>();
            let window = v["modelUsage"].as_object().and_then(|m| m.values().find_map(|x| x["contextWindow"].as_u64())).unwrap_or(200_000);
            if used > 0 {
                self.emit(ChatEvent::Context { used_tokens: used, window_tokens: window });
            }
        }
    }
}

/// Claude's tool names to oah's kinds, with a one-line title.
pub fn tool_call(name: &str, input: &Value) -> ToolCall {
    let s = |k: &str| input[k].as_str().unwrap_or("").to_string();
    let path = ["file_path", "notebook_path", "path"].iter().map(|k| s(k)).find(|p| !p.is_empty());
    let (kind, title) = match name {
        "Read" => (ToolKind::Read, s("file_path")),
        "Edit" | "MultiEdit" | "NotebookEdit" => (ToolKind::Edit, path.clone().unwrap_or_default()),
        "Write" => (ToolKind::Write, s("file_path")),
        "Bash" | "BashOutput" | "PowerShell" => (ToolKind::Bash, s("command")),
        "Grep" => (ToolKind::Search, s("pattern")),
        "Glob" => (ToolKind::Search, s("pattern")),
        "WebFetch" => (ToolKind::Web, s("url")),
        "WebSearch" => (ToolKind::Web, s("query")),
        "Task" | "Agent" => (ToolKind::Task, s("description")),
        "TodoWrite" => (ToolKind::Other, "Update plan".into()),
        _ => (ToolKind::Other, name.to_string()),
    };
    ToolCall { kind, name: name.into(), title: if title.is_empty() { name.into() } else { title }, input: input.clone(), path }
}

fn todos(input: &Value) -> Vec<PlanItem> {
    input["todos"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| PlanItem {
            text: t["content"].as_str().unwrap_or("").into(),
            status: match t["status"].as_str() {
                Some("completed") => PlanStatus::Completed,
                Some("in_progress") => PlanStatus::InProgress,
                _ => PlanStatus::Pending,
            },
        })
        .collect()
}

fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// The diff Claude reports after an edit (`tool_use_result.structuredPatch`).
fn patch_of(r: &Value) -> Option<FileDiff> {
    let path = r["filePath"].as_str()?;
    let hunks = r["structuredPatch"].as_array().filter(|h| !h.is_empty());
    match hunks {
        Some(hunks) => {
            let mut patch = String::new();
            for h in hunks {
                patch.push_str(&format!(
                    "@@ -{},{} +{},{} @@\n",
                    h["oldStart"].as_u64().unwrap_or(0),
                    h["oldLines"].as_u64().unwrap_or(0),
                    h["newStart"].as_u64().unwrap_or(0),
                    h["newLines"].as_u64().unwrap_or(0)
                ));
                for l in h["lines"].as_array().into_iter().flatten().filter_map(|l| l.as_str()) {
                    patch.push_str(l);
                    patch.push('\n');
                }
            }
            Some(FileDiff::from_patch(path, patch))
        }
        // A new file: everything is added.
        None if r["type"] == "create" => Some(FileDiff::replace(path, "", r["content"].as_str().unwrap_or(""))),
        None => None,
    }
}

/// What an edit would change, for the approval card.
fn proposed_diff(name: &str, input: &Value) -> Option<FileDiff> {
    let path = input["file_path"].as_str()?;
    match name {
        "Edit" => Some(FileDiff::replace(path, input["old_string"].as_str()?, input["new_string"].as_str()?)),
        "MultiEdit" => {
            let mut patch = String::new();
            for e in input["edits"].as_array()? {
                patch.push_str(&FileDiff::replace(path, e["old_string"].as_str()?, e["new_string"].as_str()?).patch);
            }
            Some(FileDiff::from_patch(path, patch))
        }
        "Write" => Some(FileDiff::replace(path, "", input["content"].as_str()?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(lines: &[Value]) -> Vec<ChatEvent> {
        let out = Arc::new(Mutex::new(Vec::new()));
        let o = out.clone();
        let sink: Sink = Arc::new(move |e| o.lock().unwrap().push(e));
        let state = Arc::new(Mutex::new(State { alive: true, turn_id: Some("turn-1".into()), ..Default::default() }));
        let mut tr = Translator::new(sink, state);
        for l in lines {
            tr.handle(l);
        }
        let v = out.lock().unwrap().clone();
        v
    }

    #[test]
    fn translates_a_turn() {
        let ev = run(&[
            json!({"type":"system","subtype":"init","session_id":"s1","model":"claude-haiku-5-5","cwd":"/x"}),
            json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"m1"}}}),
            json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}),
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}}),
            json!({"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"Hi"}]}}),
            json!({"type":"assistant","message":{"id":"m2","content":[{"type":"tool_use","id":"tu1","name":"Edit","input":{"file_path":"a.txt","old_string":"hello","new_string":"hello world"}}]}}),
            json!({"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Edit","input":{"file_path":"a.txt","old_string":"hello","new_string":"hello world"},"permission_suggestions":[{"type":"setMode","mode":"acceptEdits","destination":"session"}],"tool_use_id":"tu1"}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tu1","content":"ok"}]},"tool_use_result":{"filePath":"a.txt","structuredPatch":[{"oldStart":1,"oldLines":1,"newStart":1,"newLines":1,"lines":["-hello","+hello world"]}]}}),
            json!({"type":"result","subtype":"success","is_error":false,"duration_ms":6499,"total_cost_usd":0.0123,"usage":{"input_tokens":6,"output_tokens":506,"cache_read_input_tokens":100487,"cache_creation_input_tokens":54628,"iterations":[{"input_tokens":2,"cache_read_input_tokens":54008,"cache_creation_input_tokens":620}]},"modelUsage":{"claude-haiku-5-5":{"contextWindow":1000000}}}),
        ]);
        assert!(matches!(&ev[0], ChatEvent::Started { session_id: Some(s), .. } if s == "s1"));
        assert!(matches!(&ev[1], ChatEvent::TextDelta { item_id, text } if item_id == "m1:0" && text == "Hi"));
        assert!(!ev.iter().any(|e| matches!(e, ChatEvent::Text { .. })), "streamed text isn't sent twice");
        assert!(matches!(&ev[2], ChatEvent::ToolStarted { item_id, tool } if item_id == "tu1" && tool.kind == ToolKind::Edit && tool.title == "a.txt"));
        match &ev[3] {
            ChatEvent::Approval { request } => {
                assert_eq!(request.kind, ApprovalKind::Edit);
                assert!(request.can_remember);
                assert_eq!(request.diff.as_ref().unwrap().added, 1);
            }
            e => panic!("{e:?}"),
        }
        assert!(matches!(&ev[4], ChatEvent::ToolFinished { ok: true, diff: Some(d), .. } if d.added == 1 && d.removed == 1));
        match &ev[5] {
            ChatEvent::TurnFinished { turn_id, usage: Some(u), cost_usd: Some(c), .. } => {
                assert_eq!(turn_id, "turn-1");
                assert_eq!(u.output_tokens, 506);
                assert!((c - 0.0123).abs() < 1e-9);
            }
            e => panic!("{e:?}"),
        }
        assert!(matches!(&ev[6], ChatEvent::Context { used_tokens: 54630, window_tokens: 1_000_000 }));
    }

    #[test]
    fn maps_tools_and_todos() {
        assert_eq!(tool_call("Bash", &json!({"command":"ls -la"})).title, "ls -la");
        assert_eq!(tool_call("Grep", &json!({"pattern":"fn main"})).kind, ToolKind::Search);
        assert_eq!(tool_call("mcp__x__y", &json!({})).title, "mcp__x__y");
        let ev = run(&[json!({"type":"assistant","message":{"id":"m","content":[{"type":"tool_use","id":"t","name":"TodoWrite","input":{"todos":[{"content":"a","status":"completed"},{"content":"b","status":"in_progress"}]}}]}})]);
        assert!(matches!(&ev[0], ChatEvent::Plan { items } if items.len() == 2 && items[1].status == PlanStatus::InProgress));
    }
}
