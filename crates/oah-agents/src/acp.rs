//! OpenCode, Antigravity and Grok Build over the Agent Client Protocol (ACP, v1): `opencode acp`,
//! `agy acp` or `grok agent stdio`, JSON-RPC 2.0 on stdio.
//!
//! We send `initialize`, then `session/new` (or `session/load` / `session/resume` to reopen a
//! chat), and one `session/prompt` at a time: messages sent while a prompt runs wait their turn.
//! The agent streams `session/update` notifications (message and thought chunks, tool calls and
//! their updates, plans, usage, config options) and asks with requests:
//! `session/request_permission` (answered with one of its options) and `elicitation/create`
//! (a form, shown as a question card); Grok also with `_x.ai/ask_user_question` (its question tool)
//! and `_x.ai/exit_plan_mode` (a finished plan, to approve). We advertise no file system or
//! terminal, so `fs/*` and `terminal/*` requests are refused.

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
    resumable: bool,
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
    /// Antigravity's questions, asked as permission requests whose options are the answers:
    /// our id → (JSON-RPC id, the options).
    choices: HashMap<String, (Value, Vec<Value>)>,
    /// Grok's questions (`_x.ai/ask_user_question`): our id → (JSON-RPC id, each question's text,
    /// which its answer goes back under, and whether it takes several).
    asks: HashMap<String, (Value, Vec<(String, bool)>)>,
    /// Grok's finished plans waiting for approval (`_x.ai/exit_plan_mode`): our id → JSON-RPC id.
    plans: HashMap<String, Value>,
    /// Tool calls shown by their card instead of a row (Grok's question and plan tools).
    hidden_tools: std::collections::HashSet<String>,
    /// The mode to go back to when the agent leaves plan mode by itself.
    after_plan: PermissionMode,
    /// Each model's context window, when the agent lists them (Grok).
    windows: HashMap<String, u64>,
    /// The config option that picks the model (`session/set_config_option`), or none when the
    /// agent uses `session/set_model`.
    model_option: Option<String>,
    /// Mode switching: a config option id, or the session's modes.
    mode_option: Option<String>,
    modes: Vec<String>,
    /// The reasoning effort option (category `thought_level`) and its values, for the current
    /// model.
    effort_option: Option<String>,
    efforts: Vec<String>,
    /// Cumulative cost the agent reported; turns get the difference. None until known: a
    /// reopened session's earlier cost comes with its replay, if at all.
    cost: Option<f64>,
    cost_at_turn: Option<f64>,
    /// The agent's major version (OpenCode 1.x and 2.x differ in places); 0 when unknown.
    major: u64,
    /// The agent takes images in prompts.
    images: bool,
    /// The running turn is `/compact`: its summary isn't shown, a note is.
    compacting: bool,
    /// The session can be deleted (`session/delete`), for the throwaway one a catalog uses.
    deletable: bool,
    current_model: Option<String>,
    /// Which CLI this is: modes, sign-in and model listings differ.
    kind: Option<AgentKind>,
    /// Each model's efforts as first listed (Antigravity's per-effort model ids), kept for the
    /// model lists that come later without them.
    known_efforts: HashMap<String, (Vec<String>, Option<String>)>,
    /// Set when the process starts; part of every turn id.
    nonce: String,
    permission: PermissionMode,
    /// Tool calls started this turn and not finished: one still open when the turn ends was
    /// left running (Antigravity's background commands), and shows as a background task.
    open: HashMap<String, ToolCall>,
    tasks: TaskList,
}

/// A form field from an elicitation, and how its answer goes back.
#[derive(Clone, Debug)]
struct Field {
    key: String,
    kind: FieldKind,
    /// `<key>__other` / `<key>_custom`: where a typed answer goes.
    other_key: Option<String>,
    /// The offered choices as (value sent back, label shown).
    choices: Vec<(String, String)>,
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
        let kind = opts.kind;
        let name = if kind.is_opencode() { "OpenCode".to_string() } else { kind.label().to_string() };
        let program = match opts.binary.trim() {
            "" => kind.default_binary().to_string(),
            b => b.to_string(),
        };
        let args = if kind == AgentKind::Grok { crate::grok::acp_args() } else { vec!["acp".to_string()] };
        let mut env = opts.env.clone();
        if kind.is_opencode() {
            // OpenCode 1.x can't answer its question tool over ACP; 2.x uses form elicitation.
            env.push(EnvVar { key: "OPENCODE_CLIENT".into(), value: "acp".into(), in_keychain: false });
            env.push(EnvVar { key: "OPENCODE_ENABLE_QUESTION_TOOL".into(), value: "false".into(), in_keychain: false });
            // OpenCode lets edits and commands through unless its config says "ask". Ask for both,
            // and oah answers what its mode allows (Accept edits, Full access) by itself, so the
            // mode can change mid-chat. 1.x reads OPENCODE_PERMISSION; 2.x ignores it and takes the
            // rule only as config (OPENCODE_CONFIG_CONTENT). Your own permission settings win.
            if !env.iter().any(|e| e.key == "OPENCODE_PERMISSION") {
                env.push(EnvVar { key: "OPENCODE_PERMISSION".into(), value: ASK_FOR_EDITS_AND_COMMANDS.into(), in_keychain: false });
                with_ask_config(&mut env);
            }
        }
        if kind == AgentKind::Antigravity {
            crate::antigravity::acp_env(&mut env);
        }
        if kind == AgentKind::Grok {
            crate::grok::acp_env(&mut env);
        }
        let mut cmd = oah_transport::command(&Spec { location: &opts.location, cwd: &opts.cwd, program: &program, args, env: &env })?;
        let mut child = cmd.spawn().map_err(|e| format!("couldn't start {program}: {e}"))?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = child.stderr.take().ok_or("no stderr")?;
        let rpc = Rpc::new(child.stdin.take(), name.clone());
        let child = Arc::new(Mutex::new(Some(child)));
        let nonce = format!(
            "{:x}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0) % 0xffffff
        );
        let after_plan = if opts.permission == PermissionMode::Plan { PermissionMode::Ask } else { opts.permission };
        let state = Arc::new(Mutex::new(State {
            alive: true,
            permission: opts.permission,
            after_plan,
            nonce,
            kind: Some(kind),
            ..Default::default()
        }));

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
                let (turn, tasks) = {
                    let mut s = state.lock().unwrap();
                    s.open.clear();
                    let tasks = s.tasks.stop_all().then(|| s.tasks.event());
                    (s.turn_id.take(), tasks)
                };
                if let Some(turn_id) = turn {
                    sink(ChatEvent::TurnFinished { turn_id, usage: None, cost_usd: None, duration_ms: None, interrupted: true });
                }
                // What it left running went with it.
                if let Some(e) = tasks {
                    sink(e);
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
        if let Some(msg) = init["agentInfo"]["version"].as_str().and_then(|v| crate::probe::outdated(opts.kind, v)) {
            return Err(msg);
        }
        let caps = &init["agentCapabilities"];
        {
            let mut st = d.state.lock().unwrap();
            st.resumable = caps["loadSession"].as_bool() == Some(true) || !caps["sessionCapabilities"]["resume"].is_null();
            st.major = major_version(init["agentInfo"]["version"].as_str().unwrap_or(""));
            st.images = caps["promptCapabilities"]["image"].as_bool() == Some(true);
            st.deletable = !caps["sessionCapabilities"]["delete"].is_null();
        }
        // Grok's sessions ask their questions through `_x.ai/ask_user_question`.
        let session_params = |id: Option<&str>| {
            let mut p = json!({"cwd": opts.cwd, "mcpServers": []});
            if let Some(id) = id {
                p["sessionId"] = json!(id);
            }
            if kind == AgentKind::Grok {
                p["_meta"] = crate::grok::session_meta();
            }
            p
        };
        // Reopen the chat's session if we can; otherwise start a new one.
        let mut session = None;
        if let Some(id) = &opts.resume {
            // Antigravity and Grok resume without replaying the conversation; OpenCode's replay
            // carries the session's cost so far.
            let resume_first = matches!(kind, AgentKind::Antigravity | AgentKind::Grok) && !caps["sessionCapabilities"]["resume"].is_null();
            let reopen = if resume_first {
                d.rpc.call("session/resume", session_params(Some(id)), Duration::from_secs(120))
            } else if caps["loadSession"].as_bool() == Some(true) {
                d.state.lock().unwrap().loading = true;
                let r = d.rpc.call("session/load", session_params(Some(id)), Duration::from_secs(120));
                d.state.lock().unwrap().loading = false;
                r
            } else if !caps["sessionCapabilities"]["resume"].is_null() {
                d.rpc.call("session/resume", session_params(Some(id)), Duration::from_secs(120))
            } else {
                Err("can't reopen sessions".into())
            };
            match reopen {
                Ok(mut r) => {
                    r["sessionId"] = json!(id);
                    session = Some(r);
                }
                // No answer, or the agent went away: the session may well still be there, so
                // the chat keeps it (and fails to start) instead of carrying on in a new one
                // that remembers nothing.
                Err(e) if e.contains("didn't answer") || e.ends_with(" exited") => {
                    return Err(format!("Couldn't reopen this chat's session ({e}). Try again."));
                }
                Err(e) => (d.sink)(ChatEvent::Notice {
                    level: NoticeLevel::Warn,
                    text: format!("Couldn't reopen the earlier session ({e}); this continues in a new one."),
                }),
            }
        }
        let new_session = || d.rpc.call("session/new", session_params(None), Duration::from_secs(120));
        let session = match session {
            Some(s) => s,
            // OpenCode's only sign-in is `opencode auth login` in a terminal: `authenticate`
            // can't do it, so a refusal goes straight to saying how.
            None => match new_session() {
                Ok(s) => {
                    // A new session has cost nothing yet.
                    d.state.lock().unwrap().cost = Some(0.0);
                    s
                }
                Err(e) if is_auth_error(&e) => return Err(not_signed_in(kind).to_string()),
                Err(e) => return Err(e),
            },
        };
        let session_id = session["sessionId"].as_str().ok_or_else(|| format!("{name} didn't start a session"))?.to_string();
        d.state.lock().unwrap().session_id = Some(session_id.clone());

        let (models, current) = d.read_options(&session);
        (d.sink)(ChatEvent::Started { session_id: Some(session_id), model: current.clone(), cwd: Some(opts.cwd.clone()) });
        if !models.is_empty() {
            (d.sink)(ChatEvent::Models { models, current: current.clone() });
        }
        if kind.is_opencode() && d.state.lock().unwrap().major < 2 {
            // 1.x sessions list only the current model's efforts; its CLI knows every model's.
            // (2.x's CLI listing leaves out providers that report late: the session's is the one.)
            let (sink, state, binary, location, cwd, env) =
                (d.sink.clone(), d.state.clone(), program.clone(), opts.location.clone(), opts.cwd.clone(), opts.env.clone());
            thread::spawn(move || {
                if let Ok(models) = opencode_models(&binary, &location, &cwd, &env) {
                    let current = state.lock().unwrap().current_model.clone();
                    if !models.is_empty() {
                        sink(ChatEvent::Models { models, current });
                    }
                }
            });
        }
        d.state.lock().unwrap().current_model = current.clone();
        if let Some(m) = opts.model.as_deref().filter(|m| Some(*m) != current.as_deref()) {
            let mut me = d;
            if let Err(e) = me.set_model(m) {
                (me.sink)(ChatEvent::Notice { level: NoticeLevel::Warn, text: format!("Couldn't switch to {m}: {e}") });
            }
            let _ = me.apply_mode(opts.permission);
            if let Some(e) = opts.effort.as_deref().filter(|e| !e.is_empty()) {
                let _ = me.set_effort(e);
            }
            return Ok(me);
        }
        let _ = d.apply_mode(opts.permission);
        let mut d = d;
        if let Some(e) = opts.effort.as_deref().filter(|e| !e.is_empty()) {
            let _ = d.set_effort(e);
        }
        Ok(d)
    }

    /// The model and mode pickers from `session/new`'s config options (or its unstable
    /// `models` / `modes`).
    fn read_options(&self, session: &Value) -> (Vec<ModelOption>, Option<String>) {
        let mut st = self.state.lock().unwrap();
        let (mut models, current, model_option) = models_from(session);
        st.model_option = model_option;
        read_effort(&mut st, session);
        add_efforts(&mut models, current.as_deref(), &st.efforts);
        variant_efforts(&mut models, &session["models"]);
        listed_efforts(&mut models, &session["models"]);
        st.windows.extend(crate::grok::context_windows(&session["models"]));
        st.known_efforts = models
            .iter()
            .filter(|m| !m.efforts.is_empty())
            .map(|m| (m.id.clone(), (m.efforts.clone(), m.default_effort.clone())))
            .collect();
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
        // Grok lists no modes, but takes `session/set_mode` with these.
        if st.modes.is_empty() && st.kind == Some(AgentKind::Grok) {
            st.modes = vec!["default".into(), "plan".into()];
        }
        (models, current)
    }

    /// A config option change answers with every option as it is now: the model, its efforts.
    fn adopt_options(&self, r: &Value) {
        if !r["configOptions"].is_array() {
            return;
        }
        let mut st = self.state.lock().unwrap();
        read_effort(&mut st, r);
        let (mut models, current, _) = models_from(r);
        if current.is_some() {
            st.current_model = current.clone();
        }
        // Efforts as first listed (in order) before the option's.
        known_efforts(&mut models, &st.known_efforts);
        add_efforts(&mut models, current.as_deref(), &st.efforts);
        drop(st);
        if !models.is_empty() {
            (self.sink)(ChatEvent::Models { models, current });
        }
    }

    fn session(&self) -> DriverResult<String> {
        let s = self.state.lock().unwrap();
        if !s.alive {
            return Err(format!("{} isn't running", self.rpc.name));
        }
        s.session_id.clone().ok_or_else(|| format!("{} has no session", self.rpc.name))
    }

    /// oah's permission mode as the agent's own mode, when it has a matching one: OpenCode's
    /// plan / build, Antigravity's plan / accept-edits / default, Grok's plan / default.
    fn apply_mode(&self, mode: PermissionMode) -> DriverResult<()> {
        let (option, modes, kind) = {
            let s = self.state.lock().unwrap();
            (s.mode_option.clone(), s.modes.clone(), s.kind)
        };
        let want = agent_mode(kind, mode);
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
    let (sid, turn_id, images) = {
        let mut s = state.lock().unwrap();
        s.busy = true;
        s.turns += 1;
        s.stretch += 1;
        s.cost_at_turn = s.cost;
        // Unique per process: a restarted agent counts from 1 again, and ids made from this one
        // (text without a message id) must not land in an earlier message.
        let id = format!("turn-{}-{}", s.nonce, s.turns);
        s.turn_id = Some(id.clone());
        (s.session_id.clone().unwrap_or_default(), id, s.images)
    };
    sink(ChatEvent::TurnStarted { turn_id: turn_id.clone() });
    let mut blocks = vec![json!({"type": "text", "text": prompt.text})];
    if images {
        for img in &prompt.images {
            blocks.push(json!({"type": "image", "mimeType": img.mime, "data": img.data}));
        }
    } else if !prompt.images.is_empty() {
        sink(ChatEvent::Notice { level: NoticeLevel::Warn, text: "This agent doesn't take images: they weren't sent.".into() });
    }
    thread::spawn(move || {
        let started = std::time::Instant::now();
        let r = rpc.call("session/prompt", json!({"sessionId": sid, "prompt": blocks}), PROMPT_LIMIT);
        let (cost, interrupted, compacted, context, tasks) = {
            let mut s = state.lock().unwrap();
            s.busy = false;
            s.turn_id = None;
            // Tool calls the turn ended without: left running in the background. Antigravity
            // never says when those end; others may, with a later `tool_call_update`.
            let left: Vec<(String, ToolCall)> = s.open.drain().collect();
            let ended_well = r.as_ref().is_ok_and(|v| v["stopReason"] != "cancelled");
            let tracked = s.kind != Some(AgentKind::Antigravity);
            let mut changed = false;
            for (id, call) in left.into_iter().filter(|_| ended_well) {
                let kind = match call.kind {
                    ToolKind::Bash => TaskKind::Command,
                    ToolKind::Task => TaskKind::Agent,
                    _ => TaskKind::Other,
                };
                let mut t = BackgroundTask::running(id.clone(), kind, call.title);
                t.item_id = Some(id);
                t.tracked = tracked;
                changed |= s.tasks.add(t);
            }
            let tasks = changed.then(|| s.tasks.event());
            s.pending.clear();
            s.questions.clear();
            s.choices.clear();
            s.asks.clear();
            s.plans.clear();
            let reason = r.as_ref().ok().and_then(|v| v["stopReason"].as_str()).unwrap_or("");
            // Without a cost from before the turn, its share can't be told: none, not all of it.
            let cost = match (s.cost, s.cost_at_turn) {
                (Some(now), Some(before)) => Some((now - before).max(0.0)),
                _ => None,
            };
            // Grok says the turn's own cost (ticks, 10^10 per dollar) and the context it ends with.
            let meta = r.as_ref().ok().map(|v| &v["_meta"]);
            let cost = meta.and_then(|m| m["usage"]["costUsdTicks"].as_f64()).map(|t| t / 1e10).or(cost);
            let context = meta
                .and_then(|m| m["totalTokens"].as_u64())
                .filter(|t| *t > 0)
                .and_then(|used| Some((used, *s.windows.get(s.current_model.as_deref()?)?)));
            (cost, reason == "cancelled", std::mem::take(&mut s.compacting), context, tasks)
        };
        // Before the turn's end, so that doesn't settle these tool calls as cut off.
        if let Some(e) = tasks {
            sink(e);
        }
        if let Some((used_tokens, window_tokens)) = context {
            sink(ChatEvent::Context { used_tokens, window_tokens, parts: vec![] });
        }
        if compacted && r.is_ok() && !interrupted {
            sink(ChatEvent::Notice { level: NoticeLevel::Info, text: "Context compacted.".into() });
        }
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
                    usage: usage_of(&v["usage"]).or_else(|| usage_of(&v["_meta"]["usage"])),
                    cost_usd: cost.filter(|c| *c > 0.0),
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
            files: prompt.files.clone(),
        });
        // Ended tasks have been seen; so have ones the agent never reports on (it can say how
        // they went now, if asked).
        let tasks = {
            let mut s = self.state.lock().unwrap();
            let untracked = s.tasks.forget_untracked();
            (s.tasks.clear_ended() | untracked).then(|| s.tasks.event())
        };
        if let Some(e) = tasks {
            (self.sink)(e);
        }
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
        let (pending, questions, asks, plans) = {
            let mut s = self.state.lock().unwrap();
            let mut pending = s.pending.drain().collect::<Vec<_>>();
            pending.extend(s.choices.drain());
            let asks = s.asks.drain().map(|(_, (id, _))| id).collect::<Vec<_>>();
            (pending, s.questions.drain().collect::<Vec<_>>(), asks, s.plans.drain().map(|(_, id)| id).collect::<Vec<_>>())
        };
        for (_, (id, _)) in pending {
            let _ = self.rpc.reply(&id, json!({"outcome": {"outcome": "cancelled"}}));
        }
        for (_, (id, _)) in questions {
            let _ = self.rpc.reply(&id, json!({"action": "cancel"}));
        }
        for id in asks {
            let _ = self.rpc.reply(&id, json!({"outcome": "cancelled"}));
        }
        // The plan stays a plan: Grok keeps planning until the turn stops.
        for id in plans {
            let _ = self.rpc.reply(&id, json!({"outcome": "revise"}));
        }
        // Messages already sent wait their turn: they're in the transcript as sent, and the
        // window steers by stopping the turn and then sending, so this one must still go.
        self.rpc.notify("session/cancel", Some(json!({"sessionId": sid})))
    }

    fn answer(&mut self, request_id: &str, decision: Decision) -> DriverResult<()> {
        // A finished plan (Grok): approved, Grok leaves plan mode itself and says so; denied, it
        // keeps planning.
        let plan = self.state.lock().unwrap().plans.remove(request_id);
        if let Some(rpc_id) = plan {
            (self.sink)(ChatEvent::ApprovalResolved { request_id: request_id.into(), decision });
            let outcome = if decision == Decision::Deny { "revise" } else { "approved" };
            return self.rpc.reply(&rpc_id, json!({ "outcome": outcome }));
        }
        let (rpc_id, options) = self.state.lock().unwrap().pending.remove(request_id).ok_or("that question was already answered")?;
        (self.sink)(ChatEvent::ApprovalResolved { request_id: request_id.into(), decision });
        self.rpc.reply(&rpc_id, permission_outcome(&options, decision))
    }

    fn answer_question(&mut self, request_id: &str, answers: std::collections::BTreeMap<String, Vec<String>>) -> DriverResult<()> {
        let ask = self.state.lock().unwrap().asks.remove(request_id);
        if let Some((rpc_id, asked)) = ask {
            let reply = ask_reply(&asked, &answers);
            (self.sink)(ChatEvent::QuestionResolved { request_id: request_id.into(), answers });
            return self.rpc.reply(&rpc_id, reply);
        }
        let choice = self.state.lock().unwrap().choices.remove(request_id);
        if let Some((rpc_id, options)) = choice {
            let reply = choice_reply(&options, &answers);
            (self.sink)(ChatEvent::QuestionResolved { request_id: request_id.into(), answers });
            return self.rpc.reply(&rpc_id, reply);
        }
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
        if r["configOptions"].is_array() {
            self.adopt_options(&r);
        } else {
            self.state.lock().unwrap().current_model = Some(model.to_string());
        }
        Ok(())
    }

    fn set_effort(&mut self, effort: &str) -> DriverResult<bool> {
        let sid = self.session()?;
        let (option, values) = {
            let s = self.state.lock().unwrap();
            (s.effort_option.clone(), s.efforts.clone())
        };
        // No effort to pick for this model: nothing to do.
        let Some(id) = option else { return Ok(true) };
        let value = if effort.is_empty() { "default" } else { effort };
        if !values.iter().any(|v| v == value) {
            return Ok(true);
        }
        let r = self.rpc.call(
            "session/set_config_option",
            json!({"sessionId": sid, "configId": id, "value": value}),
            Duration::from_secs(30),
        )?;
        // OpenCode's efforts are model variants: the answer can move the model too.
        self.adopt_options(&r);
        Ok(true)
    }

    /// `/compact` as OpenCode, Antigravity and Grok have it: a turn running their own `compact`
    /// command, which takes no instructions. The summary isn't shown; a note says it's done.
    fn compact(&mut self, instructions: &str) -> DriverResult<bool> {
        if !instructions.trim().is_empty() {
            return Err(format!("{}'s /compact doesn't take instructions", self.rpc.name));
        }
        self.session()?;
        {
            let mut s = self.state.lock().unwrap();
            if s.busy {
                return Err("Wait for the agent to finish before compacting.".into());
            }
            s.compacting = true;
        }
        let prompt =
            Prompt { id: format!("compact-{}", self.state.lock().unwrap().turns), text: "/compact".into(), images: vec![], files: vec![] };
        run_prompt(self.rpc.clone(), self.state.clone(), self.sink.clone(), self.queue.clone(), prompt);
        Ok(true)
    }

    fn background(&self) -> usize {
        self.state.lock().unwrap().tasks.running()
    }

    fn resumable(&self) -> bool {
        self.state.lock().unwrap().resumable
    }

    fn idle(&self) -> bool {
        let s = self.state.lock().unwrap();
        !s.busy
            && s.pending.is_empty()
            && s.questions.is_empty()
            && s.choices.is_empty()
            && s.asks.is_empty()
            && s.plans.is_empty()
            && self.queue.lock().unwrap().is_empty()
    }

    fn set_permission(&mut self, mode: PermissionMode) -> DriverResult<()> {
        self.permission = mode;
        {
            let mut s = self.state.lock().unwrap();
            s.permission = mode;
            if mode != PermissionMode::Plan {
                s.after_plan = mode;
            }
        }
        self.apply_mode(mode)
    }

    fn stop(&mut self) {
        self.rpc.stdin.lock().unwrap().take();
        // Stdin closed: it may exit by itself (saving its session); then it and everything it
        // started (commands, servers) end.
        if let Some(mut c) = self.child.lock().unwrap().take() {
            oah_transport::stop_tree(&mut c, std::time::Duration::from_millis(500));
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
                "_x.ai/ask_user_question" => self.ask_user(id, p),
                "_x.ai/exit_plan_mode" => self.exit_plan(id, p),
                _ => {
                    let _ = self.rpc.reply_error(id, -32601, "oah doesn't offer this");
                }
            }
            return;
        }
        if method == "session/update" {
            // A reopened session replays its history as updates; the chat already has it. Its
            // usage still counts: the session's cost so far, so the next turn gets only its own.
            let kind = p["update"]["sessionUpdate"].as_str().unwrap_or("");
            if self.state.lock().unwrap().loading && !matches!(kind, "available_commands_update" | "usage_update") {
                return;
            }
            self.update(&p["update"]);
        }
    }

    fn update(&mut self, u: &Value) {
        match u["sessionUpdate"].as_str().unwrap_or("") {
            kind @ ("agent_message_chunk" | "agent_thought_chunk") => {
                let Some(text) = u["content"]["text"].as_str().filter(|t| !t.is_empty()) else { return };
                // `/compact`'s summary is for the agent; the chat gets a note when it's done.
                if self.state.lock().unwrap().compacting {
                    return;
                }
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
                    self.state.lock().unwrap().cost = Some(amount);
                }
                if let (Some(used), Some(size)) = (u["used"].as_u64(), u["size"].as_u64()) {
                    self.emit(ChatEvent::Context { used_tokens: used, window_tokens: size, parts: vec![] });
                }
            }
            "config_option_update" => {
                let (mut models, current, option) = models_from(u);
                let mut st = self.state.lock().unwrap();
                read_effort(&mut st, u);
                if !models.is_empty() {
                    st.model_option = option;
                    st.current_model = current.clone();
                    known_efforts(&mut models, &st.known_efforts);
                    add_efforts(&mut models, current.as_deref(), &st.efforts);
                    drop(st);
                    self.emit(ChatEvent::Models { models, current });
                }
            }
            "available_commands_update" => {
                let mut commands = acp_commands(&u["availableCommands"]);
                match self.state.lock().unwrap().kind {
                    // Antigravity's `logout` would sign every chat out.
                    Some(AgentKind::Antigravity) => commands.retain(|c| c.name != "logout"),
                    Some(AgentKind::Grok) => commands.retain(|c| !crate::grok::hidden_command(&c.name)),
                    _ => {}
                }
                self.emit(ChatEvent::Commands { commands })
            }
            // The agent went in or out of plan mode by itself (Grok: its plan tool, or an approved
            // plan): the chat's mode follows. Our own switches find it already there.
            "current_mode_update" => {
                let Some(mode) = u["currentModeId"].as_str() else { return };
                let changed = {
                    let mut st = self.state.lock().unwrap();
                    match (mode == "plan", st.permission == PermissionMode::Plan) {
                        (true, false) => {
                            st.after_plan = st.permission;
                            st.permission = PermissionMode::Plan;
                            Some(PermissionMode::Plan)
                        }
                        (false, true) => {
                            st.permission = st.after_plan;
                            Some(st.after_plan)
                        }
                        _ => None,
                    }
                };
                if let Some(mode) = changed {
                    self.emit(ChatEvent::PermissionChanged { mode });
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
        // A question's tool call ("Which color?" → "Response received"): the card is its record.
        if is_question(&id) && self.state.lock().unwrap().kind == Some(AgentKind::Antigravity) {
            return;
        }
        // Grok names its tools in `_meta` ("x.ai/tool"); its question and plan-approval tools are
        // shown by their cards. Only the first updates carry the name.
        let grok_tool = &u["_meta"]["x.ai/tool"];
        {
            let mut st = self.state.lock().unwrap();
            if matches!(grok_tool["name"].as_str(), Some("ask_user_question" | "exit_plan_mode")) {
                st.hidden_tools.insert(id.clone());
            }
            if st.hidden_tools.contains(&id) {
                return;
            }
        }
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
        if let Some(k) = u["kind"].as_str().or(grok_tool["kind"].as_str().filter(|_| known.is_none())) {
            call.kind = tool_kind(k);
            call.name = k.to_string();
        }
        // Grok's first title is the tool's own name (`run_terminal_command`): the input says more.
        if let Some(t) = u["title"].as_str().filter(|t| !t.is_empty() && grok_tool["name"].as_str() != Some(t)) {
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
            self.emit(ChatEvent::ToolStarted { item_id: id.clone(), tool: call.clone() });
        }
        match u["status"].as_str() {
            Some("completed") | Some("failed") => {
                let ok = u["status"] == "completed";
                let diff = diffs_of(&u["content"]);
                let output = clip(output_of(u));
                // A background task that ended.
                let ended = {
                    let mut st = self.state.lock().unwrap();
                    st.open.remove(&id);
                    let status = if ok { TaskStatus::Completed } else { TaskStatus::Failed };
                    let detail = output.lines().rev().find(|l| !l.trim().is_empty()).map(|l| l.trim().chars().take(160).collect());
                    st.tasks
                        .finish(&id, status, detail)
                        .map(|t| format!("Background task ended: {}", t.title))
                        .map(|n| (n, st.tasks.event()))
                };
                self.emit(ChatEvent::ToolFinished { item_id: id, ok, output, diff });
                if let Some((notice, tasks)) = ended {
                    self.emit(tasks);
                    self.emit(ChatEvent::Notice { level: NoticeLevel::Info, text: notice });
                }
            }
            _ => {
                let mut st = self.state.lock().unwrap();
                if st.busy && !st.loading {
                    st.open.insert(id, call);
                }
            }
        }
    }

    fn permission(&mut self, rpc_id: &Value, p: &Value) {
        let options: Vec<Value> = p["options"].as_array().cloned().unwrap_or_default();
        let tc = &p["toolCall"];
        let id = tc["toolCallId"].as_str().map(String::from);
        // Antigravity asks its questions this way: each option is an answer. Never answered by
        // the mode, whatever it allows.
        if id.as_deref().is_some_and(is_question) && self.state.lock().unwrap().kind == Some(AgentKind::Antigravity) {
            let request_id = format!("acpq-{rpc_id}");
            let question = Question {
                id: "answer".into(),
                header: String::new(),
                question: tc["title"].as_str().filter(|t| !t.is_empty()).unwrap_or("The agent asks:").to_string(),
                options: options
                    .iter()
                    .filter_map(|o| o["name"].as_str())
                    .map(|n| QuestionOption { label: n.to_string(), description: String::new() })
                    .collect(),
                multi: false,
                other: false,
                secret: false,
            };
            self.state.lock().unwrap().choices.insert(request_id.clone(), (rpc_id.clone(), options));
            self.emit(ChatEvent::Question { request: QuestionRequest { request_id, questions: vec![question] } });
            return;
        }
        let known = id.as_ref().and_then(|i| self.state.lock().unwrap().tools.get(i).cloned());
        let kind = tc["kind"].as_str().map(tool_kind).or(known.as_ref().map(|k| k.kind)).unwrap_or(ToolKind::Other);
        // Full access answers everything, Accept edits answers edits: allow once, never always.
        let mode = self.state.lock().unwrap().permission;
        let edit = matches!(kind, ToolKind::Edit | ToolKind::Write);
        if mode == PermissionMode::FullAccess || (mode == PermissionMode::AcceptEdits && edit) {
            let _ = self.rpc.reply(rpc_id, permission_outcome(&options, Decision::Allow));
            return;
        }
        let title = tc["title"]
            .as_str()
            .filter(|t| !t.is_empty())
            .map(String::from)
            .or(known.as_ref().map(|k| k.title.clone()))
            .unwrap_or_else(|| "Use a tool".into());
        let input =
            if tc["rawInput"].is_null() { known.as_ref().map(|k| k.input.clone()).unwrap_or(Value::Null) } else { tc["rawInput"].clone() };
        let (akind, detail) = match kind {
            ToolKind::Bash => (ApprovalKind::Command, command_of(&input).map(String::from).or(Some(title.clone()))),
            ToolKind::Edit | ToolKind::Write => (ApprovalKind::Edit, None),
            _ => (ApprovalKind::Other, input["description"].as_str().map(String::from)),
        };
        // Antigravity warns about risky "Allow always" choices.
        let warning = options.iter().find_map(|o| {
            let w = &o["_meta"]["agy.security.warning"];
            let text = [w["title"].as_str(), w["message"].as_str()].into_iter().flatten().collect::<Vec<_>>().join(": ");
            Some(text.chars().take(512).collect::<String>()).filter(|t| !t.is_empty())
        });
        let detail = match (detail, warning) {
            (Some(d), Some(w)) => Some(format!("{d}\n\n⚠ {w}")),
            (None, Some(w)) => Some(format!("⚠ {w}")),
            (d, None) => d,
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

    /// Grok's question tool: each question with its options, answered by the question's text.
    fn ask_user(&mut self, rpc_id: &Value, p: &Value) {
        let mut asked = Vec::new();
        let questions: Vec<Question> = p["questions"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(i, q)| {
                let text = q["question"].as_str().filter(|t| !t.is_empty())?.to_string();
                let multi = q["multiSelect"] == true;
                asked.push((text.clone(), multi));
                Some(Question {
                    id: format!("q{i}"),
                    header: q["header"].as_str().unwrap_or("").to_string(),
                    question: text,
                    options: q["options"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|o| {
                            let label = o["label"].as_str()?.to_string();
                            let description = o["description"].as_str().filter(|d| !d.eq_ignore_ascii_case(&label)).unwrap_or("");
                            Some(QuestionOption { label, description: description.to_string() })
                        })
                        .collect(),
                    multi,
                    other: true,
                    secret: false,
                })
            })
            .collect();
        if questions.is_empty() {
            let _ = self.rpc.reply(rpc_id, json!({"outcome": "cancelled"}));
            return;
        }
        let request_id = format!("acpq-{rpc_id}");
        self.state.lock().unwrap().asks.insert(request_id.clone(), (rpc_id.clone(), asked));
        self.emit(ChatEvent::Question { request: QuestionRequest { request_id, questions } });
    }

    /// Grok's finished plan, to approve before it leaves plan mode. Always the user's call.
    fn exit_plan(&mut self, rpc_id: &Value, p: &Value) {
        let request_id = format!("acp-{rpc_id}");
        self.state.lock().unwrap().plans.insert(request_id.clone(), rpc_id.clone());
        self.emit(ChatEvent::Approval {
            request: ApprovalRequest {
                request_id,
                item_id: p["toolCallId"].as_str().map(String::from),
                kind: ApprovalKind::Plan,
                title: "Leave plan mode and start on this plan?".into(),
                detail: p["planContent"].as_str().filter(|t| !t.trim().is_empty()).map(String::from),
                diff: None,
                can_remember: false,
            },
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

/// One question in a session of its own, answered as plain text (Grok's one-off answers: the
/// question can be longer than a command line takes). Whatever the agent asks to run or edit is
/// refused and its questions are skipped. The session is deleted afterwards.
pub fn ask_once(opts: StartOptions, question: &str, limit: Duration) -> DriverResult<String> {
    let kind = opts.kind;
    let (tx, rx) = std::sync::mpsc::channel::<ChatEvent>();
    let tx = Mutex::new(tx);
    let sink: Sink = Arc::new(move |e| {
        let _ = tx.lock().unwrap().send(e);
    });
    let mut d = AcpDriver::start(StartOptions { permission: PermissionMode::Ask, resume: None, ..opts }, sink)?;
    d.send(Prompt { id: "oneshot".into(), text: question.into(), images: vec![], files: vec![] })?;
    let started = std::time::Instant::now();
    // The last message is the answer.
    let (mut text, mut message) = (String::new(), String::new());
    let mut error = None;
    loop {
        let Some(left) = limit.checked_sub(started.elapsed()) else {
            error = Some(format!("{} didn't answer in time", d.rpc.name));
            break;
        };
        match rx.recv_timeout(left) {
            Ok(ChatEvent::TextDelta { item_id, text: t }) => {
                if item_id != message {
                    message = item_id;
                    text.clear();
                }
                text.push_str(&t);
            }
            Ok(ChatEvent::Approval { request }) => {
                let _ = d.answer(&request.request_id, Decision::Deny);
            }
            Ok(ChatEvent::Question { request }) => {
                let _ = d.answer_question(&request.request_id, Default::default());
            }
            Ok(ChatEvent::Error { message }) => error = Some(message),
            Ok(ChatEvent::TurnFinished { .. }) | Ok(ChatEvent::Exited { .. }) => break,
            Ok(_) => {}
            Err(_) => {
                error = Some(format!("{} didn't answer in time", d.rpc.name));
                break;
            }
        }
    }
    let (sid, deletable) = {
        let s = d.state.lock().unwrap();
        (s.session_id.clone(), s.deletable)
    };
    if let Some(sid) = sid {
        let method = if kind == AgentKind::Grok { Some("_x.ai/session/delete") } else { deletable.then_some("session/delete") };
        if let Some(m) = method {
            let _ = d.rpc.call(m, json!({"sessionId": sid}), Duration::from_secs(10));
        }
    }
    d.stop();
    match error {
        Some(e) if text.trim().is_empty() => Err(e),
        _ => Ok(text),
    }
}

/// OpenCode's models for a chat that hasn't started: `opencode models` prints `provider/model`
/// per line.
pub fn opencode_models(binary: &str, location: &oah_core::settings::Location, cwd: &str, env: &[EnvVar]) -> DriverResult<Vec<ModelOption>> {
    let program = if binary.trim().is_empty() { "opencode" } else { binary.trim() };
    let run = |args: &[&str]| {
        let args = args.iter().map(|a| a.to_string()).collect();
        oah_transport::run(&Spec { location, cwd, program, args, env }, None, Duration::from_secs(30))
    };
    // `--verbose` adds names and effort levels; OpenCode 2 dropped the flag, so then the plain list.
    let mut out = run(&["models", "--verbose"])?;
    if !out.ok() {
        out = run(&["models"])?;
    }
    if !out.ok() {
        let why = out.stderr.lines().chain(out.stdout.lines()).map(str::trim).rfind(|l| !l.is_empty());
        return Err(why.unwrap_or("opencode models failed").to_string());
    }
    Ok(parse_opencode_models(&out.stdout))
}

/// `opencode models` (one `provider/model` per line) or `--verbose` (each line followed by the
/// model's JSON, whose `variants` keys are its effort levels, and `name`).
pub fn parse_opencode_models(out: &str) -> Vec<ModelOption> {
    let mut models: Vec<ModelOption> = Vec::new();
    let mut json = String::new();
    let flush = |models: &mut Vec<ModelOption>, json: &mut String| {
        if let (Some(m), Ok(v)) = (models.last_mut(), serde_json::from_str::<Value>(json)) {
            if let Some(name) = v["name"].as_str() {
                m.name = name.to_string();
            }
            if let Some(vs) = v["variants"].as_object() {
                m.efforts = vs.keys().cloned().collect();
                m.efforts.sort_by_key(|e| effort_rank(e));
            }
        }
        json.clear();
    };
    for line in out.lines() {
        let t = line.trim_end();
        let is_id = !t.is_empty() && !t.starts_with([' ', '{', '}', '"', '[', ']']) && t.contains('/') && !t.contains(' ');
        if is_id {
            flush(&mut models, &mut json);
            let (provider, model) = t.split_once('/').unwrap_or(("", t));
            models.push(ModelOption {
                id: t.to_string(),
                name: model.to_string(),
                description: provider.to_string(),
                ..Default::default()
            });
        } else if !models.is_empty() {
            json.push_str(line);
            json.push('\n');
        }
    }
    flush(&mut models, &mut json);
    models
}

/// Effort levels from lowest to highest (unknown ones last).
pub(crate) fn effort_rank(e: &str) -> usize {
    ["none", "minimal", "low", "medium", "high", "xhigh", "max"].iter().position(|x| *x == e).unwrap_or(99)
}

// ---- Helpers ---------------------------------------------------------------------------------

/// "Authentication required" (-32000), or the provider's 401 passed through.
fn is_auth_error(e: &str) -> bool {
    let e = e.to_lowercase();
    e.contains("auth") || e.contains("401") || e.contains("sign in") || e.contains("log in") || e.contains("login")
}

/// What to do when the agent isn't signed in, and where oah's Sign in button is.
const NOT_SIGNED_IN: &str = "OpenCode isn't signed in. Sign in with `opencode auth login` (Settings → Providers → OpenCode → Sign in).";

fn not_signed_in(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Antigravity => crate::antigravity::NOT_SIGNED_IN,
        AgentKind::Grok => crate::grok::NOT_SIGNED_IN,
        _ => NOT_SIGNED_IN,
    }
}

/// The agent's mode for oah's permission mode. What the mode allows past the agent's own asking
/// (Accept edits, Full access) oah answers itself, so the mode can change mid-chat.
fn agent_mode(kind: Option<AgentKind>, mode: PermissionMode) -> &'static str {
    match (kind, mode) {
        (_, PermissionMode::Plan) => "plan",
        (Some(AgentKind::Antigravity), PermissionMode::AcceptEdits) => "accept-edits",
        (Some(AgentKind::Antigravity | AgentKind::Grok), _) => "default",
        _ => "build",
    }
}

fn known_efforts(models: &mut [ModelOption], known: &HashMap<String, (Vec<String>, Option<String>)>) {
    for m in models.iter_mut().filter(|m| m.efforts.is_empty()) {
        if let Some((e, default)) = known.get(&m.id) {
            m.efforts = e.clone();
            m.default_effort = m.default_effort.clone().or(default.clone());
        }
    }
}

/// Antigravity lists each model once per effort (`claude-opus-5-5-high`, unstable `models`) next
/// to its config option's base models (`claude-opus-5-5`): the suffixes are that model's efforts.
fn variant_efforts(models: &mut [ModelOption], listed: &Value) {
    let ids: Vec<&str> = listed["availableModels"].as_array().into_iter().flatten().filter_map(|m| m["modelId"].as_str()).collect();
    for m in models.iter_mut().filter(|m| m.efforts.is_empty()) {
        let mut efforts: Vec<String> = ids
            .iter()
            .filter_map(|id| id.strip_prefix(m.id.as_str())?.strip_prefix('-'))
            .filter(|e| effort_rank(e) < 99)
            .map(String::from)
            .collect();
        efforts.sort_by_key(|e| effort_rank(e));
        if efforts.len() > 1 {
            m.efforts = efforts;
        }
    }
}

/// Models listed with their efforts in `_meta.reasoningEfforts` (Grok's session `models`): each
/// model gets its own, not just the current one.
fn listed_efforts(models: &mut [ModelOption], listed: &Value) {
    let meta = crate::grok::parse_models(listed);
    for m in models.iter_mut() {
        if let Some(l) = meta.iter().find(|l| l.id == m.id && !l.efforts.is_empty()) {
            m.efforts = l.efforts.clone();
            m.default_effort = l.default_effort.clone();
        }
    }
}

/// Grok's answers: `{outcome: accepted, answers: {<question>: <label or labels>}}`; nothing
/// answered cancels. A typed answer goes as it was typed.
fn ask_reply(asked: &[(String, bool)], answers: &std::collections::BTreeMap<String, Vec<String>>) -> Value {
    let mut out = serde_json::Map::new();
    for (i, (text, multi)) in asked.iter().enumerate() {
        let picked: Vec<String> = answers
            .get(&format!("q{i}"))
            .into_iter()
            .flatten()
            .map(|a| a.trim_end_matches(" (Recommended)").to_string())
            .filter(|a| !a.trim().is_empty())
            .collect();
        match picked.len() {
            0 => {}
            1 if !multi => {
                out.insert(text.clone(), json!(picked[0]));
            }
            _ => {
                out.insert(text.clone(), json!(picked));
            }
        }
    }
    if out.is_empty() {
        return json!({"outcome": "cancelled"});
    }
    json!({"outcome": "accepted", "answers": out})
}

/// The models (with every model's efforts) and commands OpenCode offers, for a chat that hasn't
/// started. 1.x lists models from its CLI (`models --verbose`); 2.x only knows them in a session
/// (its CLI leaves out providers that report late and dropped `--verbose`), so then a throwaway
/// session is opened, each model picked once to learn its efforts, and the session deleted.
pub fn opencode_catalog(opts: StartOptions) -> DriverResult<(Vec<ModelOption>, Vec<SlashCommand>)> {
    if opts.kind == oah_core::settings::AgentKind::Opencode {
        if let Ok(models) = opencode_cli_catalog(&opts) {
            return Ok((models, opencode_commands()));
        }
    }
    opencode_session_catalog(opts)
}

fn opencode_cli_catalog(opts: &StartOptions) -> DriverResult<Vec<ModelOption>> {
    let program = if opts.binary.trim().is_empty() { opts.kind.default_binary() } else { opts.binary.trim() };
    let args = vec!["models".to_string(), "--verbose".to_string()];
    let out = oah_transport::run(
        &Spec { location: &opts.location, cwd: &opts.cwd, program, args, env: &opts.env },
        None,
        Duration::from_secs(30),
    )?;
    // 2.x rejects `--verbose` (and prints its help).
    if !out.ok() {
        return Err("no verbose listing".into());
    }
    let models = parse_opencode_models(&out.stdout);
    if models.is_empty() {
        return Err("no models listed".into());
    }
    Ok(models)
}

/// How long a catalog spends picking models to learn their efforts.
const EFFORT_PROBE_BUDGET: Duration = Duration::from_secs(6);

fn opencode_session_catalog(opts: StartOptions) -> DriverResult<(Vec<ModelOption>, Vec<SlashCommand>)> {
    // The session's models as first listed, and its commands once they come.
    type Seen = (Vec<ModelOption>, Option<Vec<SlashCommand>>);
    let seen: Arc<Mutex<Seen>> = Arc::new(Mutex::new((vec![], None)));
    let s2 = seen.clone();
    let sink: Sink = Arc::new(move |e| {
        let mut s = s2.lock().unwrap();
        match e {
            ChatEvent::Models { models, .. } if s.0.is_empty() => s.0 = models,
            ChatEvent::Commands { commands } => s.1 = Some(commands),
            _ => {}
        }
    });
    let mut d = AcpDriver::start(StartOptions { model: None, effort: None, resume: None, permission: PermissionMode::Ask, ..opts }, sink)?;
    let mut models = seen.lock().unwrap().0.clone();
    let started = std::time::Instant::now();
    for m in models.iter_mut() {
        if !m.efforts.is_empty() {
            continue;
        }
        if started.elapsed() > EFFORT_PROBE_BUDGET {
            break;
        }
        if d.set_model(&m.id).is_ok() {
            m.efforts = d.state.lock().unwrap().efforts.iter().filter(|e| *e != "default").cloned().collect();
        }
    }
    // The commands come right after the session starts; give them a moment if not yet.
    for _ in 0..10 {
        if seen.lock().unwrap().1.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let (sid, deletable) = {
        let s = d.state.lock().unwrap();
        (s.session_id.clone(), s.deletable)
    };
    if let (Some(sid), true) = (sid, deletable) {
        let _ = d.rpc.call("session/delete", json!({"sessionId": sid}), Duration::from_secs(10));
    }
    d.stop();
    let commands = seen.lock().unwrap().1.clone().unwrap_or_else(opencode_commands);
    Ok((models, commands))
}

/// OpenCode's commands before its first `available_commands_update` (a chat that hasn't started).
/// Its skills come once a chat runs.
pub fn opencode_commands() -> Vec<SlashCommand> {
    let c = |name: &str, description: &str, hint: &str| SlashCommand {
        name: name.into(),
        description: description.into(),
        hint: hint.into(),
        skill: false,
    };
    vec![c("init", "Guided AGENTS.md setup", ""), c("review", "Review changes, uncommitted by default", "commit|branch|pr")]
}

/// `availableCommands: [{name, description, input: {hint}}]`.
pub(crate) fn acp_commands(v: &Value) -> Vec<SlashCommand> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            Some(SlashCommand {
                name: c["name"].as_str()?.trim_start_matches('/').to_string(),
                description: c["description"].as_str().unwrap_or("").into(),
                hint: c["input"]["hint"].as_str().unwrap_or("").into(),
                skill: false,
            })
        })
        .filter(|c| !c.name.is_empty())
        .collect()
}

fn tool_kind(k: &str) -> ToolKind {
    match k {
        "read" => ToolKind::Read,
        "edit" | "delete" | "move" => ToolKind::Edit,
        "write" => ToolKind::Write,
        "search" => ToolKind::Search,
        "execute" => ToolKind::Bash,
        "fetch" => ToolKind::Web,
        // OpenCode's `task` tool (a subagent) comes as `think`.
        "think" => ToolKind::Task,
        _ => ToolKind::Other,
    }
}

fn input_title(input: &Value) -> String {
    command_of(input)
        .or_else(|| ["filePath", "file_path", "path", "pattern", "query", "url", "description"].iter().find_map(|k| input[*k].as_str()))
        .unwrap_or("")
        .to_string()
}

/// A shell tool's command line: `command` (OpenCode), `CommandLine` / `commandLine` /
/// `command_line` (Antigravity).
fn command_of(input: &Value) -> Option<&str> {
    ["command", "CommandLine", "commandLine", "command_line"].iter().find_map(|k| input[*k].as_str())
}

/// Antigravity's question tool calls (`interaction_…`).
fn is_question(tool_call_id: &str) -> bool {
    tool_call_id.starts_with("interaction_")
}

/// The answer to a choice question: the option whose name was picked (a picked option is the only
/// answer Antigravity takes), or cancelled.
fn choice_reply(options: &[Value], answers: &std::collections::BTreeMap<String, Vec<String>>) -> Value {
    let picked = answers.values().flatten().next().map(|a| a.trim_end_matches(" (Recommended)").to_string());
    match picked.and_then(|p| options.iter().find(|o| o["name"].as_str() == Some(p.as_str()))).and_then(|o| o["optionId"].as_str()) {
        Some(id) => json!({"outcome": {"outcome": "selected", "optionId": id}}),
        None => json!({"outcome": {"outcome": "cancelled"}}),
    }
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
        if let Some(t) = raw["metadata"]["output"]
            .as_str()
            .or(raw["output"].as_str())
            .or(raw["combinedOutput"].as_str())
            .or(raw["combined_output"].as_str())
            .or(raw.as_str())
        {
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
        cache_write_tokens: n("cachedWriteTokens").max(n("cacheCreationTokens")),
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
        ..Default::default()
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

/// The session's reasoning effort option: category `thought_level` (OpenCode calls it
/// `effort`), with its values.
fn read_effort(st: &mut State, v: &Value) {
    let Some(opts) = v["configOptions"].as_array() else { return };
    match opts.iter().find(|o| o["category"] == "thought_level" || o["id"] == "effort") {
        Some(o) => {
            st.effort_option = o["id"].as_str().map(String::from);
            st.efforts = select_values(&o["options"]).into_iter().map(|m| m.id).collect();
        }
        None => {
            st.effort_option = None;
            st.efforts.clear();
        }
    }
}

/// Gives the current model the session's effort levels (`default` is "none picked", not one).
fn add_efforts(models: &mut [ModelOption], current: Option<&str>, efforts: &[String]) {
    if let Some(m) = models.iter_mut().find(|m| Some(m.id.as_str()) == current) {
        if m.efforts.is_empty() {
            m.efforts = efforts.iter().filter(|e| *e != "default").cloned().collect();
        }
    }
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
        // (value, label, description): a choice is sent back by its `const`, shown by its title.
        let choices = |p: &Value| -> Vec<(String, String, String)> {
            if let Some(e) = p["enum"].as_array() {
                return e.iter().filter_map(|x| x.as_str().map(|s| (s.to_string(), s.to_string(), String::new()))).collect();
            }
            p["oneOf"]
                .as_array()
                .or(p["anyOf"].as_array())
                .into_iter()
                .flatten()
                .filter_map(|x| {
                    let value = x["const"].as_str()?.to_string();
                    let label = x["title"].as_str().unwrap_or(&value).to_string();
                    let description = x["description"].as_str().filter(|d| *d != label).unwrap_or("").to_string();
                    Some((value, label, description))
                })
                .collect()
        };
        let (kind, offered) = match prop["type"].as_str() {
            Some("boolean") => {
                (FieldKind::Bool, vec![("true".into(), "Yes".into(), String::new()), ("false".into(), "No".into(), String::new())])
            }
            Some("array") => (FieldKind::Multi, choices(&prop["items"])),
            _ => {
                let c = choices(prop);
                (if c.is_empty() { FieldKind::Text } else { FieldKind::Choice }, c)
            }
        };
        let options = offered
            .iter()
            .map(|(v, t, d)| QuestionOption {
                label: if Some(v) == default.as_ref() { format!("{t} (Recommended)") } else { t.clone() },
                description: d.clone(),
            })
            .collect();
        let title = prop["title"].as_str().or(prop["description"].as_str()).unwrap_or(key).to_string();
        let ask = prop["description"]
            .as_str()
            .filter(|d| prop["title"].is_string() && *d != title)
            .map(|d| format!("{title}: {d}"))
            .unwrap_or(title);
        questions.push(Question {
            id: key.clone(),
            header: if questions.is_empty() && !message.is_empty() { String::new() } else { key.clone() },
            question: if questions.is_empty() && !message.is_empty() { format!("{message}\n{ask}") } else { ask },
            options,
            multi: kind == FieldKind::Multi,
            other: kind == FieldKind::Text || other_key.is_some(),
            secret: prop["format"] == "password",
        });
        fields.push(Field { key: key.clone(), kind, other_key, choices: offered.into_iter().map(|(v, t, _)| (v, t)).collect() });
    }
    (questions, fields)
}

/// The answers → `{action, content}`. A picked label goes back as its choice's value; typed
/// text goes to the field's `_custom` / `__other` key when it has one. Nothing answered cancels.
fn elicitation_reply(fields: &[Field], answers: &std::collections::BTreeMap<String, Vec<String>>) -> Value {
    if answers.values().all(|a| a.is_empty()) {
        return json!({"action": "cancel"});
    }
    let strip = |s: &str| s.trim_end_matches(" (Recommended)").to_string();
    let mut content = serde_json::Map::new();
    for f in fields {
        let Some(a) = answers.get(&f.key).filter(|a| !a.is_empty()) else { continue };
        let mut picked = Vec::new();
        let mut typed = Vec::new();
        for answer in a {
            match f.choices.iter().find(|(_, label)| *label == strip(answer)) {
                Some((value, _)) => picked.push(value.clone()),
                None => typed.push(answer.clone()),
            }
        }
        let typed = Some(typed.join(", ")).filter(|t| !t.trim().is_empty());
        match f.kind {
            FieldKind::Bool => {
                if let Some(v) = picked.first() {
                    content.insert(f.key.clone(), json!(v == "true"));
                }
            }
            FieldKind::Multi => {
                content.insert(f.key.clone(), json!(picked));
            }
            FieldKind::Choice => {
                if let Some(v) = picked.first() {
                    content.insert(f.key.clone(), json!(v));
                }
            }
            FieldKind::Text => {
                if let Some(t) = &typed {
                    content.insert(f.key.clone(), json!(t));
                }
                continue;
            }
        }
        if let Some(t) = typed {
            match &f.other_key {
                Some(k) => content.insert(k.clone(), json!(t)),
                // No field for it: the typed answer stands in for a choice.
                None => content.insert(f.key.clone(), json!(t)),
            };
        }
    }
    json!({"action": "accept", "content": content})
}

/// OpenCode's permission rule asking before edits and commands.
const ASK_FOR_EDITS_AND_COMMANDS: &str = r#"{"edit":"ask","bash":"ask"}"#;

/// Puts the ask-first rule in `OPENCODE_CONFIG_CONTENT`: added to your own config there when it
/// has no `permission`, left alone when it has one (or isn't JSON oah can read).
fn with_ask_config(env: &mut Vec<EnvVar>) {
    let rule: Value = serde_json::from_str(ASK_FOR_EDITS_AND_COMMANDS).unwrap_or_default();
    match env.iter_mut().rfind(|e| e.key == "OPENCODE_CONFIG_CONTENT") {
        Some(e) => {
            if let Ok(Value::Object(mut config)) = serde_json::from_str::<Value>(&e.value) {
                if !config.contains_key("permission") {
                    config.insert("permission".into(), rule);
                    e.value = Value::Object(config).to_string();
                }
            }
        }
        None => {
            env.push(EnvVar { key: "OPENCODE_CONFIG_CONTENT".into(), value: json!({"permission": rule}).to_string(), in_keychain: false })
        }
    }
}

/// `2.2.6` → 2; `v1.18.31` → 1; unknown → 0.
fn major_version(v: &str) -> u64 {
    v.trim().trim_start_matches('v').split('.').next().and_then(|m| m.parse().ok()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oah_core::settings::AgentKind;

    fn run(lines: &[Value], permission: PermissionMode) -> (Vec<ChatEvent>, Arc<Mutex<State>>) {
        run_as(None, lines, permission)
    }

    fn run_as(kind: Option<AgentKind>, lines: &[Value], permission: PermissionMode) -> (Vec<ChatEvent>, Arc<Mutex<State>>) {
        let out = Arc::new(Mutex::new(Vec::new()));
        let o = out.clone();
        let sink: Sink = Arc::new(move |e| o.lock().unwrap().push(e));
        let state =
            Arc::new(Mutex::new(State { alive: true, turn_id: Some("turn-1".into()), stretch: 1, permission, kind, ..Default::default() }));
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

    /// OpenCode 2 has no `models --verbose`: the plain list still comes back.
    #[test]
    #[cfg(unix)]
    fn lists_opencode_2_models() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("oah-opencode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("opencode");
        let script = "#!/bin/sh\nfor a; do [ \"$a\" = --verbose ] && { echo 'ERROR' >&2; echo '  Unrecognized flag: --verbose in command opencode models' >&2; exit 1; }; done\nprintf 'opencode/big-pickle\\nopencode/glm-5.3-flash-free\\n'\n";
        std::fs::write(&bin, script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let loc = oah_core::settings::Location::Local;
        let m = opencode_models(bin.to_str().unwrap(), &loc, dir.to_str().unwrap(), &[]).unwrap();
        assert_eq!(m.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["opencode/big-pickle", "opencode/glm-5.3-flash-free"]);
        assert_eq!((m[0].name.as_str(), m[0].description.as_str()), ("big-pickle", "opencode"));
        std::fs::write(&bin, "#!/bin/sh\necho 'not signed in' >&2\nexit 2\n").unwrap();
        assert_eq!(opencode_models(bin.to_str().unwrap(), &loc, dir.to_str().unwrap(), &[]).unwrap_err().to_string(), "not signed in");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn explains_how_to_sign_in() {
        assert!(is_auth_error("Authentication required: no auth method id provided"));
        assert!(is_auth_error("provider returned 401"));
        assert!(!is_auth_error("model not found"));
        assert!(NOT_SIGNED_IN.contains("opencode auth login"));
    }

    #[test]
    fn reads_available_commands() {
        let (ev, state) = run(
            &[upd(json!({"sessionUpdate": "available_commands_update", "availableCommands": [
                {"name": "init", "description": "create/update AGENTS.md"},
                {"name": "review", "description": "review changes", "input": {"hint": "commit|branch|pr"}}
            ]}))],
            PermissionMode::Ask,
        );
        let ChatEvent::Commands { commands } = &ev[0] else { panic!("{ev:?}") };
        assert_eq!(commands.len(), 2);
        assert_eq!((commands[1].name.as_str(), commands[1].hint.as_str()), ("review", "commit|branch|pr"));
        // While a reopened session replays, history is skipped but commands still arrive.
        state.lock().unwrap().loading = true;
        let out = Arc::new(Mutex::new(Vec::new()));
        let o = out.clone();
        let mut tr = Translator { sink: Arc::new(move |e| o.lock().unwrap().push(e)), state, rpc: Rpc::new(None, "OpenCode") };
        tr.handle(&upd(json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "old"}})));
        tr.handle(&upd(json!({"sessionUpdate": "available_commands_update", "availableCommands": [{"name": "init", "description": ""}]})));
        assert!(matches!(&out.lock().unwrap()[..], [ChatEvent::Commands { .. }]));
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
        assert!(matches!(&ev[8], ChatEvent::Context { used_tokens: 12000, window_tokens: 200000, .. }));
    }

    /// Antigravity's question, as captured from 1.x: a tool call, then a permission request whose
    /// options are the answers.
    #[test]
    fn antigravity_questions_become_cards_even_with_full_access() {
        let call = upd(
            json!({"toolCallId": "interaction_28ffdbf7", "title": "Which color do you prefer?", "status": "pending", "rawInput": {}, "sessionUpdate": "tool_call"}),
        );
        let req = json!({"jsonrpc": "2.0", "id": 2, "method": "session/request_permission", "params": {"sessionId": "s",
            "toolCall": {"toolCallId": "interaction_28ffdbf7", "status": "pending", "title": "Which color do you prefer?", "rawInput": {}},
            "options": [{"optionId": "1", "name": "red", "kind": "allow_once"}, {"optionId": "2", "name": "blue", "kind": "allow_once"}]}});
        let done = upd(
            json!({"toolCallId": "interaction_28ffdbf7", "status": "completed", "rawOutput": "Response received", "sessionUpdate": "tool_call_update"}),
        );
        let (ev, state) = run_as(Some(AgentKind::Antigravity), &[call, req, done], PermissionMode::FullAccess);
        // Only the card: no tool row for the question, and full access doesn't answer it.
        match &ev[..] {
            [ChatEvent::Question { request }] => {
                assert_eq!(request.request_id, "acpq-2");
                assert_eq!(request.questions[0].question, "Which color do you prefer?");
                let labels: Vec<&str> = request.questions[0].options.iter().map(|o| o.label.as_str()).collect();
                assert_eq!(labels, ["red", "blue"]);
                assert!(!request.questions[0].other);
            }
            e => panic!("{e:?}"),
        }
        let (_, options) = state.lock().unwrap().choices.get("acpq-2").cloned().unwrap();
        let answer = |a: &str| choice_reply(&options, &[("answer".to_string(), vec![a.to_string()])].into_iter().collect());
        assert_eq!(answer("blue"), json!({"outcome": {"outcome": "selected", "optionId": "2"}}));
        assert_eq!(answer("green")["outcome"]["outcome"], "cancelled");
    }

    #[test]
    fn antigravity_commands_and_shell_tools() {
        let cmds = upd(
            json!({"sessionUpdate": "available_commands_update", "availableCommands": [{"name": "goal", "description": "Run until done."}, {"name": "logout", "description": "Sign out."}]}),
        );
        let tool = upd(
            json!({"sessionUpdate": "tool_call", "toolCallId": "t1", "kind": "execute", "status": "in_progress", "rawInput": {"CommandLine": "cargo test", "Cwd": "/w"}}),
        );
        let end = upd(
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed", "rawOutput": {"combinedOutput": "ok: 9 passed", "exitCode": 0}}),
        );
        let (ev, _) = run_as(Some(AgentKind::Antigravity), &[cmds, tool, end], PermissionMode::Ask);
        match &ev[0] {
            ChatEvent::Commands { commands } => assert_eq!(commands.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["goal"]),
            e => panic!("{e:?}"),
        }
        assert!(matches!(&ev[1], ChatEvent::ToolStarted { tool, .. } if tool.title == "cargo test" && tool.kind == ToolKind::Bash));
        assert!(matches!(&ev[2], ChatEvent::ToolFinished { ok: true, output, .. } if output == "ok: 9 passed"));
        // Asking to run it shows the command.
        let req = json!({"jsonrpc": "2.0", "id": 9, "method": "session/request_permission", "params": {"sessionId": "s",
            "toolCall": {"toolCallId": "t2", "kind": "execute", "title": "rm -rf build", "rawInput": {"CommandLine": "rm -rf build"}},
            "options": [{"optionId": "a", "kind": "allow_once", "name": "Allow"},
                        {"optionId": "b", "kind": "allow_always", "name": "Always", "_meta": {"agy.security.warning": {"title": "Destructive", "message": "Deletes files."}}}]}});
        let (ev, _) = run_as(Some(AgentKind::Antigravity), &[req], PermissionMode::Ask);
        match &ev[0] {
            ChatEvent::Approval { request } => {
                assert_eq!(request.kind, ApprovalKind::Command);
                assert_eq!(request.detail.as_deref(), Some("rm -rf build\n\n⚠ Destructive: Deletes files."));
            }
            e => panic!("{e:?}"),
        }
    }

    #[test]
    fn antigravity_modes_and_model_efforts() {
        assert_eq!(agent_mode(Some(AgentKind::Antigravity), PermissionMode::Ask), "default");
        assert_eq!(agent_mode(Some(AgentKind::Antigravity), PermissionMode::AcceptEdits), "accept-edits");
        assert_eq!(agent_mode(Some(AgentKind::Antigravity), PermissionMode::FullAccess), "default");
        assert_eq!(agent_mode(Some(AgentKind::Antigravity), PermissionMode::Plan), "plan");
        assert_eq!(agent_mode(Some(AgentKind::Opencode), PermissionMode::Ask), "build");
        // `session/new` from agy 1.5.3: base models in the config option, efforts in the ids.
        let session = json!({
            "models": {"availableModels": [
                {"modelId": "gemini-4-argon-medium"}, {"modelId": "gemini-4-argon-high"}, {"modelId": "gemini-4-argon-low"},
                {"modelId": "claude-opus-5-5-low"}, {"modelId": "claude-opus-5-5-high"}, {"modelId": "gpt-oss-120b-medium"}],
                "currentModelId": "gemini-4-argon-medium"},
            "configOptions": [
                {"id": "model", "category": "model", "currentValue": "gemini-4-argon", "options": [
                    {"value": "gemini-4-argon", "name": "Gemini 4 Argon"}, {"value": "claude-opus-5-5", "name": "Claude Opus 5.5"},
                    {"value": "gpt-oss-120b-medium", "name": "GPT-OSS 120B (Medium)"}]},
                {"id": "thought_level", "category": "thought_level", "currentValue": "medium", "options": [{"value": "low"}, {"value": "medium"}, {"value": "high"}]}]
        });
        let (mut models, current, option) = models_from(&session);
        assert_eq!((current.as_deref(), option.as_deref()), (Some("gemini-4-argon"), Some("model")));
        variant_efforts(&mut models, &session["models"]);
        assert_eq!(models[0].efforts, ["low", "medium", "high"]);
        assert_eq!(models[1].efforts, ["low", "high"]);
        assert!(models[2].efforts.is_empty());
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
        let (ev, _) = run(std::slice::from_ref(&req), PermissionMode::FullAccess);
        assert!(ev.is_empty(), "full access answers by itself");
        // Accept edits answers edits by itself, and still asks before a command.
        let (ev, _) = run(std::slice::from_ref(&req), PermissionMode::AcceptEdits);
        assert!(matches!(&ev[..], [ChatEvent::Approval { .. }]));
        let mut edit = req.clone();
        edit["params"]["toolCall"]["kind"] = json!("edit");
        let (ev, _) = run(&[edit], PermissionMode::AcceptEdits);
        assert!(ev.is_empty(), "accept edits answers edits");
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

    /// Real OpenCode 2 forms (from its question tool and its web search prompt): a choice goes
    /// back as its `const` even when its title says more, typed text as `<key>_custom`.
    #[test]
    fn answers_opencode_2_forms_with_values() {
        let ask = |props: Value| {
            let req = json!({"jsonrpc": "2.0", "id": 9, "method": "elicitation/create", "params": {"mode": "form", "message": "Questions",
                "requestedSchema": {"type": "object", "properties": props}}});
            let (ev, state) = run(&[req], PermissionMode::Ask);
            let q = match &ev[0] {
                ChatEvent::Question { request } => request.clone(),
                e => panic!("{e:?}"),
            };
            let (_, fields) = state.lock().unwrap().questions.get("acpq-9").cloned().unwrap();
            (q, fields)
        };
        let answer = |fields: &[Field], key: &str, a: &[&str]| {
            let mut m = std::collections::BTreeMap::new();
            m.insert(key.to_string(), a.iter().map(|s| s.to_string()).collect());
            elicitation_reply(fields, &m)["content"].clone()
        };

        let (q, f) = ask(json!({"choice": {"type": "string", "description": "Allow OpenCode to search the web?", "oneOf": [
            {"const": "allow", "title": "Allow search via Exa"}, {"const": "disable", "title": "Disable web search"}]}}));
        assert_eq!(q.questions[0].options[0].label, "Allow search via Exa");
        assert_eq!(answer(&f, "choice", &["Disable web search"]), json!({"choice": "disable"}));

        let storage = json!({
            "q0": {"type": "string", "title": "Storage", "description": "Which storage?", "oneOf": [
                {"const": "SQLite", "title": "SQLite", "description": "SQLite"}, {"const": "Postgres", "title": "Postgres", "description": "Small and SQL"}]},
            "q0_custom": {"type": "string", "title": "Storage (other)"}});
        let (q, f) = ask(storage);
        assert_eq!(q.questions.len(), 1, "the _custom field is the question's typed answer, not a question");
        assert!(q.questions[0].other);
        assert_eq!(q.questions[0].options[0].description, "", "a description repeating the title is left out");
        assert_eq!(q.questions[0].options[1].description, "Small and SQL");
        assert_eq!(answer(&f, "q0", &["Postgres"]), json!({"q0": "Postgres"}));
        assert_eq!(answer(&f, "q0", &["DuckDB"]), json!({"q0_custom": "DuckDB"}));
        assert_eq!(answer(&f, "q0", &["SQLite", "and a cache"]), json!({"q0": "SQLite", "q0_custom": "and a cache"}));

        let (_, f) = ask(json!({
            "q1": {"type": "array", "title": "Features", "items": {"anyOf": [{"const": "Search", "title": "Search"}, {"const": "Sync", "title": "Sync"}]}},
            "q1_custom": {"type": "string"}}));
        assert_eq!(answer(&f, "q1", &["Search", "Sync", "Offline"]), json!({"q1": ["Search", "Sync"], "q1_custom": "Offline"}));

        // No choices offered: the typed answer is the answer.
        let (q, f) = ask(json!({"q0": {"type": "string", "title": "Name", "oneOf": []}, "q0_custom": {"type": "string"}}));
        assert!(q.questions[0].options.is_empty() && q.questions[0].other);
        assert_eq!(answer(&f, "q0", &["oah"]), json!({"q0": "oah"}));
    }

    #[test]
    fn asks_first_in_opencode_2_config_too() {
        let var = |k: &str, v: &str| EnvVar { key: k.into(), value: v.into(), in_keychain: false };
        let get = |env: &[EnvVar]| {
            serde_json::from_str::<Value>(&env.iter().find(|e| e.key == "OPENCODE_CONFIG_CONTENT").unwrap().value).unwrap()
        };
        let mut env = vec![];
        with_ask_config(&mut env);
        assert_eq!(get(&env), json!({"permission": {"edit": "ask", "bash": "ask"}}));
        let mut env = vec![var("OPENCODE_CONFIG_CONTENT", r#"{"model":"x/y"}"#)];
        with_ask_config(&mut env);
        assert_eq!(get(&env), json!({"model": "x/y", "permission": {"edit": "ask", "bash": "ask"}}));
        let mut env = vec![var("OPENCODE_CONFIG_CONTENT", r#"{"permission":{"bash":"allow"}}"#)];
        with_ask_config(&mut env);
        assert_eq!(get(&env), json!({"permission": {"bash": "allow"}}), "yours wins");
    }

    #[test]
    fn reads_major_versions() {
        assert_eq!(major_version("2.2.6"), 2);
        assert_eq!(major_version("v1.18.31"), 1);
        assert_eq!(major_version(""), 0);
    }

    /// A reopened session's replay says what it cost so far; the next turn is charged only its own.
    #[test]
    fn cost_after_a_reopen_counts_only_new_turns() {
        let (_, state) = run(
            &[upd(json!({"sessionUpdate": "usage_update", "used": 10, "size": 100, "cost": {"amount": 1.5, "currency": "USD"}}))],
            PermissionMode::Ask,
        );
        assert_eq!(state.lock().unwrap().cost, Some(1.5));
        // Unknown before the turn: no share is claimed.
        let st = State::default();
        assert_eq!(st.cost, None);
    }

    #[test]
    fn compact_summaries_are_not_shown() {
        let state =
            Arc::new(Mutex::new(State { alive: true, turn_id: Some("t".into()), stretch: 1, compacting: true, ..Default::default() }));
        let out = Arc::new(Mutex::new(Vec::new()));
        let o = out.clone();
        let mut tr = Translator { sink: Arc::new(move |e| o.lock().unwrap().push(e)), state, rpc: Rpc::new(None, "OpenCode") };
        tr.handle(&upd(json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Summary of the chat…"}})));
        tr.handle(&upd(json!({"sessionUpdate": "usage_update", "used": 900, "size": 200000})));
        let ev = out.lock().unwrap().clone();
        assert!(!ev.iter().any(|e| matches!(e, ChatEvent::TextDelta { .. })));
        assert!(ev.iter().any(|e| matches!(e, ChatEvent::Context { used_tokens: 900, .. })));
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
        // --verbose: each id followed by its JSON; variant keys are the effort levels.
        let v = parse_opencode_models(
            r#"opencode/beam-free
{
  "id": "beam-free",
  "name": "Beam Free",
  "variants": { "max": {"reasoningEffort": "max"}, "low": {"reasoningEffort": "low"}, "high": {"reasoningEffort": "high"} }
}
opencode/big-pickle
{
  "id": "big-pickle",
  "name": "Big Pickle"
}
"#,
        );
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].name, "Beam Free");
        assert_eq!(v[0].efforts, ["low", "high", "max"]);
        assert!(v[1].efforts.is_empty());
    }

    #[test]
    fn reads_the_effort_option() {
        let session = json!({"configOptions": [
            {"id": "model", "category": "model", "currentValue": "opencode/beam-free", "options": [{"value": "opencode/beam-free", "name": "Beam"}, {"value": "opencode/big-pickle", "name": "Pickle"}]},
            {"id": "effort", "category": "thought_level", "currentValue": "low", "options": [{"value": "low", "name": "Low"}, {"value": "max", "name": "Max"}, {"value": "default", "name": "Default"}]}
        ]});
        let mut st = State::default();
        read_effort(&mut st, &session);
        assert_eq!(st.effort_option.as_deref(), Some("effort"));
        let (mut models, current, _) = models_from(&session);
        add_efforts(&mut models, current.as_deref(), &st.efforts);
        assert_eq!(models[0].efforts, ["low", "max"], "default isn't a level");
        assert!(models[1].efforts.is_empty());
        read_effort(&mut st, &json!({"configOptions": []}));
        assert!(st.effort_option.is_none());
    }

    /// Against a real OpenCode (ignored): OAH_TEST_KIND `opencode` or `opencode2`, OAH_TEST_DIR a
    /// project folder, OAH_TEST_MODEL a (free) model, OAH_TEST_DATA an XDG_DATA_HOME to keep the
    /// test's sessions out of the real ones, OAH_TEST_BINARY another build (a 1.x). The catalog,
    /// a turn, an approval, an effort, /compact and a reopen, each checked.
    #[test]
    #[ignore]
    fn real_opencode_full() {
        use crate::driver::{Driver, Prompt};
        let kind = match std::env::var("OAH_TEST_KIND").as_deref() {
            Ok("opencode2") => AgentKind::Opencode2,
            _ => AgentKind::Opencode,
        };
        let dir = std::env::var("OAH_TEST_DIR").expect("OAH_TEST_DIR");
        let model = std::env::var("OAH_TEST_MODEL").expect("OAH_TEST_MODEL");
        let env: Vec<EnvVar> = std::env::var("OAH_TEST_DATA")
            .ok()
            .map(|d| vec![EnvVar { key: "XDG_DATA_HOME".into(), value: d, in_keychain: false }])
            .unwrap_or_default();
        let opts = |resume: Option<String>| StartOptions {
            effort: None,
            kind,
            cwd: dir.clone(),
            location: oah_core::settings::Location::Local,
            binary: std::env::var("OAH_TEST_BINARY").unwrap_or_default(),
            model: Some(model.clone()),
            permission: PermissionMode::Ask,
            resume,
            env: env.clone(),
        };

        // 1. The catalog for a chat that hasn't started.
        let t0 = std::time::Instant::now();
        let (models, commands) = opencode_catalog(StartOptions { model: None, ..opts(None) }).expect("catalog");
        eprintln!(
            "catalog: {} models in {:?}; commands {:?}",
            models.len(),
            t0.elapsed(),
            commands.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
        for m in models.iter().take(30) {
            eprintln!("  {} ({}) efforts={:?}", m.id, m.name, m.efforts);
        }
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| !m.efforts.is_empty()), "some model has effort levels");
        let v2 = std::env::var("OAH_TEST_BINARY").is_err();
        if v2 {
            assert!(commands.iter().any(|c| c.name == "compact"), "the real commands, not the built-in guess");
        }

        let events = Arc::new(Mutex::new(Vec::<ChatEvent>::new()));
        let start = |resume: Option<String>| {
            let e = events.clone();
            let sink: Sink = Arc::new(move |ev| e.lock().unwrap().push(ev));
            AcpDriver::start(opts(resume), sink).expect("start opencode")
        };
        // Runs until the turn ends, allowing what's asked; the events of the turn.
        let wait_turn = |d: &mut AcpDriver, from: usize| -> Vec<ChatEvent> {
            let t0 = std::time::Instant::now();
            loop {
                assert!(t0.elapsed() < Duration::from_secs(300), "turn took too long");
                thread::sleep(Duration::from_millis(200));
                let evs: Vec<ChatEvent> = events.lock().unwrap()[from..].to_vec();
                for e in &evs {
                    if let ChatEvent::Approval { request } = e {
                        if d.state.lock().unwrap().pending.contains_key(&request.request_id) {
                            eprintln!("  approving: {}", request.title);
                            d.answer(&request.request_id, Decision::Allow).unwrap();
                        }
                    }
                }
                if evs.iter().any(|e| matches!(e, ChatEvent::TurnFinished { .. })) {
                    return evs;
                }
            }
        };
        let send = |d: &mut AcpDriver, text: &str| -> Vec<ChatEvent> {
            let from = events.lock().unwrap().len();
            d.send(Prompt { id: "p".into(), text: text.into(), images: vec![], files: vec![] }).unwrap();
            wait_turn(d, from)
        };
        let text_of = |evs: &[ChatEvent]| -> String {
            evs.iter().filter_map(|e| if let ChatEvent::TextDelta { text, .. } = e { Some(text.as_str()) } else { None }).collect()
        };

        // 2. A turn.
        let mut d = start(None);
        let session = events.lock().unwrap().iter().find_map(|e| match e {
            ChatEvent::Started { session_id, model, .. } => {
                eprintln!("started session={session_id:?} model={model:?}");
                session_id.clone()
            }
            _ => None,
        });
        eprintln!("major version {}", d.state.lock().unwrap().major);
        let evs = send(&mut d, "Reply with exactly the word: pong");
        eprintln!("turn 1: {:?}", text_of(&evs));
        assert!(text_of(&evs).to_lowercase().contains("pong"));
        assert!(evs.iter().any(|e| matches!(e, ChatEvent::Context { .. })), "context reported");
        assert!(evs.iter().any(|e| matches!(e, ChatEvent::TurnFinished { usage: Some(_), .. })), "usage reported");

        // 3. A command asks first in Ask mode (oah's OPENCODE_PERMISSION).
        let evs = send(&mut d, "Run the shell command `echo oah-$((6*7))` with your bash tool and tell me its output.");
        let asked = evs.iter().any(|e| matches!(e, ChatEvent::Approval { .. }));
        eprintln!("turn 2 asked={asked}: {:?}", text_of(&evs));
        assert!(asked, "a command asks in Ask mode");
        assert!(text_of(&evs).contains("oah-42"));

        // 4. An effort the model has.
        let efforts = d.state.lock().unwrap().efforts.clone();
        eprintln!("efforts for {model}: {efforts:?}");
        if let Some(e) = efforts.iter().find(|e| *e != "default") {
            let from = events.lock().unwrap().len();
            assert!(d.set_effort(e).unwrap());
            let picked = events.lock().unwrap()[from..].iter().any(|ev| matches!(ev, ChatEvent::Models { .. }));
            eprintln!("set effort {e}: models re-read = {picked}");
        }

        // 5. /compact: a note, not the summary.
        let from = events.lock().unwrap().len();
        assert!(d.compact("").unwrap());
        let evs = wait_turn(&mut d, from);
        let note = evs.iter().any(|e| matches!(e, ChatEvent::Notice { text, .. } if text == "Context compacted."));
        eprintln!("compact: note={note}, text shown={:?}", text_of(&evs));
        assert!(note && text_of(&evs).is_empty());
        assert!(d.compact("focus on tests").is_err(), "no instructions");
        let cost_before = d.state.lock().unwrap().cost;
        d.stop();

        // 6. Reopen: the earlier turns are known, the next one is charged only its own cost.
        let mut d = start(session.clone());
        eprintln!("reopened: cost so far {:?} (before close {cost_before:?})", d.state.lock().unwrap().cost);
        let evs = send(&mut d, "What exact word did you reply with the first time? One word.");
        eprintln!("after reopen: {:?}", text_of(&evs));
        assert!(text_of(&evs).to_lowercase().contains("pong"));
        let turn_cost = evs.iter().find_map(|e| if let ChatEvent::TurnFinished { cost_usd, .. } = e { Some(*cost_usd) } else { None });
        eprintln!("turn cost after reopen: {turn_cost:?}");
        d.stop();
    }

    /// Against a real OpenCode (ignored): OAH_TEST_DIR is a project folder, OAH_TEST_MODEL an
    /// optional model. Runs a turn, answers approvals, then reopens the session and asks about it.
    #[test]
    #[ignore]
    fn real_opencode_turn_and_resume() {
        use crate::driver::{Driver, Prompt};
        let dir = std::env::var("OAH_TEST_DIR").expect("OAH_TEST_DIR");
        let model = std::env::var("OAH_TEST_MODEL").ok();
        let events = Arc::new(Mutex::new(Vec::<ChatEvent>::new()));
        let start = |resume: Option<String>| {
            let e = events.clone();
            let sink: Sink = Arc::new(move |ev| e.lock().unwrap().push(ev));
            let opts = StartOptions {
                effort: None,
                kind: AgentKind::Opencode,
                cwd: dir.clone(),
                location: oah_core::settings::Location::Local,
                binary: String::new(),
                model: model.clone(),
                permission: PermissionMode::Ask,
                resume,
                env: vec![],
            };
            AcpDriver::start(opts, sink).expect("start opencode")
        };
        let run = |d: &mut AcpDriver, text: &str| -> String {
            let from = events.lock().unwrap().len();
            d.send(Prompt { id: "p".into(), text: text.into(), images: vec![], files: vec![] }).unwrap();
            let t0 = std::time::Instant::now();
            loop {
                assert!(t0.elapsed() < Duration::from_secs(300), "turn took too long");
                thread::sleep(Duration::from_millis(200));
                let evs: Vec<ChatEvent> = events.lock().unwrap()[from..].to_vec();
                for e in &evs {
                    if let ChatEvent::Approval { request } = e {
                        if d.state.lock().unwrap().pending.contains_key(&request.request_id) {
                            eprintln!("  approving: {}", request.title);
                            d.answer(&request.request_id, Decision::Allow).unwrap();
                        }
                    }
                }
                if evs.iter().any(|e| matches!(e, ChatEvent::TurnFinished { .. })) {
                    let mut text = String::new();
                    for e in &evs {
                        match e {
                            ChatEvent::TextDelta { text: t, .. } => text.push_str(t),
                            ChatEvent::ToolStarted { tool, .. } => eprintln!("  tool: {:?} {}", tool.kind, tool.title),
                            ChatEvent::ToolFinished { ok, .. } => eprintln!("  tool done ok={ok}"),
                            ChatEvent::Context { used_tokens, window_tokens, .. } => eprintln!("  context {used_tokens}/{window_tokens}"),
                            ChatEvent::TurnFinished { usage, cost_usd, .. } => eprintln!("  finished usage={usage:?} cost={cost_usd:?}"),
                            ChatEvent::Error { message } => eprintln!("  error: {message}"),
                            _ => {}
                        }
                    }
                    return text;
                }
            }
        };
        let mut d = start(None);
        let session = events.lock().unwrap().iter().find_map(|e| match e {
            ChatEvent::Started { session_id, model, .. } => {
                eprintln!("started session={session_id:?} model={model:?}");
                session_id.clone()
            }
            _ => None,
        });
        let models =
            events.lock().unwrap().iter().find_map(|e| if let ChatEvent::Models { models, .. } = e { Some(models.len()) } else { None });
        eprintln!("models offered: {models:?}");
        let a = run(&mut d, "Read src/cart.js and tell me in one sentence what subtotal() does. Don't change any files.");
        eprintln!("answer 1: {a}");
        assert!(!a.trim().is_empty());
        d.stop();
        drop(d);
        let mut d2 = start(session);
        let b = run(&mut d2, "Which file did you just read? Answer with only the path.");
        eprintln!("answer 2 (resumed): {b}");
        assert!(b.contains("cart"), "the resumed session remembers: {b}");
        d2.stop();
    }

    /// Against a real `agy` (ignored): OAH_TEST_DIR a folder, OAH_TEST_RESUME a conversation to
    /// reopen (so no new one is left in its history). Checks the session's models, efforts and
    /// commands, switches model, effort and mode, then runs one cheap turn that asks to run a
    /// command (Ask mode: an approval card) and reads its output.
    #[test]
    #[ignore]
    fn real_antigravity() {
        use crate::driver::{Driver, Prompt};
        let dir = std::env::var("OAH_TEST_DIR").expect("OAH_TEST_DIR");
        let events = Arc::new(Mutex::new(Vec::<ChatEvent>::new()));
        let e = events.clone();
        let sink: Sink = Arc::new(move |ev| e.lock().unwrap().push(ev));
        let opts = StartOptions {
            kind: AgentKind::Antigravity,
            cwd: dir.clone(),
            location: oah_core::settings::Location::Local,
            binary: String::new(),
            model: Some("gemini-3.8-flash".into()),
            effort: Some("low".into()),
            permission: PermissionMode::Ask,
            resume: std::env::var("OAH_TEST_RESUME").ok(),
            env: vec![],
        };
        let mut d = AcpDriver::start(opts, sink).expect("start agy");
        thread::sleep(Duration::from_millis(800));
        {
            let evs = events.lock().unwrap();
            for e in evs.iter() {
                match e {
                    ChatEvent::Started { session_id, model, .. } => eprintln!("started session={session_id:?} model={model:?}"),
                    ChatEvent::Models { models, current } => eprintln!(
                        "models (current {current:?}): {}",
                        models.iter().map(|m| format!("{} {:?}", m.id, m.efforts)).collect::<Vec<_>>().join(", ")
                    ),
                    ChatEvent::Commands { commands } => {
                        eprintln!("commands: {}", commands.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(" "))
                    }
                    ChatEvent::Notice { text, .. } => eprintln!("notice: {text}"),
                    _ => {}
                }
            }
            let models = evs
                .iter()
                .rev()
                .find_map(|e| if let ChatEvent::Models { models, .. } = e { Some(models.clone()) } else { None })
                .expect("models");
            assert!(models.iter().any(|m| m.efforts.len() > 1), "efforts per model");
        }
        let st = || {
            let s = d.state.lock().unwrap();
            (s.current_model.clone(), s.efforts.clone(), s.resumable)
        };
        eprintln!("after start: {:?}", st());
        assert_eq!(st().0.as_deref(), Some("gemini-3.8-flash"));
        assert!(st().2, "resumable");
        d.set_permission(PermissionMode::Plan).expect("plan mode");
        d.set_permission(PermissionMode::AcceptEdits).expect("accept-edits mode");
        d.set_permission(PermissionMode::Ask).expect("default mode");

        let from = events.lock().unwrap().len();
        d.send(Prompt {
            id: "p1".into(),
            text: "Run the shell command `echo oah-agy-check` and tell me exactly what it printed. Nothing else.".into(),
            images: vec![],
            files: vec![],
        })
        .unwrap();
        let t0 = std::time::Instant::now();
        let mut approvals = 0;
        loop {
            assert!(t0.elapsed() < Duration::from_secs(240), "turn took too long");
            thread::sleep(Duration::from_millis(200));
            let evs: Vec<ChatEvent> = events.lock().unwrap()[from..].to_vec();
            for e in &evs {
                if let ChatEvent::Approval { request } = e {
                    if d.state.lock().unwrap().pending.contains_key(&request.request_id) {
                        approvals += 1;
                        eprintln!("  approval: {:?} {} — {:?}", request.kind, request.title, request.detail);
                        d.answer(&request.request_id, Decision::Allow).unwrap();
                    }
                }
                if let ChatEvent::Question { request } = e {
                    if d.state.lock().unwrap().choices.contains_key(&request.request_id) {
                        eprintln!("  question: {:?}", request.questions[0]);
                        let first = request.questions[0].options[0].label.clone();
                        d.answer_question(&request.request_id, [("answer".to_string(), vec![first])].into_iter().collect()).unwrap();
                    }
                }
            }
            if evs.iter().any(|e| matches!(e, ChatEvent::TurnFinished { .. })) {
                let mut text = String::new();
                for e in &evs {
                    match e {
                        ChatEvent::TextDelta { text: t, .. } => text.push_str(t),
                        ChatEvent::ReasoningDelta { .. } => {}
                        ChatEvent::ToolStarted { tool, .. } => eprintln!("  tool: {:?} {} {}", tool.kind, tool.title, tool.input),
                        ChatEvent::ToolFinished { ok, output, .. } => eprintln!("  tool done ok={ok} output={output:?}"),
                        ChatEvent::Context { used_tokens, window_tokens, .. } => eprintln!("  context {used_tokens}/{window_tokens}"),
                        ChatEvent::TurnFinished { usage, cost_usd, interrupted, .. } => {
                            eprintln!("  finished usage={usage:?} cost={cost_usd:?} interrupted={interrupted}")
                        }
                        ChatEvent::Error { message } => eprintln!("  error: {message}"),
                        ChatEvent::Notice { text, .. } => eprintln!("  notice: {text}"),
                        _ => {}
                    }
                }
                eprintln!("answer: {text}");
                assert!(text.contains("oah-agy-check"), "the command's output comes back: {text}");
                break;
            }
        }
        eprintln!("approvals asked: {approvals}");
        d.stop();
    }
    /// Grok 1.0.77: its question tool, a finished plan, mode changes it makes itself, tool kinds
    /// from `_meta`, and the commands oah leaves out.
    #[test]
    fn grok_questions_plans_and_modes() {
        let meta = |name: &str, kind: &str| json!({"x.ai/tool": {"version": 1, "name": name, "kind": kind, "namespace": "grok_build"}});
        let ask_call = upd(
            json!({"sessionUpdate": "tool_call", "toolCallId": "call-a-0", "title": "ask_user_question", "rawInput": {}, "_meta": meta("ask_user_question", "ask_user")}),
        );
        let ask_done = upd(json!({"sessionUpdate": "tool_call_update", "toolCallId": "call-a-0", "status": "completed", "content": []}));
        let ask = json!({"jsonrpc": "2.0", "id": 0, "method": "_x.ai/ask_user_question", "params": {"sessionId": "s", "toolCallId": "call-a-0", "mode": "default",
            "questions": [{"question": "What is your favorite color?", "options": [{"label": "red", "description": "Red"}, {"label": "blue", "description": "Warm"}], "multiSelect": null},
                          {"question": "Which sizes?", "options": [{"label": "S"}, {"label": "M"}], "multiSelect": true}]}});
        let shell = upd(
            json!({"sessionUpdate": "tool_call", "toolCallId": "call-b-0", "title": "run_terminal_command", "rawInput": {"command": "ls"}, "_meta": meta("run_terminal_command", "execute")}),
        );
        let into_plan = upd(json!({"sessionUpdate": "current_mode_update", "currentModeId": "plan"}));
        let plan = json!({"jsonrpc": "2.0", "id": 1, "method": "_x.ai/exit_plan_mode", "params": {"sessionId": "s", "toolCallId": "call-c-2", "planContent": "1. Say done.\n"}});
        let out_of_plan = upd(json!({"sessionUpdate": "current_mode_update", "currentModeId": "default"}));
        let cmds = upd(json!({"sessionUpdate": "available_commands_update", "availableCommands": [
            {"name": "compact", "description": "Compress", "input": null}, {"name": "always-approve", "description": "Toggle", "input": {"hint": "on|off"}}]}));
        let (ev, state) = run_as(
            Some(AgentKind::Grok),
            &[ask_call, ask, ask_done, shell, into_plan, plan, out_of_plan, cmds],
            PermissionMode::FullAccess,
        );

        // The question: a card per question, no tool row, never answered by the mode.
        let q = ev.iter().find_map(|e| if let ChatEvent::Question { request } = e { Some(request.clone()) } else { None }).unwrap();
        assert_eq!(q.questions.len(), 2);
        assert_eq!(
            (q.questions[0].question.as_str(), q.questions[0].options[0].description.as_str()),
            ("What is your favorite color?", "")
        );
        assert_eq!(q.questions[0].options[1].description, "Warm");
        assert!(q.questions[1].multi && q.questions[0].other);
        assert!(state.lock().unwrap().asks.contains_key(&q.request_id));
        let tools: Vec<&ToolCall> =
            ev.iter().filter_map(|e| if let ChatEvent::ToolStarted { tool, .. } = e { Some(tool) } else { None }).collect();
        assert_eq!(tools.len(), 1, "only the shell command has a row");
        assert_eq!((tools[0].kind, tools[0].title.as_str()), (ToolKind::Bash, "ls"));
        let answers =
            [("q0".to_string(), vec!["blue".to_string()]), ("q1".to_string(), vec!["S".into(), "M".into()])].into_iter().collect();
        let asked = state.lock().unwrap().asks[&q.request_id].1.clone();
        assert_eq!(
            ask_reply(&asked, &answers),
            json!({"outcome": "accepted", "answers": {"What is your favorite color?": "blue", "Which sizes?": ["S", "M"]}})
        );
        assert_eq!(ask_reply(&asked, &Default::default()), json!({"outcome": "cancelled"}));

        // Plan mode, by Grok's own doing: the chat follows in, then back to where it was.
        let modes: Vec<PermissionMode> =
            ev.iter().filter_map(|e| if let ChatEvent::PermissionChanged { mode } = e { Some(*mode) } else { None }).collect();
        assert_eq!(modes, [PermissionMode::Plan, PermissionMode::FullAccess]);
        // The finished plan is the user's to approve, even with full access.
        let p = ev
            .iter()
            .find_map(|e| match e {
                ChatEvent::Approval { request } if request.kind == ApprovalKind::Plan => Some(request.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(p.detail.as_deref(), Some("1. Say done.\n"));
        assert!(state.lock().unwrap().plans.contains_key(&p.request_id));

        let cmds =
            ev.iter().rev().find_map(|e| if let ChatEvent::Commands { commands } = e { Some(commands.clone()) } else { None }).unwrap();
        assert_eq!(cmds.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["compact"]);
        assert_eq!(agent_mode(Some(AgentKind::Grok), PermissionMode::Plan), "plan");
        assert_eq!(agent_mode(Some(AgentKind::Grok), PermissionMode::AcceptEdits), "default");
    }

    /// Grok's session `models` list every model's efforts in `_meta`; the config option only names them.
    #[test]
    fn grok_models_have_their_own_efforts() {
        let session = json!({
            "models": {"currentModelId": "grok-4.7", "availableModels": [
                {"modelId": "grok-4.7", "name": "Grok 4.7", "_meta": {"totalContextTokens": 256000, "reasoningEfforts": [{"id": "high", "value": "high", "default": true}, {"id": "low", "value": "low"}]}},
                {"modelId": "grok-4.5", "name": "Grok 4.5", "_meta": {"totalContextTokens": 128000, "reasoningEfforts": [{"id": "medium", "value": "medium", "default": true}, {"id": "low", "value": "low"}]}}]},
            "configOptions": [
                {"id": "model", "category": "model", "type": "select", "currentValue": "grok-4.7", "options": [{"value": "grok-4.7", "name": "Grok 4.7"}, {"value": "grok-4.5", "name": "Grok 4.5"}]},
                {"id": "reasoning_effort", "category": "thought_level", "type": "select", "currentValue": "high", "options": [{"value": "high", "name": "High"}, {"value": "low", "name": "Low"}]}]
        });
        let sink: Sink = Arc::new(|_| {});
        let d = AcpDriver {
            rpc: Rpc::new(None, "Grok"),
            child: Arc::new(Mutex::new(None)),
            state: Arc::new(Mutex::new(State { kind: Some(AgentKind::Grok), ..Default::default() })),
            sink,
            queue: Arc::new(Mutex::new(VecDeque::new())),
            permission: PermissionMode::Ask,
        };
        let (models, current) = d.read_options(&session);
        assert_eq!(current.as_deref(), Some("grok-4.7"));
        assert_eq!(models[1].efforts, ["low", "medium"]);
        assert_eq!(models[1].default_effort.as_deref(), Some("medium"));
        let st = d.state.lock().unwrap();
        assert_eq!(st.modes, ["default", "plan"]);
        assert_eq!(st.windows.get("grok-4.5"), Some(&128000));
    }

    /// Against a real `grok` (ignored): OAH_TEST_DIR a folder, OAH_TEST_RESUME a session to reopen.
    /// Checks the session's models and efforts, switches effort and mode, runs a turn that asks
    /// to run a command (Ask mode: an approval card), reads its output, cost and context; then a
    /// one-off answer, whose session is deleted.
    #[test]
    #[ignore]
    fn real_grok() {
        use crate::driver::{Driver, Prompt};
        let dir = std::env::var("OAH_TEST_DIR").expect("OAH_TEST_DIR");
        let events = Arc::new(Mutex::new(Vec::<ChatEvent>::new()));
        let e = events.clone();
        let sink: Sink = Arc::new(move |ev| e.lock().unwrap().push(ev));
        let opts = StartOptions {
            kind: AgentKind::Grok,
            cwd: dir.clone(),
            location: oah_core::settings::Location::Local,
            binary: String::new(),
            model: None,
            effort: Some("low".into()),
            permission: PermissionMode::Ask,
            resume: std::env::var("OAH_TEST_RESUME").ok(),
            env: vec![],
        };
        let mut d = AcpDriver::start(opts.clone(), sink).expect("start grok");
        thread::sleep(Duration::from_millis(800));
        {
            let evs = events.lock().unwrap();
            let models = evs
                .iter()
                .rev()
                .find_map(|e| if let ChatEvent::Models { models, .. } = e { Some(models.clone()) } else { None })
                .expect("models");
            eprintln!("models: {}", models.iter().map(|m| format!("{} {:?}", m.id, m.efforts)).collect::<Vec<_>>().join(", "));
            assert!(models.iter().all(|m| m.efforts.len() > 1), "efforts for every model");
            if let Some(c) =
                evs.iter().rev().find_map(|e| if let ChatEvent::Commands { commands } = e { Some(commands.clone()) } else { None })
            {
                eprintln!("commands: {}", c.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(" "));
                assert!(!c.iter().any(|c| c.name == "always-approve"));
            }
        }
        {
            let s = d.state.lock().unwrap();
            eprintln!("efforts {:?} modes {:?} resumable {}", s.efforts, s.modes, s.resumable);
            assert!(s.resumable);
        }
        d.set_permission(PermissionMode::Plan).expect("plan mode");
        d.set_permission(PermissionMode::Ask).expect("default mode");
        let from = events.lock().unwrap().len();
        d.send(Prompt {
            id: "p1".into(),
            text: "Run the shell command `echo oah-grok-check` and tell me exactly what it printed. Nothing else.".into(),
            images: vec![],
            files: vec![],
        })
        .unwrap();
        let t0 = std::time::Instant::now();
        loop {
            assert!(t0.elapsed() < Duration::from_secs(240), "turn took too long");
            thread::sleep(Duration::from_millis(200));
            let evs: Vec<ChatEvent> = events.lock().unwrap()[from..].to_vec();
            for e in &evs {
                if let ChatEvent::Approval { request } = e {
                    if d.state.lock().unwrap().pending.contains_key(&request.request_id) {
                        eprintln!("  approval: {:?} {} — {:?}", request.kind, request.title, request.detail);
                        d.answer(&request.request_id, Decision::Allow).unwrap();
                    }
                }
            }
            if evs.iter().any(|e| matches!(e, ChatEvent::TurnFinished { .. })) {
                let mut text = String::new();
                let mut context = false;
                for e in &evs {
                    match e {
                        ChatEvent::TextDelta { text: t, .. } => text.push_str(t),
                        ChatEvent::ToolStarted { tool, .. } => eprintln!("  tool: {:?} {}", tool.kind, tool.title),
                        ChatEvent::ToolFinished { ok, output, .. } => eprintln!("  tool done ok={ok} output={output:?}"),
                        ChatEvent::Context { used_tokens, window_tokens, .. } => {
                            context = true;
                            eprintln!("  context {used_tokens}/{window_tokens}")
                        }
                        ChatEvent::TurnFinished { usage, cost_usd, interrupted, .. } => {
                            eprintln!("  finished usage={usage:?} cost={cost_usd:?} interrupted={interrupted}");
                            assert!(usage.as_ref().is_some_and(|u| u.input_tokens > 0) && cost_usd.is_some());
                        }
                        ChatEvent::Error { message } => eprintln!("  error: {message}"),
                        _ => {}
                    }
                }
                eprintln!("answer: {text}");
                assert!(text.contains("oah-grok-check"), "the command's output comes back: {text}");
                assert!(context, "context after the turn");
                break;
            }
        }
        d.stop();
        let answer = ask_once(opts, "Reply with just the word: pong", Duration::from_secs(120)).expect("one-off");
        eprintln!("one-off: {answer}");
        assert!(answer.to_lowercase().contains("pong"));
    }

    #[test]
    fn tool_calls_left_open_end_as_background_tasks() {
        let lines = [
            upd(
                json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "kind": "execute", "title": "npm run dev", "status": "in_progress"}),
            ),
            upd(json!({"sessionUpdate": "tool_call", "toolCallId": "c2", "kind": "read", "title": "a.txt", "status": "in_progress"})),
            upd(json!({"sessionUpdate": "tool_call_update", "toolCallId": "c2", "status": "completed"})),
        ];
        let out = Arc::new(Mutex::new(Vec::new()));
        let o = out.clone();
        let sink: Sink = Arc::new(move |e| o.lock().unwrap().push(e));
        let state =
            Arc::new(Mutex::new(State { alive: true, busy: true, turn_id: Some("turn-1".into()), stretch: 1, ..Default::default() }));
        let mut tr = Translator { sink, state: state.clone(), rpc: Rpc::new(None, "OpenCode") };
        for l in &lines {
            tr.handle(l);
        }
        // Only the one still running is open.
        assert_eq!(state.lock().unwrap().open.keys().collect::<Vec<_>>(), ["c1"]);
        // Say the turn ended with it open: it's a task, and its later update ends it.
        {
            let mut s = state.lock().unwrap();
            let call = s.open.remove("c1").unwrap();
            let mut t = BackgroundTask::running("c1", TaskKind::Command, call.title);
            t.item_id = Some("c1".into());
            s.tasks.add(t);
        }
        tr.handle(&upd(json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "completed", "rawOutput": {"combinedOutput": "ready\nlistening on :3000\n"}})));
        let ev = out.lock().unwrap().clone();
        let Some(ChatEvent::Tasks { tasks }) = ev.iter().rev().find(|e| matches!(e, ChatEvent::Tasks { .. })) else { panic!("no tasks") };
        assert_eq!((tasks[0].status, tasks[0].detail.as_deref()), (TaskStatus::Completed, Some("listening on :3000")));
        assert!(matches!(ev.last(), Some(ChatEvent::Notice { text, .. }) if text.contains("npm run dev")));
    }

    /// `OAH_TEST_DIR=/tmp/x cargo test -p oah-agents real_antigravity_background -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_antigravity_background() {
        use crate::driver::{Driver, Prompt};
        let dir = std::env::var("OAH_TEST_DIR").expect("OAH_TEST_DIR");
        let events = Arc::new(Mutex::new(Vec::<ChatEvent>::new()));
        let e = events.clone();
        let sink: Sink = Arc::new(move |ev| e.lock().unwrap().push(ev));
        let opts = StartOptions {
            kind: AgentKind::Antigravity,
            cwd: dir,
            location: oah_core::settings::Location::Local,
            binary: String::new(),
            model: Some("gemini-3.8-flash".into()),
            effort: Some("low".into()),
            permission: PermissionMode::FullAccess,
            resume: None,
            env: vec![],
        };
        let mut d = AcpDriver::start(opts, sink).expect("start agy");
        let text = "Run this shell command in the background (don't wait for it): sleep 30; echo bg-finished > done.txt . Then immediately reply 'started' and end your turn. Do not check on it.";
        d.send(Prompt { id: "p1".into(), text: text.into(), images: vec![], files: vec![] }).unwrap();
        for _ in 0..240 {
            if events.lock().unwrap().iter().any(|e| matches!(e, ChatEvent::TurnFinished { .. })) {
                break;
            }
            thread::sleep(Duration::from_millis(500));
        }
        let evs = events.lock().unwrap().clone();
        for e in &evs {
            if matches!(
                e,
                ChatEvent::Tasks { .. } | ChatEvent::TurnFinished { .. } | ChatEvent::ToolStarted { .. } | ChatEvent::ToolFinished { .. }
            ) {
                eprintln!("{}", serde_json::to_string(e).unwrap());
            }
        }
        let tasks = evs.iter().find_map(|e| if let ChatEvent::Tasks { tasks } = e { Some(tasks.clone()) } else { None }).expect("a task");
        assert_eq!((tasks[0].kind, tasks[0].status, tasks[0].tracked), (TaskKind::Command, TaskStatus::Running, false));
        assert_eq!(d.background(), 0, "an untracked task doesn't keep the chat awake");
        d.stop();
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
