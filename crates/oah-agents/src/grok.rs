//! xAI's Grok Build CLI (`grok`): what differs from the other ACP agents.
//!
//! - Chats run over ACP (`grok agent stdio`, see [`crate::acp`]). Sessions open with
//!   `_meta.askUserQuestion`, so its question tool asks through `_x.ai/ask_user_question`; its plan
//!   mode is the ACP mode `plan`, and a finished plan asks through `_x.ai/exit_plan_mode`.
//! - Models and commands for a chat that hasn't started: `initialize`'s `_meta.modelState` (every
//!   model with its efforts) and `_meta.availableCommands`. No session: `grok` keeps every one.
//! - Sign-in: `grok login` in a terminal (`--device-auth` in WSL, where no browser opens). Who: the
//!   email in `~/.grok/auth.json`; nothing else is read out of it, and the tokens never leave it.
//!   `XAI_API_KEY` set for it reads as "API key".
//! - Limits: `_x.ai/billing`, what `/usage` shows: the share of the plan's credits used this
//!   period (a week on SuperGrok), and when the period ends.
//! - One-off questions: a session at the lowest effort, deleted afterwards (see
//!   [`crate::acp::ask_once`]); the question can be longer than a command line takes.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use oah_core::settings::{EnvVar, Location};
use oah_transport::Spec;
use serde_json::{json, Value};

use crate::account::{Account, LimitExtra, LimitWindow, Limits};
use crate::driver::{DriverResult, StartOptions};
use crate::event::{ChatEvent, ModelOption, SlashCommand, ToolCall, ToolKind};
use crate::jsonrpc::Rpc;

pub const NOT_SIGNED_IN: &str = "Grok isn't signed in. Run `grok login` in a terminal (Settings → Providers → Grok Build → Sign in).";

fn var(key: &str, value: &str) -> EnvVar {
    EnvVar { key: key.into(), value: value.into(), in_keychain: false }
}

/// The environment every `grok` run gets: no self-update mid-run. Yours wins when set.
pub fn acp_env(env: &mut Vec<EnvVar>) {
    if !env.iter().any(|e| e.key == "GROK_DISABLE_AUTOUPDATER") {
        env.push(var("GROK_DISABLE_AUTOUPDATER", "1"));
    }
}

/// `grok agent stdio`: ACP on stdin and stdout.
pub fn acp_args() -> Vec<String> {
    vec!["agent".into(), "stdio".into()]
}

/// What a session opens with: questions come to oah as `_x.ai/ask_user_question`.
pub fn session_meta() -> Value {
    json!({"askUserQuestion": true})
}

fn program(binary: &str) -> &str {
    if binary.trim().is_empty() {
        "grok"
    } else {
        binary.trim()
    }
}

fn home_cwd(location: &Location) -> String {
    if matches!(location, Location::Local) {
        oah_core::paths::home_dir().to_string_lossy().into_owned()
    } else {
        "~".into()
    }
}

/// Runs `f` against a `grok agent stdio` that has answered `initialize` (its result is the second
/// argument), then stops it. No session is opened.
fn with_agent<T>(binary: &str, location: &Location, env: &[EnvVar], f: impl FnOnce(&Rpc, &Value) -> DriverResult<T>) -> DriverResult<T> {
    let prog = program(binary);
    let cwd = home_cwd(location);
    let mut env = env.to_vec();
    acp_env(&mut env);
    let mut child = oah_transport::spawn(&Spec { location, cwd: &cwd, program: prog, args: acp_args(), env: &env })
        .map_err(|e| format!("couldn't start {prog}: {e}"))?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let rpc = Rpc::new(child.stdin.take(), "Grok");
    {
        let rpc = rpc.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    // Its own requests (none expected without a session) are refused.
                    if !rpc.take_reply(&v) && v.get("id").is_some() && v.get("method").is_some() {
                        let _ = rpc.reply_error(&v["id"], -32601, "oah doesn't offer this");
                    }
                }
            }
            rpc.fail_all();
        });
    }
    let result = rpc
        .call(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false},
                "clientInfo": {"name": "oah", "title": "oah", "version": env!("CARGO_PKG_VERSION")}
            }),
            Duration::from_secs(60),
        )
        .and_then(|init| f(&rpc, &init));
    rpc.stdin.lock().unwrap().take();
    let _ = child.kill();
    let _ = child.wait();
    result
}

// ---- Saved conversations ---------------------------------------------------------------------

/// What was said in a session after the first `skip` lines of its history
/// (`sessions/<folder>/<session>/chat_history.jsonl`, one message a line): the events a driver
/// would have reported for the same turns, for a chat that was open in Grok Build's own
/// interface meanwhile. `tag` keeps their ids apart from every other turn's in the chat.
pub fn history_events(text: &str, skip: usize, tag: &str) -> Vec<ChatEvent> {
    let mut out = Vec::new();
    let mut turn: Option<String> = None;
    let finish = |out: &mut Vec<ChatEvent>, turn: &mut Option<String>| {
        if let Some(turn_id) = turn.take() {
            out.push(ChatEvent::TurnFinished { turn_id, usage: None, cost_usd: None, duration_ms: None, interrupted: false });
        }
    };
    // A message's text: as it is, or its text parts.
    let content = |v: &Value| match &v["content"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    };
    for (n, v) in text.lines().enumerate().skip(skip).filter_map(|(n, l)| Some((n, serde_json::from_str::<Value>(l).ok()?))) {
        let id = format!("cli-{tag}-{n}");
        match v["type"].as_str() {
            Some("user") => {
                // What the user typed is wrapped in `<user_query>`; the other user lines are
                // what Grok tells the model itself (its rules, reminders).
                let all = content(&v);
                let said = match all.split_once("<user_query>").and_then(|(_, rest)| rest.rsplit_once("</user_query>")) {
                    Some((query, _)) => query.trim(),
                    None if v["synthetic_reason"].is_null() && !all.trim_start().starts_with('<') => all.trim(),
                    None => continue,
                };
                if said.is_empty() {
                    continue;
                }
                finish(&mut out, &mut turn);
                out.push(ChatEvent::UserMessage { id: id.clone(), text: said.to_string(), images: vec![], files: vec![] });
                out.push(ChatEvent::TurnStarted { turn_id: id.clone() });
                turn = Some(id);
            }
            _ if turn.is_none() => {}
            Some("reasoning") => {
                let t = v["summary"].as_array().into_iter().flatten().filter_map(|s| s["text"].as_str()).collect::<Vec<_>>().join("\n");
                if !t.trim().is_empty() {
                    out.push(ChatEvent::Reasoning { item_id: id, text: t });
                }
            }
            Some("assistant") => {
                let t = content(&v);
                if !t.trim().is_empty() {
                    out.push(ChatEvent::Text { item_id: id, text: t });
                }
                for call in v["tool_calls"].as_array().into_iter().flatten() {
                    let name = call["name"].as_str().unwrap_or("tool");
                    // Its arguments are JSON in a string.
                    let input = call["arguments"].as_str().and_then(|a| serde_json::from_str::<Value>(a).ok()).unwrap_or(Value::Null);
                    let s = |k: &str| input[k].as_str().unwrap_or("").to_string();
                    let path = ["target_file", "file_path", "path"].iter().map(|k| s(k)).find(|p| !p.is_empty());
                    let kind = match name {
                        "run_terminal_command" => ToolKind::Bash,
                        n if n.contains("read") => ToolKind::Read,
                        n if n.contains("edit") || n.contains("replace") => ToolKind::Edit,
                        n if n.contains("write") || n.contains("create") => ToolKind::Write,
                        n if n.contains("search") || n.contains("grep") || n.contains("glob") || n.contains("list") => ToolKind::Search,
                        n if n.contains("web") || n.contains("fetch") => ToolKind::Web,
                        _ => ToolKind::Other,
                    };
                    let title = [s("command"), path.clone().unwrap_or_default(), s("pattern"), s("query"), s("url")]
                        .into_iter()
                        .find(|t| !t.is_empty())
                        .unwrap_or_else(|| name.to_string());
                    let tool = ToolCall { kind, name: name.into(), title, input, path };
                    out.push(ChatEvent::ToolStarted { item_id: call["id"].as_str().unwrap_or("").into(), tool });
                }
            }
            Some("tool_result") => {
                // A command's result starts with how it ended (`exit: 0`).
                let all = content(&v);
                let (ok, output) = match all.strip_prefix("exit: ").and_then(|rest| rest.split_once('\n')) {
                    Some((code, output)) => (code.trim() == "0", output.to_string()),
                    None => (true, all.clone()),
                };
                let mut output = output.trim_end().to_string();
                if output.len() > 16 * 1024 {
                    let mut cut = 16 * 1024;
                    while !output.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    output.truncate(cut);
                    output.push_str("\n…");
                }
                out.push(ChatEvent::ToolFinished { item_id: v["tool_call_id"].as_str().unwrap_or("").into(), ok, output, diff: None });
            }
            _ => {}
        }
    }
    finish(&mut out, &mut turn);
    out
}

// ---- Sign-in ---------------------------------------------------------------------------------

/// The terminal command that signs in. In WSL a browser may not open, so it shows a code to
/// enter on any device instead.
pub fn login_line(prog: &str, location: &Location) -> String {
    match location {
        Location::Wsl { .. } => format!("{prog} login --device-auth"),
        _ => format!("{prog} login"),
    }
}

/// Who `grok` is signed in as on `location`: the email in its `auth.json`, or an API key.
pub fn account(location: &Location, env: &[EnvVar]) -> Account {
    let api_key = env.iter().any(|e| e.key == "XAI_API_KEY" && !e.value.trim().is_empty());
    let found = match location {
        Location::Local => {
            let home = env
                .iter()
                .find(|e| e.key == "GROK_HOME" && !e.value.trim().is_empty())
                .map(|e| std::path::PathBuf::from(e.value.trim()))
                .unwrap_or_else(|| oah_core::paths::home_dir().join(".grok"));
            match std::fs::read_to_string(home.join("auth.json")) {
                Ok(s) => Ok(signed_in_as(&s)),
                Err(_) if api_key || std::env::var("XAI_API_KEY").is_ok_and(|k| !k.trim().is_empty()) => Ok(Some(Found::ApiKey)),
                Err(_) => Ok(None),
            }
        }
        remote => remote_account(remote, env),
    };
    match found {
        Ok(Some(Found::Account(email))) => Account { signed_in: true, label: email.or(Some("Grok".into())), plan: None, error: None },
        Ok(Some(Found::ApiKey)) => Account { signed_in: true, label: Some("API key".into()), plan: None, error: None },
        Ok(None) if api_key => Account { signed_in: true, label: Some("API key".into()), plan: None, error: None },
        Ok(None) => Account { signed_in: false, label: None, plan: None, error: None },
        Err(e) => Account { error: Some(e), ..Default::default() },
    }
}

#[derive(Debug, PartialEq)]
enum Found {
    /// A grok.com sign-in, and its email when the file names one.
    Account(Option<String>),
    ApiKey,
}

/// `auth.json`: one entry per sign-in (`{"<issuer>::<client>": {"key": …, "email": …}}`).
fn signed_in_as(auth: &str) -> Option<Found> {
    let v: Value = serde_json::from_str(auth.trim()).ok()?;
    let entries: Vec<&Value> = v.as_object()?.values().filter(|e| e["key"].as_str().is_some_and(|k| !k.is_empty())).collect();
    if entries.is_empty() {
        return None;
    }
    let email = entries.iter().find_map(|e| e["email"].as_str().filter(|m| m.contains('@')));
    Some(Found::Account(email.map(String::from)))
}

/// In WSL or over SSH: whether `auth.json` holds a sign-in, and its email. Only those come back.
const REMOTE_ACCOUNT: &str = r#"f="${GROK_HOME:-$HOME/.grok}/auth.json"
if [ -s "$f" ] && grep -q '"key"[[:space:]]*:[[:space:]]*"[^"]' "$f"; then
  e=$(grep -o '"email"[[:space:]]*:[[:space:]]*"[^"]*@[^"]*"' "$f" | head -n 1 | sed 's/.*"\([^"]*\)"$/\1/')
  echo "account $e"
elif [ -n "$XAI_API_KEY" ]; then echo api-key
else echo none; fi"#;

fn remote_account(location: &Location, env: &[EnvVar]) -> Result<Option<Found>, String> {
    let args = vec!["-c".into(), REMOTE_ACCOUNT.into()];
    let out = oah_transport::run(&Spec { location, cwd: "~", program: "sh", args, env }, None, Duration::from_secs(20))?;
    if !out.ok() {
        return Err(out.stderr.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("couldn't read the sign-in").to_string());
    }
    Ok(parse_remote_account(&out.stdout))
}

fn parse_remote_account(out: &str) -> Option<Found> {
    let line = out.lines().map(str::trim).rfind(|l| !l.is_empty())?;
    match line {
        "api-key" => Some(Found::ApiKey),
        "none" => None,
        l => l.strip_prefix("account").map(|e| Found::Account(Some(e.trim().to_string()).filter(|e| e.contains('@')))),
    }
}

// ---- Limits ----------------------------------------------------------------------------------

/// The plan's credit use on `location`, as `/usage` shows it.
pub fn limits(binary: &str, location: &Location, env: &[EnvVar]) -> Limits {
    let fail = |e: String| Limits { error: Some(e), ..Default::default() };
    // Not installed there: nothing to show.
    if matches!(location, Location::Local) && crate::probe::resolve(oah_core::settings::AgentKind::Grok, binary).is_none() {
        return Limits { missing: true, ..Default::default() };
    }
    if !matches!(location, Location::Local) {
        let prog = program(binary);
        let cwd = home_cwd(location);
        let spec = Spec { location, cwd: &cwd, program: prog, args: vec!["--version".into()], env };
        match oah_transport::run(&spec, None, Duration::from_secs(15)) {
            Ok(o) if o.code == Some(127) => return Limits { missing: true, ..Default::default() },
            Ok(_) => {}
            Err(e) => return fail(e),
        }
    }
    match with_agent(binary, location, env, |rpc, _| rpc.call("_x.ai/billing", json!({}), Duration::from_secs(30))) {
        Ok(v) => match parse_billing(&v) {
            Ok((windows, extra)) => Limits { windows, extra, ..Default::default() },
            Err(e) => fail(e),
        },
        Err(e) if is_auth_error(&e) => fail(NOT_SIGNED_IN.into()),
        Err(e) => fail(e),
    }
}

fn is_auth_error(e: &str) -> bool {
    let e = e.to_lowercase();
    e.contains("authentication required") || e.contains("not signed in") || e.contains("unauthenticated") || e.contains("401")
}

/// `_x.ai/billing` → the period's window (`creditUsagePercent`, left out of the reply while it's
/// 0) and the plan's name. No `config`: the plan's usage isn't the user's to see (a team's, or an
/// API key's).
pub fn parse_billing(v: &Value) -> Result<(Vec<LimitWindow>, Vec<LimitExtra>), String> {
    let c = &v["config"];
    if !c.is_object() {
        return Err("Grok doesn't report usage limits for this sign-in (a team's or an API key's are managed elsewhere).".into());
    }
    let period = if c["currentPeriod"].is_object() { &c["currentPeriod"] } else { &c["billingPeriod"] };
    let label = match period["type"].as_str().unwrap_or("") {
        t if t.contains("WEEK") => "This week",
        t if t.contains("MONTH") => "This month",
        t if t.contains("DAY") => "Today",
        _ => "This billing period",
    };
    let used = c["creditUsagePercent"].as_f64().or_else(|| c["creditUsagePercent"]["val"].as_f64()).unwrap_or(0.0);
    let ends = period["end"].as_str().map(|s| json!(s)).unwrap_or_else(|| c["billingPeriodEnd"].clone());
    let windows =
        vec![LimitWindow { label: label.into(), used_percent: used.round().clamp(0.0, 100.0), resets_at: crate::account::reset_ms(&ends) }];
    let extra = v["subscription_tier"]
        .as_str()
        .filter(|t| !t.is_empty())
        .map(|t| vec![LimitExtra { label: "Plan".into(), value: Some(t.to_string()), note: None }])
        .unwrap_or_default();
    Ok((windows, extra))
}

// ---- Models and commands ---------------------------------------------------------------------

/// `modelState` (from `initialize`, or a session's `models`): every model with its efforts
/// (lowest first) and the one it uses unpicked.
pub fn parse_models(state: &Value) -> Vec<ModelOption> {
    state["availableModels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = m["modelId"].as_str()?.to_string();
            let listed: Vec<&Value> = m["_meta"]["reasoningEfforts"].as_array().into_iter().flatten().collect();
            let mut efforts: Vec<String> =
                listed.iter().filter_map(|e| e["value"].as_str().or(e["id"].as_str()).map(String::from)).collect();
            efforts.sort_by_key(|e| crate::acp::effort_rank(e));
            let default_effort = listed
                .iter()
                .find(|e| e["default"] == true)
                .and_then(|e| e["value"].as_str().or(e["id"].as_str()))
                .or(m["_meta"]["reasoningEffort"].as_str())
                .map(String::from)
                .filter(|_| !efforts.is_empty());
            Some(ModelOption {
                name: m["name"].as_str().unwrap_or(&id).to_string(),
                description: m["description"].as_str().unwrap_or("").to_string(),
                id,
                efforts,
                default_effort,
            })
        })
        .collect()
}

/// Each model's context window, by id.
pub fn context_windows(state: &Value) -> Vec<(String, u64)> {
    state["availableModels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| Some((m["modelId"].as_str()?.to_string(), m["_meta"]["totalContextTokens"].as_u64()?)))
        .collect()
}

/// The commands oah leaves out: `/always-approve` (oah's permission modes do that, and answer
/// what they allow themselves), `/login` and `/logout` (every chat shares the sign-in).
pub fn hidden_command(name: &str) -> bool {
    matches!(name, "always-approve" | "login" | "logout")
}

/// The models and commands for a chat that hasn't started, from `initialize` alone.
pub fn catalog(opts: &StartOptions) -> DriverResult<(Vec<ModelOption>, Vec<SlashCommand>)> {
    let (models, commands) = with_agent(&opts.binary, &opts.location, &opts.env, |_, init| {
        let models = parse_models(&init["_meta"]["modelState"]);
        let mut commands = crate::acp::acp_commands(&init["_meta"]["availableCommands"]);
        commands.retain(|c| !hidden_command(&c.name));
        Ok((models, commands))
    })?;
    if models.is_empty() {
        return Err(NOT_SIGNED_IN.into());
    }
    Ok((models, commands))
}

/// The quick model for one-off questions: the default (first listed) model at its lowest effort.
pub fn pick_cheap(models: &[ModelOption]) -> Option<(String, Option<String>, String)> {
    let m = models.first()?;
    let effort = m.efforts.first().cloned();
    let label = match &effort {
        Some(e) => format!("{} ({e})", m.name),
        None => m.name.clone(),
    };
    Some((m.id.clone(), effort, label))
}

/// The models `grok` offers on `location`, for picking a one-off's model.
pub fn models(binary: &str, location: &Location, env: &[EnvVar]) -> DriverResult<Vec<ModelOption>> {
    with_agent(binary, location, env, |_, init| Ok(parse_models(&init["_meta"]["modelState"])))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session's history as Grok Build 1.x saves it (two turns captured from it, cut
    /// short): what was said after the lines the chat already has.
    #[test]
    fn reads_what_was_said_in_a_saved_history() {
        let lines = [
            json!({"type": "system", "content": "You are Grok…"}),
            json!({"type": "user", "content": [{"type": "text", "text": "<user_info>\nOS Version: linux\n</user_info>"}]}),
            json!({"type": "user", "content": [{"type": "text", "text": "<user_query>\nfrom the chat\n</user_query>"}], "prompt_index": 0}),
            json!({"type": "assistant", "content": "before", "model_id": "grok-4.7-build"}),
            // From here on, the CLI's.
            json!({"type": "user", "content": [{"type": "text", "text": "<system-reminder>\nMCP servers currently connecting\n</system-reminder>"}], "synthetic_reason": "system_reminder"}),
            json!({"type": "user", "content": [{"type": "text", "text": "<user_query>\nRun the shell command `echo hi-from-tool` and then reply with just DONE.\n</user_query>"}], "prompt_index": 1}),
            json!({"type": "reasoning", "id": "rs_1", "summary": [{"type": "summary_text", "text": "Running `echo hi-from-tool`."}], "status": "completed"}),
            json!({"type": "assistant", "content": "I'll run that command.", "tool_calls": [{"id": "call-1", "name": "run_terminal_command", "arguments": "{\"command\":\"echo hi-from-tool\",\"description\":\"Print\"}"}]}),
            json!({"type": "tool_result", "tool_call_id": "call-1", "content": "exit: 0\nhi-from-tool\n"}),
            json!({"type": "reasoning", "id": "rs_2", "summary": [], "status": "completed"}),
            json!({"type": "assistant", "content": "DONE", "model_id": "grok-4.7-build"}),
            json!({"type": "assistant", "content": "", "tool_calls": [{"id": "call-2", "name": "run_terminal_command", "arguments": "{\"command\":\"false\"}"}]}),
            json!({"type": "tool_result", "tool_call_id": "call-2", "content": "exit: 1\n"}),
        ];
        let file = lines.iter().map(Value::to_string).collect::<Vec<_>>().join("\n");
        let ev = history_events(&file, 4, "r1");
        let kinds: Vec<String> = ev.iter().map(|e| serde_json::to_value(e).unwrap()["type"].as_str().unwrap().to_string()).collect();
        assert_eq!(
            kinds,
            [
                "userMessage",
                "turnStarted",
                "reasoning",
                "text",
                "toolStarted",
                "toolFinished",
                "text",
                "toolStarted",
                "toolFinished",
                "turnFinished"
            ]
        );
        assert!(
            matches!(&ev[0], ChatEvent::UserMessage { id, text, .. } if id == "cli-r1-5" && text == "Run the shell command `echo hi-from-tool` and then reply with just DONE.")
        );
        assert!(
            matches!(&ev[4], ChatEvent::ToolStarted { item_id, tool } if item_id == "call-1" && tool.kind == ToolKind::Bash && tool.title == "echo hi-from-tool")
        );
        assert!(matches!(&ev[5], ChatEvent::ToolFinished { ok: true, output, .. } if output == "hi-from-tool"));
        assert!(matches!(&ev[6], ChatEvent::Text { text, .. } if text == "DONE"));
        assert!(matches!(&ev[8], ChatEvent::ToolFinished { ok: false, .. }));
        assert!(history_events(&file, lines.len(), "r1").is_empty());
    }

    /// `initialize`'s `_meta.modelState` from grok 1.0.77 (cut to two models).
    fn state() -> Value {
        json!({"currentModelId": "grok-4.7", "availableModels": [
            {"modelId": "grok-4.7", "name": "Grok 4.7", "description": "SpaceXAI's latest frontier model",
             "_meta": {"totalContextTokens": 256000, "supportsReasoningEffort": true, "reasoningEffort": "high", "reasoningEfforts": [
                {"id": "xhigh", "value": "xhigh", "label": "Extra High", "default": false},
                {"id": "high", "value": "high", "label": "High", "default": true},
                {"id": "medium", "value": "medium", "label": "Medium", "default": false},
                {"id": "low", "value": "low", "label": "Low", "default": false}], "contextWindows": [256000, 500000]}},
            {"modelId": "grok-4.5", "name": "Grok 4.5", "_meta": {"totalContextTokens": 256000, "reasoningEfforts": [
                {"id": "high", "value": "high", "default": true}, {"id": "low", "value": "low", "default": false}]}},
            {"modelId": "grok-code", "name": "Grok Code"}]})
    }

    #[test]
    fn models_with_their_efforts() {
        let m = parse_models(&state());
        assert_eq!(m.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["grok-4.7", "grok-4.5", "grok-code"]);
        assert_eq!(m[0].efforts, ["low", "medium", "high", "xhigh"]);
        assert_eq!(m[0].default_effort.as_deref(), Some("high"));
        assert_eq!((m[0].name.as_str(), m[0].description.as_str()), ("Grok 4.7", "SpaceXAI's latest frontier model"));
        assert_eq!(m[1].efforts, ["low", "high"]);
        assert!(m[2].efforts.is_empty() && m[2].default_effort.is_none());
        assert_eq!(context_windows(&state())[0], ("grok-4.7".to_string(), 256000));
        assert_eq!(pick_cheap(&m), Some(("grok-4.7".into(), Some("low".into()), "Grok 4.7 (low)".into())));
        assert!(parse_models(&Value::Null).is_empty());
    }

    #[test]
    fn reads_billing() {
        // From grok 1.0.77 on SuperGrok, nothing used yet (proto JSON leaves zeros out).
        let v = json!({"config": {"currentPeriod": {"type": "USAGE_PERIOD_TYPE_WEEKLY", "start": "2026-11-04T15:53:19.529+00:00", "end": "2026-11-11T15:53:19.529+00:00"},
            "prepaidBalance": {"val": 0}, "isUnifiedBillingUser": true}, "subscription_tier": "SuperGrok"});
        let (w, extra) = parse_billing(&v).unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!((w[0].label.as_str(), w[0].used_percent), ("This week", 0.0));
        assert_eq!(w[0].resets_at, Some(1_794_412_399_529));
        assert_eq!(extra[0].value.as_deref(), Some("SuperGrok"));
        let mut some = v.clone();
        some["config"]["creditUsagePercent"] = json!(37.4);
        assert_eq!(parse_billing(&some).unwrap().0[0].used_percent, 37.0);
        some["config"]["currentPeriod"]["type"] = json!("USAGE_PERIOD_TYPE_MONTHLY");
        assert_eq!(parse_billing(&some).unwrap().0[0].label, "This month");
        assert!(parse_billing(&json!({"subscription_tier": ""})).is_err());
    }

    #[test]
    fn reads_who_from_auth_json() {
        let file = r#"{"https://auth.x.ai::b1a0": {"key": "eyJ.x.y", "auth_mode": "oidc", "email": "me@x.dev", "refresh_token": "r"}}"#;
        assert_eq!(signed_in_as(file), Some(Found::Account(Some("me@x.dev".into()))));
        assert_eq!(signed_in_as(r#"{"https://accounts.x.ai/sign-in": {"key": "k"}}"#), Some(Found::Account(None)));
        assert_eq!(signed_in_as("{}"), None);
        assert_eq!(signed_in_as(r#"{"x": {"key": ""}}"#), None);
        assert_eq!(signed_in_as("not json"), None);
        assert_eq!(parse_remote_account("account me@x.dev\n"), Some(Found::Account(Some("me@x.dev".into()))));
        assert_eq!(parse_remote_account("account \n"), Some(Found::Account(None)));
        assert_eq!(parse_remote_account("none"), None);
        assert_eq!(parse_remote_account("api-key"), Some(Found::ApiKey));
    }

    #[test]
    fn sign_in_and_env() {
        assert_eq!(login_line("grok", &Location::Local), "grok login");
        assert_eq!(login_line("grok", &Location::Wsl { distro: "Ubuntu".into() }), "grok login --device-auth");
        let mut env = vec![];
        acp_env(&mut env);
        assert_eq!((env[0].key.as_str(), env[0].value.as_str()), ("GROK_DISABLE_AUTOUPDATER", "1"));
        let mut env = vec![var("GROK_DISABLE_AUTOUPDATER", "0")];
        acp_env(&mut env);
        assert_eq!(env.len(), 1);
        assert!(hidden_command("always-approve") && !hidden_command("compact"));
    }

    /// Against the real `grok` here (ignored): who's signed in, the plan's window, the models.
    #[test]
    #[ignore]
    fn real_account_limits_models() {
        let loc = Location::Local;
        let a = account(&loc, &[]);
        eprintln!("account: signed_in={} label={:?} error={:?}", a.signed_in, a.label, a.error);
        assert!(a.signed_in);
        let l = limits("", &loc, &[]);
        eprintln!("limits: error={:?} missing={} extra={:?}", l.error, l.missing, l.extra);
        for w in &l.windows {
            eprintln!("  {} {}% resets {:?}", w.label, w.used_percent, w.resets_at);
        }
        assert_eq!(l.windows.len(), 1);
        let m = models("", &loc, &[]).unwrap();
        eprintln!("models: {}", m.iter().map(|m| format!("{} {:?}", m.id, m.efforts)).collect::<Vec<_>>().join(", "));
        assert!(m.iter().any(|m| m.efforts.len() > 1));
    }
}
