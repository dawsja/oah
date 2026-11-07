//! Agents over the Agent Client Protocol (ACP, v1): OpenCode (`opencode acp`) and any agent the
//! user adds. JSON-RPC 2.0 on stdio.
//!
//! We send `initialize`, then `session/new` (or `session/load` / `session/resume` to reopen a
//! chat), and one `session/prompt` at a time: messages sent while a prompt runs wait their turn.
//! The agent streams `session/update` notifications (message and thought chunks, tool calls and
//! their updates, plans, usage, config options) and asks with requests:
//! `session/request_permission` (answered with one of its options) and `elicitation/create`
//! (a form, shown as a question card). We advertise no file system or terminal, so `fs/*` and
//! `terminal/*` requests are refused.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use oah_core::settings::{AgentKind, EnvVar, PermissionMode};
use oah_transport::Spec;
use serde_json::{json, Value};

use crate::driver::{Driver, DriverResult, Prompt, Sink, StartOptions};
use crate::event::*;
use crate::jsonrpc::Rpc;

const PROTOCOL: u64 = 1;
/// A prompt can run for a long time; it ends when the agent says so (or is cancelled).
const PROMPT_LIMIT: Duration = Duration::from_secs(12 * 60 * 60);

pub struct AcpDriver {
    rpc: Rpc,
    child: Arc<Mutex<Option<Child>>>,
    state: Arc<Mutex<State>>,
    sink: Sink,
    queue: Arc<Mutex<VecDeque<Prompt>>>,
    permission: PermissionMode,
}

#[derive(Default)]
struct State {
    session_id: Option<String>,
    turn_id: Option<String>,
    turns: u32,
    /// A prompt is running.
    busy: bool,
    alive: bool,
    /// `session/load` replays history as updates; the chat already has it.
    loading: bool,
    /// Text and thought chunks without a message id are grouped per stretch between tool calls.
    stretch: u32,
    /// Tool calls so far, to merge partial updates.
    tools: HashMap<String, ToolCall>,
    /// Permission requests waiting: our id → (JSON-RPC id, the offered options).
    pending: HashMap<String, (Value, Vec<Value>)>,
    /// Form elicitations waiting: our id → (JSON-RPC id, the fields).
    questions: HashMap<String, (Value, Vec<Field>)>,
    /// The config option that picks the model (`session/set_config_option`), or none when the
    /// agent uses `session/set_model`.
    model_option: Option<String>,
    /// Mode switching: a config option id, or the session's modes.
    mode_option: Option<String>,
    modes: Vec<String>,
    /// Cumulative cost the agent reported; turns get the difference.
    cost: f64,
    cost_at_turn: f64,
    permission: PermissionMode,
}

/// A form field from an elicitation, and how its answer goes back.
#[derive(Clone, Debug)]
struct Field {
    key: String,
    kind: FieldKind,
    /// `<key>__other` / `<key>_custom`: where a typed answer goes.
    other_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum FieldKind {
    Bool,
    Choice,
    Multi,
    Text,
}

impl AcpDriver {
    pub fn start(opts: StartOptions, sink: Sink) -> DriverResult<AcpDriver> {
        let opencode = opts.kind == AgentKind::Opencode;
        let name = if opencode { "OpenCode".to_string() } else { "The agent".to_string() };
        let program = match opts.binary.trim() {
            "" if opencode => "opencode".to_string(),
            "" => return Err("This ACP agent has no command: set one in Settings → Providers.".into()),
            b => b.to_string(),
        };
        let mut args = opts.args.clone();
        let mut env = opts.env.clone();
        if opencode {
            args.insert(0, "acp".into());
            // OpenCode 1.x can't answer its question tool over ACP; 2.x uses form elicitation.
            env.push(EnvVar { key: "OPENCODE_CLIENT".into(), value: "acp".into(), in_keychain: false });
            env.push(EnvVar { key: "OPENCODE_ENABLE_QUESTION_TOOL".into(), value: "false".into(), in_keychain: false });
        }
        let mut cmd = oah_transport::command(&Spec { location: &opts.location, cwd: &opts.cwd, program: &program, args, env: &env })?;
        let mut child = cmd.spawn().map_err(|e| format!("couldn't start {program}: {e}"))?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = child.stderr.take().ok_or("no stderr")?;
        let rpc = Rpc::new(child.stdin.take(), name.clone());
        let child = Arc::new(Mutex::new(Some(child)));
        let state = Arc::new(Mutex::new(State { alive: true, permission: opts.permission, ..Default::default() }));

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
            let (sink, state, rpc, child, program) = (sink.clone(), state.clone(), rpc.clone(), child.clone(), program.clone());
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
                let message = if code == Some(127) {
                    Some(format!("{program} isn't installed there, or isn't on that machine's PATH."))
                } else {
                    Some(tail.lock().unwrap().clone()).filter(|m| !m.is_empty())
                };
                sink(ChatEvent::Exited { code, message });
            });
        }

        let d = AcpDriver { rpc, child, state, sink, queue: Arc::new(Mutex::new(VecDeque::new())), permission: opts.permission };
        let init = d.rpc.call(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL,
                "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false, "elicitation": {"form": {}}},
                "clientInfo": {"name": "oah", "title": "oah", "version": env!("CARGO_PKG_VERSION")}
            }),
            Duration::from_secs(60),
        )?;
        if init["protocolVersion"].as_u64().is_some_and(|v| v != PROTOCOL) {
            return Err(format!("{name} speaks ACP version {}, oah speaks {PROTOCOL}", init["protocolVersion"]));
        }
        let caps = &init["agentCapabilities"];
        let auth_hint = if opencode {
            "Sign in with `opencode auth login` (Settings → Providers → OpenCode → Sign in)."
        } else {
            "Sign in with the agent's own command first."
        };

        // Reopen the chat's session if we can; otherwise start a new one.
        let mut session = None;
        if let Some(id) = &opts.resume {
            let reopen = if caps["loadSession"].as_bool() == Some(true) {
                d.state.lock().unwrap().loading = true;
                let r = d.rpc.call("session/load", json!({"sessionId": id, "cwd": opts.cwd, "mcpServers": []}), Duration::from_secs(120));
                d.state.lock().unwrap().loading = false;
                r
            } else if !caps["sessionCapabilities"]["resume"].is_null() {
                d.rpc.call("session/resume", json!({"sessionId": id, "cwd": opts.cwd, "mcpServers": []}), Duration::from_secs(120))
            } else {
                Err("can't reopen sessions".into())
            };
            match reopen {
                Ok(mut r) => {
                    r["sessionId"] = json!(id);
                    session = Some(r);
                }
                Err(e) => (d.sink)(ChatEvent::Notice {
                    level: NoticeLevel::Warn,
                    text: format!("Couldn't reopen the earlier session ({e}); this continues in a new one."),
                }),
            }
        }
        let session = match session {
            Some(s) => s,
            None => d.rpc.call("session/new", json!({"cwd": opts.cwd, "mcpServers": []}), Duration::from_secs(120)).map_err(|e| {
                if e.contains("-32000") || e.to_lowercase().contains("auth") {
                    format!("{name} isn't signed in. {auth_hint}")
                } else {
                    e
                }
            })?,
        };
        let session_id = session["sessionId"].as_str().ok_or_else(|| format!("{name} didn't start a session"))?.to_string();
        d.state.lock().unwrap().session_id = Some(session_id.clone());

        let (models, current) = d.read_options(&session);
        (d.sink)(ChatEvent::Started { session_id: Some(session_id), model: current.clone(), cwd: Some(opts.cwd.clone()) });
        if !models.is_empty() {
            (d.sink)(ChatEvent::Models { models, current: current.clone() });
        }
        if let Some(m) = opts.model.as_deref().filter(|m| Some(*m) != current.as_deref()) {
            let mut me = d;
            if let Err(e) = me.set_model(m) {
                (me.sink)(ChatEvent::Notice { level: NoticeLevel::Warn, text: format!("Couldn't switch to {m}: {e}") });
            }
            let _ = me.apply_mode(opts.permission);
            return Ok(me);
        }
        let _ = d.apply_mode(opts.permission);
        Ok(d)
    }

    /// The model and mode pickers from `session/new`'s config options (or its unstable
    /// `models` / `modes`).
    fn read_options(&self, session: &Value) -> (Vec<ModelOption>, Option<String>) {
        let mut st = self.state.lock().unwrap();
        let (models, current, model_option) = models_from(session);
        st.model_option = model_option;
        if let Some(opts) = session["configOptions"].as_array() {
            if let Some(mode) = opts.iter().find(|o| o["category"] == "mode" || o["id"] == "mode") {
                st.mode_option = mode["id"].as_str().map(String::from);
                st.modes = select_values(&mode["options"]).into_iter().map(|m| m.id).collect();
            }
        }
        if st.modes.is_empty() {
            st.modes = session["modes"]["availableModes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| m["id"].as_str().map(String::from))
                .collect();
        }
        (models, current)
    }

    fn session(&self) -> DriverResult<String> {
        let s = self.state.lock().unwrap();
        if !s.alive {
            return Err(format!("{} isn't running", self.rpc.name));
        }
        s.session_id.clone().ok_or_else(|| format!("{} has no session", self.rpc.name))
    }

    /// Plan mode maps to the agent's `plan` mode when it has one (OpenCode: plan / build).
    fn apply_mode(&self, mode: PermissionMode) -> DriverResult<()> {
        let (option, modes) = {
            let s = self.state.lock().unwrap();
            (s.mode_option.clone(), s.modes.clone())
        };
        let want = if mode == PermissionMode::Plan { "plan" } else { "build" };
        if !modes.iter().any(|m| m == want) {
            return Ok(());
        }
        let sid = self.session()?;
        match option {
            Some(id) => self.rpc.call(
                "session/set_config_option",
                json!({"sessionId": sid, "configId": id, "value": want}),
                Duration::from_secs(30),
            ),
            None => self.rpc.call("session/set_mode", json!({"sessionId": sid, "modeId": want}), Duration::from_secs(30)),
        }
        .map(|_| ())
    }
}

/// Starts `prompt` now; when it ends, the next queued one starts.
fn run_prompt(rpc: Rpc, state: Arc<Mutex<State>>, sink: Sink, queue: Arc<Mutex<VecDeque<Prompt>>>, prompt: Prompt) {
    let (sid, turn_id) = {
        let mut s = state.lock().unwrap();
        s.busy = true;
        s.turns += 1;
        s.stretch += 1;
        s.cost_at_turn = s.cost;
        let id = format!("turn-{}", s.turns);
        s.turn_id = Some(id.clone());
        (s.session_id.clone().unwrap_or_default(), id)
    };
    sink(ChatEvent::TurnStarted { turn_id: turn_id.clone() });
    let mut blocks = vec![json!({"type": "text", "text": prompt.text})];
    for img in &prompt.images {
        blocks.push(json!({"type": "image", "mimeType": img.mime, "data": img.data}));
    }
    thread::spawn(move || {
        let started = std::time::Instant::now();
        let r = rpc.call("session/prompt", json!({"sessionId": sid, "prompt": blocks}), PROMPT_LIMIT);
        let (cost, interrupted) = {
            let mut s = state.lock().unwrap();
            s.busy = false;
            s.turn_id = None;
            s.pending.clear();
            s.questions.clear();
            let reason = r.as_ref().ok().and_then(|v| v["stopReason"].as_str()).unwrap_or("");
            ((s.cost - s.cost_at_turn).max(0.0), reason == "cancelled")
        };
        match &r {
            Ok(v) => {
                if let Some(reason) = v["stopReason"].as_str().filter(|r| matches!(*r, "max_tokens" | "max_turn_requests" | "refusal")) {
                    let text = match reason {
                        "max_tokens" => "The agent stopped: it hit its output limit.",
                        "max_turn_requests" => "The agent stopped: it hit its limit of steps for one turn.",
                        _ => "The agent declined to continue.",
                    };
                    sink(ChatEvent::Notice { level: NoticeLevel::Warn, text: text.into() });
                }
                sink(ChatEvent::TurnFinished {
                    turn_id,
                    usage: usage_of(&v["usage"]),
                    cost_usd: Some(cost).filter(|c| *c > 0.0),
                    duration_ms: Some(started.elapsed().as_millis() as u64),
                    interrupted,
                });
            }
            Err(e) => {
                if state.lock().unwrap().alive {
                    sink(ChatEvent::Error { message: e.clone() });
                }
                sink(ChatEvent::TurnFinished { turn_id, usage: None, cost_usd: None, duration_ms: None, interrupted: true });
            }
        }
        let next = queue.lock().unwrap().pop_front();
        if let Some(p) = next.filter(|_| state.lock().unwrap().alive) {
            run_prompt(rpc, state, sink, queue, p);
        }
    });
}

impl Driver for AcpDriver {
    fn send(&mut self, prompt: Prompt) -> DriverResult<()> {
        self.session()?;
        (self.sink)(ChatEvent::UserMessage {
            id: prompt.id.clone(),
            text: prompt.text.clone(),
            images: prompt.images.iter().map(|i| i.mime.clone()).collect(),
        });
        if self.state.lock().unwrap().busy {
            // One prompt at a time: this one runs when the current one ends.
            self.queue.lock().unwrap().push_back(prompt);
            return Ok(());
        }
        run_prompt(self.rpc.clone(), self.state.clone(), self.sink.clone(), self.queue.clone(), prompt);
        Ok(())
    }

    fn interrupt(&mut self) -> DriverResult<()> {
        let sid = self.session()?;
        // Questions in flight are withdrawn with the turn.
        let (pending, questions) = {
            let mut s = self.state.lock().unwrap();
            (s.pending.drain().collect::<Vec<_>>(), s.questions.drain().collect::<Vec<_>>())
        };
        for (_, (id, _)) in pending {
            let _ = self.rpc.reply(&id, json!({"outcome": {"outcome": "cancelled"}}));
        }
        for (_, (id, _)) in questions {
            let _ = self.rpc.reply(&id, json!({"action": "cancel"}));
        }
        self.queue.lock().unwrap().clear();
        self.rpc.notify("session/cancel", Some(json!({"sessionId": sid})))
    }

    fn answer(&mut self, request_id: &str, decision: Decision) -> DriverResult<()> {
        let (rpc_id, options) = self.state.lock().unwrap().pending.remove(request_id).ok_or("that question was already answered")?;
        (self.sink)(ChatEvent::ApprovalResolved { request_id: request_id.into(), decision });
        self.rpc.reply(&rpc_id, permission_outcome(&options, decision))
    }

    fn answer_question(&mut self, request_id: &str, answers: std::collections::BTreeMap<String, Vec<String>>) -> DriverResult<()> {
        let (rpc_id, fields) = self.state.lock().unwrap().questions.remove(request_id).ok_or("that question was already answered")?;
        let reply = elicitation_reply(&fields, &answers);
        (self.sink)(ChatEvent::QuestionResolved { request_id: request_id.into(), answers });
        self.rpc.reply(&rpc_id, reply)
    }

    fn set_model(&mut self, model: &str) -> DriverResult<()> {
        let sid = self.session()?;
        let option = self.state.lock().unwrap().model_option.clone();
        let r = match option {
            Some(id) => self.rpc.call(
                "session/set_config_option",
                json!({"sessionId": sid, "configId": id, "value": model}),
                Duration::from_secs(30),
            ),
            None => self.rpc.call("session/set_model", json!({"sessionId": sid, "modelId": model}), Duration::from_secs(30)),
        }?;
        // A config option change answers with the new options.
        if r["configOptions"].is_array() {
            let (models, current, _) = models_from(&r);
            if !models.is_empty() {
                (self.sink)(ChatEvent::Models { models, current });
            }
        }
        Ok(())
    }

    fn set_permission(&mut self, mode: PermissionMode) -> DriverResult<()> {
        self.permission = mode;
        self.state.lock().unwrap().permission = mode;
        self.apply_mode(mode)
    }

    fn stop(&mut self) {
        self.rpc.stdin.lock().unwrap().take();
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for AcpDriver {
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
        if self.rpc.take_reply(v) || v.get("method").is_none() {
            return;
        }
        let method = v["method"].as_str().unwrap_or("");
        let p = &v["params"];
        if let Some(id) = v.get("id") {
            match method {
                "session/request_permission" => self.permission(id, p),
                "elicitation/create" => self.elicitation(id, p),
                _ => {
                    let _ = self.rpc.reply_error(id, -32601, "oah doesn't offer this");
                }
            }
            return;
        }
        if method == "session/update" {
            if self.state.lock().unwrap().loading {
                return;
            }
            self.update(&p["update"]);
        }
    }

    fn update(&mut self, u: &Value) {
        match u["sessionUpdate"].as_str().unwrap_or("") {
            kind @ ("agent_message_chunk" | "agent_thought_chunk") => {
                let Some(text) = u["content"]["text"].as_str().filter(|t| !t.is_empty()) else { return };
                let thought = kind == "agent_thought_chunk";
                let item_id = match u["messageId"].as_str() {
                    Some(m) => format!("{m}{}", if thought { ":r" } else { "" }),
                    None => {
                        let s = self.state.lock().unwrap();
                        format!("{}-{}{}", s.turn_id.as_deref().unwrap_or("t"), if thought { "r" } else { "a" }, s.stretch)
                    }
                };
                self.emit(if thought {
                    ChatEvent::ReasoningDelta { item_id, text: text.into() }
                } else {
                    ChatEvent::TextDelta { item_id, text: text.into() }
                });
            }
            "tool_call" | "tool_call_update" => self.tool(u),
            "plan" => self.emit(ChatEvent::Plan {
                items: u["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|e| PlanItem {
                        text: e["content"].as_str().unwrap_or("").into(),
                        status: match e["status"].as_str() {
                            Some("completed") => PlanStatus::Completed,
                            Some("in_progress") => PlanStatus::InProgress,
                            _ => PlanStatus::Pending,
                        },
                    })
                    .collect(),
            }),
            "usage_update" => {
                if let Some(amount) = u["cost"]["amount"].as_f64() {
                    self.state.lock().unwrap().cost = amount;
                }
                if let (Some(used), Some(size)) = (u["used"].as_u64(), u["size"].as_u64()) {
                    self.emit(ChatEvent::Context { used_tokens: used, window_tokens: size });
                }
            }
            "config_option_update" => {
                let (models, current, option) = models_from(u);
                if !models.is_empty() {
                    self.state.lock().unwrap().model_option = option;
                    self.emit(ChatEvent::Models { models, current });
                }
            }
            "notice" => {
                if let Some(t) = u["message"].as_str().or(u["text"].as_str()) {
                    self.emit(ChatEvent::Notice { level: NoticeLevel::Info, text: t.into() });
                }
            }
            _ => {}
        }
    }

    fn tool(&mut self, u: &Value) {
        let Some(id) = u["toolCallId"].as_str().map(String::from) else { return };
        let mut st = self.state.lock().unwrap();
        // A text after this tool call is a new stretch.
        if u["sessionUpdate"] == "tool_call" {
            st.stretch += 1;
        }
        let known = st.tools.get(&id).cloned();
        let mut call = known.clone().unwrap_or(ToolCall {
            kind: ToolKind::Other,
            name: String::new(),
            title: String::new(),
            input: Value::Null,
            path: None,
        });
        if let Some(k) = u["kind"].as_str() {
            call.kind = tool_kind(k);
            call.name = k.to_string();
        }
        if let Some(t) = u["title"].as_str().filter(|t| !t.is_empty()) {
            call.title = t.to_string();
        }
        if !u["rawInput"].is_null() {
            call.input = u["rawInput"].clone();
            if call.title.is_empty() {
                call.title = input_title(&call.input);
            }
        }
        if let Some(p) = u["locations"][0]["path"].as_str() {
            call.path = Some(p.to_string());
        }
        if call.title.is_empty() {
            call.title = call.path.clone().unwrap_or_else(|| "tool".into());
        }
        st.tools.insert(id.clone(), call.clone());
        drop(st);
        if known.as_ref() != Some(&call) {
            self.emit(ChatEvent::ToolStarted { item_id: id.clone(), tool: call });
        }
        match u["status"].as_str() {
            Some("completed") | Some("failed") => {
                let ok = u["status"] == "completed";
                let diff = diffs_of(&u["content"]);
                self.emit(ChatEvent::ToolFinished { item_id: id, ok, output: clip(output_of(u)), diff });
            }
            _ => {}
        }
    }

    fn permission(&mut self, rpc_id: &Value, p: &Value) {
        let options: Vec<Value> = p["options"].as_array().cloned().unwrap_or_default();
        let auto = self.state.lock().unwrap().permission == PermissionMode::FullAccess;
        if auto {
            // Full access answers allow-once itself (never allow-always).
            let _ = self.rpc.reply(rpc_id, permission_outcome(&options, Decision::Allow));
            return;
        }
        let tc = &p["toolCall"];
        let id = tc["toolCallId"].as_str().map(String::from);
        let known = id.as_ref().and_then(|i| self.state.lock().unwrap().tools.get(i).cloned());
        let kind = tc["kind"].as_str().map(tool_kind).or(known.as_ref().map(|k| k.kind)).unwrap_or(ToolKind::Other);
        let title = tc["title"]
            .as_str()
            .filter(|t| !t.is_empty())
            .map(String::from)
            .or(known.as_ref().map(|k| k.title.clone()))
            .unwrap_or_else(|| "Use a tool".into());
        let input =
            if tc["rawInput"].is_null() { known.as_ref().map(|k| k.input.clone()).unwrap_or(Value::Null) } else { tc["rawInput"].clone() };
        let (akind, detail) = match kind {
            ToolKind::Bash => (ApprovalKind::Command, input["command"].as_str().map(String::from).or(Some(title.clone()))),
            ToolKind::Edit | ToolKind::Write => (ApprovalKind::Edit, None),
            _ => (ApprovalKind::Other, input["description"].as_str().map(String::from)),
        };
        let title = match akind {
            ApprovalKind::Command => "Run a command".to_string(),
            _ => title,
        };
        let can_remember = options.iter().any(|o| o["kind"] == "allow_always");
        let request_id = format!("acp-{rpc_id}");
        self.state.lock().unwrap().pending.insert(request_id.clone(), (rpc_id.clone(), options));
        self.emit(ChatEvent::Approval {
            request: ApprovalRequest { request_id, item_id: id, kind: akind, title, detail, diff: diffs_of(&tc["content"]), can_remember },
        });
    }

    fn elicitation(&mut self, rpc_id: &Value, p: &Value) {
        if p["mode"] == "url" {
            if let Some(url) = p["url"].as_str() {
                self.emit(ChatEvent::Notice {
                    level: NoticeLevel::Info,
                    text: format!("{} {url}", p["message"].as_str().unwrap_or("The agent asks you to open:")),
                });
            }
            let _ = self.rpc.reply(rpc_id, json!({"action": "cancel"}));
            return;
        }
        let (questions, fields) = form_questions(p);
        if questions.is_empty() {
            let _ = self.rpc.reply(rpc_id, json!({"action": "accept", "content": {}}));
            return;
        }
        let request_id = format!("acpq-{rpc_id}");
        self.state.lock().unwrap().questions.insert(request_id.clone(), (rpc_id.clone(), fields));
        self.emit(ChatEvent::Question { request: QuestionRequest { request_id, questions } });
    }
}

/// OpenCode's models for a chat that hasn't started: `opencode models` prints `provider/model`
/// per line.
pub fn opencode_models(binary: &str, location: &oah_core::settings::Location, cwd: &str) -> DriverResult<Vec<ModelOption>> {
    let program = if binary.trim().is_empty() { "opencode" } else { binary.trim() };
    let out = oah_transport::run(&Spec { location, cwd, program, args: vec!["models".into()], env: &[] }, None, Duration::from_secs(30))?;
    if !out.ok() {
        return Err(out.stderr.lines().last().unwrap_or("opencode models failed").to_string());
    }
    Ok(parse_opencode_models(&out.stdout))
}

pub fn parse_opencode_models(out: &str) -> Vec<ModelOption> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('{') && !l.starts_with('"') && l.contains('/') && !l.contains(' '))
        .map(|l| {
            let (provider, model) = l.split_once('/').unwrap_or(("", l));
            ModelOption { id: l.to_string(), name: model.to_string(), description: provider.to_string() }
        })
        .collect()
}

// ---- Helpers ---------------------------------------------------------------------------------

fn tool_kind(k: &str) -> ToolKind {
    match k {
        "read" => ToolKind::Read,
        "edit" | "delete" | "move" => ToolKind::Edit,
        "search" => ToolKind::Search,
        "execute" => ToolKind::Bash,
        "fetch" => ToolKind::Web,
        _ => ToolKind::Other,
    }
}

fn input_title(input: &Value) -> String {
    ["command", "filePath", "file_path", "path", "pattern", "query", "url", "description"]
        .iter()
        .find_map(|k| input[*k].as_str())
        .unwrap_or("")
        .to_string()
}

/// A tool's output: its text content, or what OpenCode puts in `rawOutput`.
fn output_of(u: &Value) -> String {
    let mut parts: Vec<String> = u["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["type"] == "content")
        .filter_map(|c| c["content"]["text"].as_str().map(String::from))
        .collect();
    if parts.is_empty() {
        let raw = &u["rawOutput"];
        if let Some(t) = raw["metadata"]["output"].as_str().or(raw["output"].as_str()).or(raw.as_str()) {
            parts.push(t.to_string());
        }
    }
    parts.join("\n")
}

fn diffs_of(content: &Value) -> Option<FileDiff> {
    let diffs: Vec<FileDiff> = content
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["type"] == "diff")
        .filter_map(|c| {
            Some(FileDiff::replace(c["path"].as_str()?, c["oldText"].as_str().unwrap_or(""), c["newText"].as_str().unwrap_or("")))
        })
        .collect();
    match diffs.len() {
        0 => None,
        1 => diffs.into_iter().next(),
        n => {
            let path = format!("{} (+{} more)", diffs[0].path, n - 1);
            Some(FileDiff::from_patch(path, diffs.iter().map(|d| format!("@@ {} @@\n{}", d.path, d.patch)).collect::<String>()))
        }
    }
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

fn usage_of(u: &Value) -> Option<Usage> {
    if u.is_null() {
        return None;
    }
    let n = |k: &str| u[k].as_u64().unwrap_or(0);
    Some(Usage {
        input_tokens: n("inputTokens"),
        output_tokens: n("outputTokens"),
        cache_read_tokens: n("cachedReadTokens"),
        cache_write_tokens: n("cachedWriteTokens"),
    })
}

/// The option to answer with: Allow → allow once, Always → allow always (else once), Deny →
/// reject once (else always). No matching option cancels.
fn permission_outcome(options: &[Value], decision: Decision) -> Value {
    let find = |kinds: &[&str]| {
        kinds.iter().find_map(|k| options.iter().find(|o| o["kind"] == *k)).and_then(|o| o["optionId"].as_str().map(String::from))
    };
    let id = match decision {
        Decision::Allow => find(&["allow_once", "allow_always"]),
        Decision::AllowAlways => find(&["allow_always", "allow_once"]),
        Decision::Deny => find(&["reject_once", "reject_always"]),
    };
    match id {
        Some(id) => json!({"outcome": {"outcome": "selected", "optionId": id}}),
        None => json!({"outcome": {"outcome": "cancelled"}}),
    }
}

/// A select config option's values (flat or grouped).
fn select_values(options: &Value) -> Vec<ModelOption> {
    let mut out = Vec::new();
    for o in options.as_array().into_iter().flatten() {
        if let Some(group) = o["options"].as_array() {
            out.extend(group.iter().filter_map(model_option));
        } else if let Some(m) = model_option(o) {
            out.push(m);
        }
    }
    out
}

fn model_option(o: &Value) -> Option<ModelOption> {
    let id = o["value"].as_str().or(o["modelId"].as_str())?.to_string();
    Some(ModelOption {
        name: o["name"].as_str().unwrap_or(&id).to_string(),
        description: o["description"].as_str().unwrap_or("").to_string(),
        id,
    })
}

/// The model choices in a session (config option with category `model`, or unstable `models`):
/// (options, current, the config option id when there is one).
fn models_from(v: &Value) -> (Vec<ModelOption>, Option<String>, Option<String>) {
    if let Some(opt) = v["configOptions"].as_array().and_then(|a| a.iter().find(|o| o["category"] == "model" || o["id"] == "model")) {
        return (select_values(&opt["options"]), opt["currentValue"].as_str().map(String::from), opt["id"].as_str().map(String::from));
    }
    let m = &v["models"];
    let list = m["availableModels"].as_array().into_iter().flatten().filter_map(model_option).collect();
    (list, m["currentModelId"].as_str().map(String::from), None)
}

/// An elicitation form → question cards, and how to read the answers back.
fn form_questions(p: &Value) -> (Vec<Question>, Vec<Field>) {
    let schema = &p["requestedSchema"];
    let Some(props) = schema["properties"].as_object() else { return (vec![], vec![]) };
    let message = p["message"].as_str().unwrap_or("").to_string();
    let others: Vec<&String> = props.keys().filter(|k| k.ends_with("__other") || k.ends_with("_custom")).collect();
    let mut questions = Vec::new();
    let mut fields = Vec::new();
    for (key, prop) in props.iter().filter(|(k, _)| !others.contains(k)) {
        let other_key =
            others.iter().find(|o| o.as_str() == format!("{key}__other") || o.as_str() == format!("{key}_custom")).map(|s| s.to_string());
        let default = prop["default"].as_str().map(String::from);
        let opts = |vals: Vec<(String, String)>| -> Vec<QuestionOption> {
            vals.into_iter()
                .map(|(v, t)| QuestionOption {
                    label: if Some(&v) == default.as_ref() { format!("{t} (Recommended)") } else { t },
                    description: String::new(),
                })
                .collect()
        };
        let choices = |p: &Value| -> Vec<(String, String)> {
            if let Some(e) = p["enum"].as_array() {
                return e.iter().filter_map(|x| x.as_str().map(|s| (s.to_string(), s.to_string()))).collect();
            }
            p["oneOf"]
                .as_array()
                .or(p["anyOf"].as_array())
                .into_iter()
                .flatten()
                .filter_map(|x| Some((x["const"].as_str()?.to_string(), x["title"].as_str().or(x["const"].as_str())?.to_string())))
                .collect()
        };
        let (kind, options) = match prop["type"].as_str() {
            Some("boolean") => (FieldKind::Bool, opts(vec![("yes".into(), "Yes".into()), ("no".into(), "No".into())])),
            Some("array") => (FieldKind::Multi, opts(choices(&prop["items"]))),
            _ => {
                let c = choices(prop);
                if c.is_empty() {
                    (FieldKind::Text, vec![])
                } else {
                    (FieldKind::Choice, opts(c))
                }
            }
        };
        let title = prop["title"].as_str().or(prop["description"].as_str()).unwrap_or(key).to_string();
        questions.push(Question {
            id: key.clone(),
            header: if questions.is_empty() && !message.is_empty() { String::new() } else { key.clone() },
            question: if questions.is_empty() && !message.is_empty() { format!("{message}\n{title}") } else { title },
            options,
            multi: kind == FieldKind::Multi,
            other: kind == FieldKind::Text || other_key.is_some(),
            secret: prop["format"] == "password",
        });
        fields.push(Field { key: key.clone(), kind, other_key });
    }
    (questions, fields)
}

/// The answers → `{action, content}`. Nothing answered cancels.
fn elicitation_reply(fields: &[Field], answers: &std::collections::BTreeMap<String, Vec<String>>) -> Value {
    if answers.values().all(|a| a.is_empty()) {
        return json!({"action": "cancel"});
    }
    let strip = |s: &str| s.trim_end_matches(" (Recommended)").to_string();
    let mut content = serde_json::Map::new();
    for f in fields {
        let Some(a) = answers.get(&f.key).filter(|a| !a.is_empty()) else { continue };
        match f.kind {
            FieldKind::Bool => {
                content.insert(f.key.clone(), json!(a[0] == "Yes"));
            }
            FieldKind::Multi => {
                content.insert(f.key.clone(), json!(a.iter().map(|s| strip(s)).collect::<Vec<_>>()));
            }
            FieldKind::Choice | FieldKind::Text => {
                content.insert(f.key.clone(), json!(strip(&a[0])));
                if let (Some(k), Some(extra)) = (&f.other_key, a.get(1)) {
                    content.insert(k.clone(), json!(extra));
                }
            }
        }
    }
    json!({"action": "accept", "content": content})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(lines: &[Value], permission: PermissionMode) -> (Vec<ChatEvent>, Arc<Mutex<State>>) {
        let out = Arc::new(Mutex::new(Vec::new()));
        let o = out.clone();
        let sink: Sink = Arc::new(move |e| o.lock().unwrap().push(e));
        let state =
            Arc::new(Mutex::new(State { alive: true, turn_id: Some("turn-1".into()), stretch: 1, permission, ..Default::default() }));
        let mut tr = Translator { sink, state: state.clone(), rpc: Rpc::new(None, "OpenCode") };
        for l in lines {
            tr.handle(l);
        }
        let v = out.lock().unwrap().clone();
        (v, state)
    }

    fn upd(u: Value) -> Value {
        json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s", "update": u}})
    }

    #[test]
    fn translates_updates() {
        let (ev, _) = run(
            &[
                upd(json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "hmm"}})),
                upd(json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Let me look."}})),
                upd(
                    json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Read src/cart.js", "kind": "read", "status": "pending", "locations": [{"path": "src/cart.js"}]}),
                ),
                upd(
                    json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "completed", "content": [{"type": "content", "content": {"type": "text", "text": "export function subtotal"}}]}),
                ),
                upd(
                    json!({"sessionUpdate": "tool_call", "toolCallId": "c2", "title": "Edit src/cart.js", "kind": "edit", "status": "in_progress"}),
                ),
                upd(
                    json!({"sessionUpdate": "tool_call_update", "toolCallId": "c2", "status": "completed", "content": [{"type": "diff", "path": "src/cart.js", "oldText": "i = 1", "newText": "i = 0"}]}),
                ),
                upd(json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Fixed."}})),
                upd(json!({"sessionUpdate": "plan", "entries": [{"content": "Fix loop", "priority": "high", "status": "completed"}]})),
                upd(json!({"sessionUpdate": "usage_update", "used": 12000, "size": 200000, "cost": {"amount": 0.02, "currency": "USD"}})),
            ],
            PermissionMode::Ask,
        );
        assert!(matches!(&ev[0], ChatEvent::ReasoningDelta { text, .. } if text == "hmm"));
        let a1 = match &ev[1] {
            ChatEvent::TextDelta { item_id, .. } => item_id.clone(),
            e => panic!("{e:?}"),
        };
        assert!(
            matches!(&ev[2], ChatEvent::ToolStarted { tool, .. } if tool.kind == ToolKind::Read && tool.path.as_deref() == Some("src/cart.js"))
        );
        assert!(matches!(&ev[3], ChatEvent::ToolFinished { ok: true, output, .. } if output.contains("subtotal")));
        assert!(matches!(&ev[5], ChatEvent::ToolFinished { diff: Some(d), .. } if d.added == 1 && d.removed == 1));
        match &ev[6] {
            ChatEvent::TextDelta { item_id, .. } => assert_ne!(item_id, &a1, "text after a tool is a new message"),
            e => panic!("{e:?}"),
        }
        assert!(matches!(&ev[7], ChatEvent::Plan { items } if items[0].status == PlanStatus::Completed));
        assert!(matches!(&ev[8], ChatEvent::Context { used_tokens: 12000, window_tokens: 200000 }));
    }

    #[test]
    fn asks_permission_and_answers_with_options() {
        let req = json!({"jsonrpc": "2.0", "id": 5, "method": "session/request_permission", "params": {"sessionId": "s",
            "toolCall": {"toolCallId": "c3", "title": "npm test", "kind": "execute", "rawInput": {"command": "npm test"}},
            "options": [{"optionId": "once", "name": "Allow", "kind": "allow_once"}, {"optionId": "always", "name": "Always", "kind": "allow_always"}, {"optionId": "reject", "name": "Reject", "kind": "reject_once"}]}});
        let (ev, state) = run(std::slice::from_ref(&req), PermissionMode::Ask);
        match &ev[0] {
            ChatEvent::Approval { request } => {
                assert_eq!(request.kind, ApprovalKind::Command);
                assert_eq!(request.detail.as_deref(), Some("npm test"));
                assert!(request.can_remember);
            }
            e => panic!("{e:?}"),
        }
        let (_, options) = state.lock().unwrap().pending.get("acp-5").cloned().unwrap();
        assert_eq!(permission_outcome(&options, Decision::AllowAlways)["outcome"]["optionId"], "always");
        assert_eq!(permission_outcome(&options, Decision::Deny)["outcome"]["optionId"], "reject");
        assert_eq!(permission_outcome(&[], Decision::Allow)["outcome"]["outcome"], "cancelled");
        let (ev, _) = run(&[req], PermissionMode::FullAccess);
        assert!(ev.is_empty(), "full access answers by itself");
    }

    #[test]
    fn elicitation_forms_become_questions() {
        let req = json!({"jsonrpc": "2.0", "id": 8, "method": "elicitation/create", "params": {"mode": "form", "message": "Pick a setup",
            "requestedSchema": {"type": "object", "properties": {
                "db": {"type": "string", "title": "Database", "enum": ["postgres", "sqlite"], "default": "sqlite"},
                "db__other": {"type": "string"},
                "extras": {"type": "array", "items": {"enum": ["auth", "jobs"]}},
                "tests": {"type": "boolean", "title": "Add tests?"}
            }, "required": ["db"]}}});
        let (ev, state) = run(&[req], PermissionMode::Ask);
        let q = match &ev[0] {
            ChatEvent::Question { request } => request.clone(),
            e => panic!("{e:?}"),
        };
        assert_eq!(q.questions.len(), 3);
        let db = q.questions.iter().find(|x| x.id == "db").unwrap();
        assert!(db.other);
        assert!(db.options.iter().any(|o| o.label == "sqlite (Recommended)"));
        assert!(q.questions.iter().find(|x| x.id == "extras").unwrap().multi);
        let (_, fields) = state.lock().unwrap().questions.get("acpq-8").cloned().unwrap();
        let mut a = std::collections::BTreeMap::new();
        a.insert("db".to_string(), vec!["sqlite (Recommended)".to_string()]);
        a.insert("extras".to_string(), vec!["auth".to_string(), "jobs".to_string()]);
        a.insert("tests".to_string(), vec!["Yes".to_string()]);
        let r = elicitation_reply(&fields, &a);
        assert_eq!(r["action"], "accept");
        assert_eq!(r["content"]["db"], "sqlite");
        assert_eq!(r["content"]["extras"], json!(["auth", "jobs"]));
        assert_eq!(r["content"]["tests"], true);
        assert_eq!(elicitation_reply(&fields, &Default::default())["action"], "cancel");
    }

    #[test]
    fn reads_models_from_config_options() {
        let s = json!({"sessionId": "s", "configOptions": [
            {"id": "model", "category": "model", "type": "select", "currentValue": "anthropic/claude-sonnet-5",
             "options": [{"group": "anthropic", "name": "Anthropic", "options": [{"value": "anthropic/claude-sonnet-5", "name": "Claude Sonnet 5"}]},
                         {"group": "openai", "name": "OpenAI", "options": [{"value": "openai/gpt-6", "name": "GPT-6"}]}]},
            {"id": "mode", "category": "mode", "type": "select", "currentValue": "build", "options": [{"value": "build", "name": "Build"}, {"value": "plan", "name": "Plan"}]}
        ]});
        let (models, current, option) = models_from(&s);
        assert_eq!(models.len(), 2);
        assert_eq!(current.as_deref(), Some("anthropic/claude-sonnet-5"));
        assert_eq!(option.as_deref(), Some("model"));
        let (m2, c2, o2) = models_from(&json!({"models": {"availableModels": [{"modelId": "x", "name": "X"}], "currentModelId": "x"}}));
        assert_eq!((m2.len(), c2.as_deref(), o2), (1, Some("x"), None));
    }

    #[test]
    fn lists_opencode_models() {
        let m = parse_opencode_models(
            "anthropic/claude-sonnet-5
openai/gpt-6
{
  \"variants\": {}
}
not a model
",
        );
        assert_eq!(m.len(), 2);
        assert_eq!((m[1].id.as_str(), m[1].name.as_str(), m[1].description.as_str()), ("openai/gpt-6", "gpt-6", "openai"));
    }

    #[test]
    fn refuses_file_system_requests() {
        let (ev, _) = run(
            &[json!({"jsonrpc": "2.0", "id": 3, "method": "fs/read_text_file", "params": {"path": "/etc/passwd"}})],
            PermissionMode::Ask,
        );
        assert!(ev.is_empty());
    }
}
