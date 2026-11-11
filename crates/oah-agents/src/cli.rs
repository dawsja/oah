//! An agent's own terminal interface, opened on a chat's session: what to run (in a
//! pseudo-terminal, see `oah_transport::pty::command_spec`) so the CLI continues the
//! conversation the chat's driver had, and the next message in the chat continues what was
//! said in the CLI. The flags follow each driver's: the same model, effort and permission mode.

use std::time::Duration;

use oah_core::settings::{AgentKind, EnvVar, Location, PermissionMode};
use oah_transport::Spec;

use crate::event::{ChatEvent, ToolCall, ToolKind};

/// The conversation the CLI opens on.
#[derive(Debug, Clone, PartialEq)]
pub enum Session {
    /// The chat's session, as its driver last reported it.
    Resume(String),
    /// A new one under an id oah picked (see [`presets_session`]), so the chat can resume it.
    New(String),
}

#[derive(Debug, Clone)]
pub struct CliOptions {
    pub kind: AgentKind,
    pub session: Session,
    pub model: Option<String>,
    /// Reasoning effort (`low` … `max`); none is the model's default.
    pub effort: Option<String>,
    pub permission: PermissionMode,
}

/// Whether the CLI starts a new conversation under an id it's given. The others name their
/// sessions themselves, so a chat has to have one (a first message sent) before its CLI opens.
pub fn presets_session(kind: AgentKind) -> bool {
    matches!(kind, AgentKind::Claude | AgentKind::Grok)
}

/// The CLI's arguments for `o`.
pub fn args(o: &CliOptions) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    fn flag(a: &mut Vec<String>, name: &str, value: &str) {
        a.extend([name.to_string(), value.to_string()]);
    }
    let model = o.model.as_deref().map(str::trim).filter(|m| !m.is_empty());
    let effort = o.effort.as_deref().filter(|e| crate::claude::valid_effort(e));
    match o.kind {
        AgentKind::Claude => {
            match &o.session {
                Session::Resume(id) => flag(&mut a, "--resume", id),
                Session::New(id) => flag(&mut a, "--session-id", id),
            }
            flag(&mut a, "--permission-mode", crate::claude::permission_mode_flag(o.permission));
            if let Some(m) = model {
                flag(&mut a, "--model", m);
            }
            if let Some(e) = effort {
                flag(&mut a, "--effort", e);
            }
            if o.permission == PermissionMode::FullAccess {
                a.push("--allow-dangerously-skip-permissions".into());
            }
        }
        AgentKind::Codex => {
            // `codex resume <id>`: Codex names its threads itself.
            if let Session::Resume(id) = &o.session {
                a.extend(["resume".to_string(), id.clone()]);
            }
            // Codex's CLI no longer has the driver's `untrusted` policy (ask before every
            // command), as a flag or as configuration. Asking first is a read-only sandbox it
            // has to ask its way out of; accepting edits, one it may write the folder in.
            let (approval, sandbox) = match o.permission {
                PermissionMode::Ask | PermissionMode::Plan => ("on-request", "read-only"),
                PermissionMode::AcceptEdits => ("on-request", "workspace-write"),
                PermissionMode::FullAccess => ("never", "danger-full-access"),
            };
            flag(&mut a, "-a", approval);
            flag(&mut a, "-s", sandbox);
            if let Some(m) = model {
                flag(&mut a, "-m", m);
            }
            if let Some(e) = effort {
                flag(&mut a, "-c", &format!("model_reasoning_effort=\"{e}\""));
            }
        }
        AgentKind::Opencode | AgentKind::Opencode2 => {
            if let Session::Resume(id) = &o.session {
                flag(&mut a, "-s", id);
            }
            if let Some(m) = model {
                flag(&mut a, "-m", m);
            }
            // Its plan agent reads and doesn't change; the other modes are rules (see [`env`]).
            if o.permission == PermissionMode::Plan {
                flag(&mut a, "--agent", "plan");
            }
        }
        AgentKind::Antigravity => {
            if let Session::Resume(id) = &o.session {
                flag(&mut a, "--conversation", id);
            }
            if let Some(m) = model {
                flag(&mut a, "--model", m);
            }
            if let Some(e) = effort {
                flag(&mut a, "--effort", e);
            }
            match o.permission {
                PermissionMode::Ask => {}
                PermissionMode::AcceptEdits => flag(&mut a, "--mode", "accept-edits"),
                PermissionMode::Plan => flag(&mut a, "--mode", "plan"),
                PermissionMode::FullAccess => a.push("--dangerously-skip-permissions".into()),
            }
        }
        AgentKind::Grok => {
            match &o.session {
                Session::Resume(id) => flag(&mut a, "--resume", id),
                Session::New(id) => flag(&mut a, "--session-id", id),
            }
            // Grok Build's mode names are Claude Code's.
            flag(&mut a, "--permission-mode", crate::claude::permission_mode_flag(o.permission));
            if let Some(m) = model {
                flag(&mut a, "-m", m);
            }
            if let Some(e) = effort {
                flag(&mut a, "--reasoning-effort", e);
            }
        }
    }
    a
}

/// What the CLI's environment gets besides the provider's own: what its driver sets too (no
/// self-update mid-run), and OpenCode's permission mode, which it only takes as rules.
pub fn env(kind: AgentKind, permission: PermissionMode, env: &mut Vec<EnvVar>) {
    match kind {
        AgentKind::Antigravity => crate::antigravity::acp_env(env),
        AgentKind::Grok => crate::grok::acp_env(env),
        AgentKind::Opencode | AgentKind::Opencode2 => match permission {
            PermissionMode::Ask => crate::acp::opencode_rule(env, crate::acp::ASK_FOR_EDITS_AND_COMMANDS),
            PermissionMode::AcceptEdits => crate::acp::opencode_rule(env, r#"{"bash":"ask"}"#),
            PermissionMode::Plan | PermissionMode::FullAccess => {}
        },
        // Started from inside a Claude Code session (oah launched from its terminal), Claude
        // Code takes itself for a child of that session and saves no conversation: nothing the
        // chat could resume, or read back. This is the conversation to keep.
        AgentKind::Claude => {
            if !env.iter().any(|e| e.key == "CLAUDE_CODE_FORCE_SESSION_PERSISTENCE") {
                env.push(EnvVar { key: "CLAUDE_CODE_FORCE_SESSION_PERSISTENCE".into(), value: "1".into(), in_keychain: false });
            }
        }
        AgentKind::Codex => {}
    }
}

/// Whether the CLI has a conversation saved under `id` (one started as [`Session::New`]), so
/// resuming that id works. Claude Code saves one with the first message, so a CLI opened and
/// closed with nothing said leaves none; Grok Build saves an empty one as it starts. `None`
/// when it can't be told (the system didn't answer, or the agent names its own).
pub fn session_saved(kind: AgentKind, location: &Location, env: &[EnvVar], id: &str) -> Option<bool> {
    if !valid_id(id) {
        return Some(false);
    }
    // Under the CLI's own folder: `projects/<folder>/<id>.jsonl` (Claude Code),
    // `sessions/<folder>/<id>/` (Grok Build).
    let (root_var, root, sub, name) = match kind {
        AgentKind::Claude => ("CLAUDE_CONFIG_DIR", ".claude", "projects", format!("{id}.jsonl")),
        AgentKind::Grok => ("GROK_HOME", ".grok", "sessions", id.to_string()),
        _ => return None,
    };
    if matches!(location, Location::Local) {
        let base = env_dir(env, root_var).unwrap_or_else(|| oah_core::paths::home_dir().join(root)).join(sub);
        let Ok(folders) = std::fs::read_dir(&base) else { return Some(false) };
        return Some(folders.flatten().any(|f| f.path().join(&name).exists()));
    }
    // The provider's environment, so a config folder set there is the one looked in.
    let script = format!(r#"for f in "${{{root_var}:-$HOME/{root}}}"/{sub}/*/{name}; do [ -e "$f" ] && exit 0; done; exit 1"#);
    let spec = Spec { location, cwd: "~", program: "sh", args: vec!["-c".into(), script], env };
    match oah_transport::run(&spec, None, Duration::from_secs(20)) {
        Ok(out) => match out.code {
            Some(0) => Some(true),
            Some(1) => Some(false),
            _ => None,
        },
        Err(_) => None,
    }
}

/// A session id as the CLIs make them (a UUID, `ses_…`): safe in a path and in a script.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

// ---- What was said in the CLI -----------------------------------------------------------------
//
// The CLI saves the conversation as it goes, as its driver's sessions are saved. When it ends,
// what was said in it is read back from there and becomes the chat's: [`mark`] notes where the
// conversation stood as the CLI opened, [`outcome`] reads what came after.

/// The agent's CLI where a chat's project is: enough to run it, or to read what it saved.
#[derive(Debug, Clone, Copy)]
pub struct Target<'a> {
    pub kind: AgentKind,
    pub location: &'a Location,
    /// The folder the chat works in (on `location`).
    pub cwd: &'a str,
    /// The CLI; empty is the usual command.
    pub binary: &'a str,
    pub env: &'a [EnvVar],
}

/// Where a chat's conversation stood as its CLI opened.
#[derive(Debug, Clone, PartialEq)]
pub enum Mark {
    /// At this moment (Unix milliseconds, this computer's clock): Claude Code stamps what it
    /// saves with the time.
    Since(u64),
    /// After this many lines of the conversation's file (Codex, Grok Build), messages of the
    /// session (OpenCode), or messages from the user in it (Antigravity).
    Past(usize),
    /// Where it stood couldn't be told (its file wasn't found, the agent didn't answer), so
    /// what's said after can't be told from what was there: the chat says the CLI had it, not
    /// what was said.
    Unread,
}

/// What the CLI is asked to note while it runs: each conversation it's on, in a file on the
/// system it runs on. So a `/clear` or a `/resume` in it is known of afterwards, and when, by
/// that system's clock.
#[derive(Debug, Clone, PartialEq)]
pub struct Watch {
    pub log: String,
    /// Added to the CLI's own arguments.
    pub args: Vec<String>,
}

/// The [`Watch`] for a CLI that can keep one: Claude Code, through a `SessionStart` hook given
/// for this run only (`--settings`), where there's a POSIX shell to run it (not on Windows
/// itself). `tag` names the run.
pub fn watch(kind: AgentKind, location: &Location, tag: &str) -> Option<Watch> {
    if kind != AgentKind::Claude || !valid_id(tag) || (cfg!(windows) && matches!(location, Location::Local)) {
        return None;
    }
    let log = format!("/tmp/oah-cli-{tag}.log");
    // The hook gets the session as JSON on stdin; a line is the time, then that.
    let command = format!(r#"{{ printf '%s ' "$(date +%s)"; cat; echo; }} >> {log}"#);
    let settings = serde_json::json!({"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": command}]}]}});
    Some(Watch { log, args: vec!["--settings".into(), settings.to_string()] })
}

/// A conversation the CLI was on, as its [`Watch`] noted it.
#[derive(Debug, Clone, PartialEq)]
struct Visit {
    session: String,
    /// When it came to it (Unix milliseconds, the clock of the system it runs on).
    at: u64,
    /// The file it saves the conversation in.
    file: Option<String>,
}

fn visits(log: &str) -> Vec<Visit> {
    log.lines()
        .filter_map(|l| {
            let (secs, json) = l.trim().split_once(' ')?;
            let v: serde_json::Value = serde_json::from_str(json).ok()?;
            let session = v["session_id"].as_str().filter(|s| valid_id(s))?.to_string();
            Some(Visit { session, at: secs.parse::<u64>().ok()? * 1000, file: v["transcript_path"].as_str().map(String::from) })
        })
        .collect()
}

/// What the CLI's time with a chat left.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    /// The conversation the CLI ended on, when that isn't the one it was opened on (a `/clear`
    /// or `/resume` there): the chat's from now on.
    pub moved_to: Option<String>,
    /// What was said, as the events a driver would have reported; none when this agent's
    /// conversations aren't read back, or this one couldn't be.
    pub events: Option<Vec<ChatEvent>>,
}

/// Where the conversation stands now, before the CLI opens on `session` (none: a new one).
/// `now`: Unix milliseconds.
pub fn mark(t: &Target, session: Option<&str>, now: u64) -> Mark {
    let past = |count: Option<usize>| count.map(Mark::Past).unwrap_or(Mark::Unread);
    match (t.kind, session) {
        (AgentKind::Claude, _) => Mark::Since(now),
        // A conversation the CLI is about to begin has nothing in it yet.
        (_, None) => Mark::Past(0),
        (AgentKind::Codex, Some(id)) => past(rollout(t, id).map(|text| text.lines().count())),
        (AgentKind::Opencode | AgentKind::Opencode2, Some(id)) => {
            past(opencode_export(t, id).map(|v| v["messages"].as_array().map_or(0, Vec::len)))
        }
        (AgentKind::Antigravity, Some(id)) => past(replay(t, id).map(|updates| crate::acp::replayed_messages(&updates))),
        (AgentKind::Grok, Some(id)) => past(grok_history(t, id).map(|text| text.lines().count())),
    }
}

/// What the CLI left, once it has ended: opened on `session` when the conversation stood at
/// `mark`, keeping `watch` (its log is read, and removed).
pub fn outcome(t: &Target, session: &str, mark: &Mark, watch: Option<&Watch>) -> Outcome {
    match (t.kind, mark) {
        (AgentKind::Claude, Mark::Since(opened)) => {
            let seen = watch.and_then(|w| take_file(t, &w.log)).map(|log| visits(&log)).unwrap_or_default();
            // Each conversation it was on, from when it first came to it; unwatched, the one
            // it was opened on, from then.
            let mut stretches: Vec<Visit> = Vec::new();
            for v in &seen {
                if !stretches.iter().any(|s| s.session == v.session) {
                    stretches.push(v.clone());
                }
            }
            if stretches.is_empty() {
                stretches.push(Visit { session: session.to_string(), at: *opened, file: None });
            }
            // A conversation it made and said nothing in (a `/clear`, then out) has no file.
            let mut events = Vec::new();
            let mut saved: Vec<&str> = Vec::new();
            // The watch tells time in whole seconds, by the clock where the CLI runs. For the
            // conversation it was opened on, this computer's own note of the moment is finer,
            // where the clock is the same one (not on an SSH host).
            let same_clock = !matches!(t.location, Location::Ssh { .. });
            for v in &stretches {
                if let Some(text) = claude_session(t, v) {
                    saved.push(&v.session);
                    let since = if same_clock && v.session == session { v.at.max(*opened) } else { v.at };
                    events.extend(crate::claude::session_events(&text, since));
                }
            }
            // The last one it was on that has something to resume is the chat's.
            let moved_to =
                seen.iter().rev().map(|v| v.session.as_str()).find(|id| saved.contains(id)).filter(|id| *id != session).map(String::from);
            Outcome { moved_to, events: (!saved.is_empty()).then_some(events) }
        }
        (AgentKind::Codex, Mark::Past(lines)) => {
            Outcome { moved_to: None, events: rollout(t, session).map(|text| crate::codex::rollout_events(&text, *lines)) }
        }
        (AgentKind::Opencode | AgentKind::Opencode2, Mark::Past(messages)) => {
            Outcome { moved_to: None, events: opencode_export(t, session).map(|v| opencode_events(&v, *messages)) }
        }
        (AgentKind::Antigravity, Mark::Past(messages)) => Outcome {
            moved_to: None,
            events: replay(t, session).map(|updates| crate::acp::replay_events(t.kind, &updates, *messages, &tag())),
        },
        (AgentKind::Grok, Mark::Past(lines)) => {
            Outcome { moved_to: None, events: grok_history(t, session).map(|text| crate::grok::history_events(&text, *lines, &tag())) }
        }
        _ => Outcome::default(),
    }
}

/// A few characters no other read-back has: the ids of what's read are made with them, where
/// the agent's own records give none.
fn tag() -> String {
    format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros()).unwrap_or(0))
}

/// A session as its agent replays it over ACP (Antigravity keeps its conversations in a form
/// only it reads).
fn replay(t: &Target, session: &str) -> Option<Vec<serde_json::Value>> {
    if !valid_id(session) {
        return None;
    }
    let opts = crate::driver::StartOptions {
        kind: t.kind,
        cwd: t.cwd.to_string(),
        location: t.location.clone(),
        binary: t.binary.to_string(),
        model: None,
        effort: None,
        permission: PermissionMode::Ask,
        resume: None,
        env: t.env.to_vec(),
    };
    crate::acp::replay(&opts, session).ok()
}

/// The file Grok Build saves a session's messages in, whole:
/// `sessions/<folder>/<session>/chat_history.jsonl` under its home folder.
fn grok_history(t: &Target, session: &str) -> Option<String> {
    if !valid_id(session) {
        return None;
    }
    if matches!(t.location, Location::Local) {
        let base = env_dir(t.env, "GROK_HOME").unwrap_or_else(|| oah_core::paths::home_dir().join(".grok")).join("sessions");
        let file =
            std::fs::read_dir(base).ok()?.flatten().map(|d| d.path().join(session).join("chat_history.jsonl")).find(|p| p.is_file())?;
        return std::fs::read_to_string(file).ok();
    }
    script(
        t,
        format!(
            r#"for f in "${{GROK_HOME:-$HOME/.grok}}"/sessions/*/{session}/chat_history.jsonl; do [ -f "$f" ] && {{ cat "$f"; exit 0; }}; done; exit 1"#
        ),
    )
}

/// The most of a saved conversation that's read back: what the CLI added is at its end.
const TAIL: u64 = 8 * 1024 * 1024;

/// A folder a CLI on this computer is pointed at by its environment: the provider's, else
/// oah's own, which the CLI inherits.
fn env_dir(env: &[EnvVar], key: &str) -> Option<std::path::PathBuf> {
    let set = env.iter().rfind(|e| e.key == key).map(|e| e.value.trim().to_string()).or_else(|| std::env::var(key).ok());
    set.filter(|v| !v.trim().is_empty()).map(std::path::PathBuf::from)
}

/// The end of a file on this computer (whole lines but perhaps the first).
fn tail_local(path: &std::path::Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(TAIL))).ok()?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// What a POSIX script prints where the project is (a WSL distro, an SSH host), run with the
/// provider's environment; none when it fails.
fn script(t: &Target, script: String) -> Option<String> {
    let spec = Spec { location: t.location, cwd: "~", program: "sh", args: vec!["-c".into(), script], env: t.env };
    oah_transport::run(&spec, None, Duration::from_secs(30)).ok().filter(|o| o.ok()).map(|o| o.stdout)
}

/// A small file where the project is, read and removed.
fn take_file(t: &Target, path: &str) -> Option<String> {
    if matches!(t.location, Location::Local) {
        let text = std::fs::read_to_string(path).ok();
        let _ = std::fs::remove_file(path);
        return text;
    }
    let f = oah_core::ssh::sh_quote(path);
    script(t, format!("cat {f} 2>/dev/null; rm -f {f}"))
}

/// The end of the file Claude Code saves a conversation in: the one its watch named, else
/// `projects/<folder>/<session>.jsonl` under its config folder.
fn claude_session(t: &Target, v: &Visit) -> Option<String> {
    if !valid_id(&v.session) {
        return None;
    }
    if matches!(t.location, Location::Local) {
        if let Some(text) = v.file.as_deref().and_then(|f| tail_local(std::path::Path::new(f))) {
            return Some(text);
        }
        let base = env_dir(t.env, "CLAUDE_CONFIG_DIR").unwrap_or_else(|| oah_core::paths::home_dir().join(".claude")).join("projects");
        let name = format!("{}.jsonl", v.session);
        return std::fs::read_dir(base).ok()?.flatten().map(|d| d.path().join(&name)).find(|p| p.is_file()).and_then(|p| tail_local(&p));
    }
    let named = v
        .file
        .as_deref()
        .map(|f| format!("f={}; [ -f \"$f\" ] && {{ tail -c {TAIL} \"$f\"; exit 0; }}\n", oah_core::ssh::sh_quote(f)))
        .unwrap_or_default();
    script(
        t,
        format!(
            r#"{named}for f in "${{CLAUDE_CONFIG_DIR:-$HOME/.claude}}"/projects/*/{id}.jsonl; do [ -f "$f" ] && {{ tail -c {TAIL} "$f"; exit 0; }}; done; exit 1"#,
            id = v.session
        ),
    )
}

/// The file Codex saves a thread in, whole: `sessions/<y>/<m>/<d>/rollout-<time>-<thread>.jsonl`
/// under its home folder.
fn rollout(t: &Target, thread: &str) -> Option<String> {
    if !valid_id(thread) {
        return None;
    }
    if matches!(t.location, Location::Local) {
        let base = env_dir(t.env, "CODEX_HOME").unwrap_or_else(|| oah_core::paths::home_dir().join(".codex")).join("sessions");
        let end = format!("-{thread}.jsonl");
        // Year, month, day, then the files.
        let mut folders = vec![base];
        for _ in 0..3 {
            folders = folders
                .iter()
                .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()))
                .filter(|p| p.is_dir())
                .collect();
        }
        let file = folders
            .iter()
            .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()))
            .find(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("rollout-") && n.ends_with(&end)))?;
        return std::fs::read_to_string(file).ok();
    }
    script(
        t,
        format!(
            r#"for f in "${{CODEX_HOME:-$HOME/.codex}}"/sessions/*/*/*/rollout-*-{thread}.jsonl; do [ -f "$f" ] && {{ cat "$f"; exit 0; }}; done; exit 1"#
        ),
    )
}

/// A session as OpenCode exports it (`opencode export <id>`: `{info, messages}`).
fn opencode_export(t: &Target, session: &str) -> Option<serde_json::Value> {
    if !valid_id(session) {
        return None;
    }
    let program = if t.binary.trim().is_empty() { t.kind.default_binary() } else { t.binary.trim() };
    let spec = Spec { location: t.location, cwd: t.cwd, program, args: vec!["export".into(), session.into()], env: t.env };
    let out = oah_transport::run(&spec, None, Duration::from_secs(60)).ok().filter(|o| o.ok())?.stdout;
    // It may say what it's exporting first.
    serde_json::from_str(&out[out.find('{')?..]).ok()
}

/// What was said in an exported OpenCode session after its first `skip` messages: the events
/// a driver would have reported for the same turns.
pub fn opencode_events(export: &serde_json::Value, skip: usize) -> Vec<ChatEvent> {
    let mut out = Vec::new();
    let mut turn: Option<String> = None;
    let finish = |out: &mut Vec<ChatEvent>, turn: &mut Option<String>| {
        if let Some(turn_id) = turn.take() {
            out.push(ChatEvent::TurnFinished { turn_id, usage: None, cost_usd: None, duration_ms: None, interrupted: false });
        }
    };
    let text_of = |p: &serde_json::Value| p["text"].as_str().unwrap_or("").to_string();
    for m in export["messages"].as_array().into_iter().flatten().skip(skip) {
        let id = m["info"]["id"].as_str().unwrap_or("");
        let parts = m["parts"].as_array().map(Vec::as_slice).unwrap_or_default();
        if m["info"]["role"] == "user" {
            // What OpenCode adds to a message itself (a file's contents) isn't what was typed.
            let said: Vec<String> = parts.iter().filter(|p| p["type"] == "text" && p["synthetic"] != true).map(text_of).collect();
            let said = said.join("\n").trim().to_string();
            if said.is_empty() {
                continue;
            }
            finish(&mut out, &mut turn);
            out.push(ChatEvent::UserMessage { id: id.to_string(), text: said, images: vec![], files: vec![] });
            out.push(ChatEvent::TurnStarted { turn_id: format!("cli-{id}") });
            turn = Some(format!("cli-{id}"));
            continue;
        }
        if turn.is_none() {
            continue;
        }
        for p in parts {
            let item_id = p["id"].as_str().unwrap_or("").to_string();
            match p["type"].as_str() {
                Some("text") if !text_of(p).trim().is_empty() => out.push(ChatEvent::Text { item_id, text: text_of(p) }),
                Some("reasoning") if !text_of(p).trim().is_empty() => out.push(ChatEvent::Reasoning { item_id, text: text_of(p) }),
                Some("tool") => {
                    let name = p["tool"].as_str().unwrap_or("tool");
                    let (state, input) = (&p["state"], &p["state"]["input"]);
                    let s = |k: &str| input[k].as_str().unwrap_or("").to_string();
                    let path = Some(s("filePath")).filter(|f| !f.is_empty()).or_else(|| Some(s("path")).filter(|f| !f.is_empty()));
                    let (kind, title) = match name {
                        "bash" => (ToolKind::Bash, s("command")),
                        "read" => (ToolKind::Read, path.clone().unwrap_or_default()),
                        "edit" | "patch" | "multiedit" => (ToolKind::Edit, path.clone().unwrap_or_default()),
                        "write" => (ToolKind::Write, path.clone().unwrap_or_default()),
                        "grep" | "glob" | "list" => (ToolKind::Search, s("pattern")),
                        "webfetch" | "websearch" => (ToolKind::Web, Some(s("url")).filter(|u| !u.is_empty()).unwrap_or_else(|| s("query"))),
                        "task" => (ToolKind::Task, s("description")),
                        _ => (ToolKind::Other, String::new()),
                    };
                    let title = Some(title)
                        .filter(|t| !t.is_empty())
                        .or_else(|| state["title"].as_str().map(String::from))
                        .unwrap_or_else(|| name.to_string());
                    out.push(ChatEvent::ToolStarted {
                        item_id: item_id.clone(),
                        tool: ToolCall { kind, name: name.into(), title, input: input.clone(), path },
                    });
                    let done = state["status"].as_str();
                    if matches!(done, Some("completed" | "error")) {
                        let mut output = state["output"].as_str().or_else(|| state["error"].as_str()).unwrap_or("").to_string();
                        if output.len() > 16 * 1024 {
                            let mut cut = 16 * 1024;
                            while !output.is_char_boundary(cut) {
                                cut -= 1;
                            }
                            output.truncate(cut);
                            output.push_str("\n…");
                        }
                        out.push(ChatEvent::ToolFinished { item_id, ok: done == Some("completed"), output, diff: None });
                    }
                }
                _ => {}
            }
        }
    }
    finish(&mut out, &mut turn);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn opts(kind: AgentKind, session: Session, permission: PermissionMode) -> CliOptions {
        CliOptions { kind, session, model: Some("m1".into()), effort: Some("high".into()), permission }
    }

    fn resume() -> Session {
        Session::Resume("abc-123".into())
    }

    #[test]
    fn claude_resumes_or_starts_under_a_given_id() {
        assert_eq!(
            args(&opts(AgentKind::Claude, resume(), PermissionMode::AcceptEdits)),
            ["--resume", "abc-123", "--permission-mode", "acceptEdits", "--model", "m1", "--effort", "high"]
        );
        let new = args(&opts(AgentKind::Claude, Session::New("u-1".into()), PermissionMode::FullAccess));
        assert_eq!(new[..4], ["--session-id", "u-1", "--permission-mode", "bypassPermissions"]);
        assert_eq!(new.last().unwrap(), "--allow-dangerously-skip-permissions");
        // No model or effort set: the CLI's own.
        let bare = CliOptions {
            kind: AgentKind::Claude,
            session: resume(),
            model: Some(" ".into()),
            effort: None,
            permission: PermissionMode::Ask,
        };
        assert_eq!(args(&bare), ["--resume", "abc-123", "--permission-mode", "default"]);
    }

    #[test]
    fn codex_resumes_with_a_policy_its_cli_takes() {
        assert_eq!(
            args(&opts(AgentKind::Codex, resume(), PermissionMode::Plan)),
            ["resume", "abc-123", "-a", "on-request", "-s", "read-only", "-m", "m1", "-c", "model_reasoning_effort=\"high\""]
        );
        let full = args(&opts(AgentKind::Codex, resume(), PermissionMode::FullAccess));
        assert_eq!(full[2..6], ["-a", "never", "-s", "danger-full-access"]);
        // The policies its CLI takes: asking first is a sandbox to ask out of.
        assert_eq!(args(&opts(AgentKind::Codex, resume(), PermissionMode::Ask))[2..6], ["-a", "on-request", "-s", "read-only"]);
        assert_eq!(
            args(&opts(AgentKind::Codex, resume(), PermissionMode::AcceptEdits))[2..6],
            ["-a", "on-request", "-s", "workspace-write"]
        );
    }

    #[test]
    fn acp_agents_open_their_session() {
        assert_eq!(args(&opts(AgentKind::Opencode, resume(), PermissionMode::Plan)), ["-s", "abc-123", "-m", "m1", "--agent", "plan"]);
        assert_eq!(args(&opts(AgentKind::Opencode2, resume(), PermissionMode::Ask)), ["-s", "abc-123", "-m", "m1"]);
        assert_eq!(
            args(&opts(AgentKind::Antigravity, resume(), PermissionMode::AcceptEdits)),
            ["--conversation", "abc-123", "--model", "m1", "--effort", "high", "--mode", "accept-edits"]
        );
        assert_eq!(
            args(&opts(AgentKind::Antigravity, resume(), PermissionMode::FullAccess)).last().unwrap(),
            "--dangerously-skip-permissions"
        );
        assert_eq!(
            args(&opts(AgentKind::Grok, Session::New("u-2".into()), PermissionMode::Plan)),
            ["--session-id", "u-2", "--permission-mode", "plan", "-m", "m1", "--reasoning-effort", "high"]
        );
        assert_eq!(args(&opts(AgentKind::Grok, resume(), PermissionMode::Ask))[..2], ["--resume", "abc-123"]);
    }

    #[test]
    fn opencode_takes_its_mode_as_rules_unless_yours_are_set() {
        let mut e = vec![];
        env(AgentKind::Opencode, PermissionMode::Ask, &mut e);
        assert_eq!(e.iter().find(|v| v.key == "OPENCODE_PERMISSION").unwrap().value, r#"{"edit":"ask","bash":"ask"}"#);
        assert!(e.iter().any(|v| v.key == "OPENCODE_CONFIG_CONTENT"));
        let mut e = vec![];
        env(AgentKind::Opencode2, PermissionMode::AcceptEdits, &mut e);
        assert_eq!(e.iter().find(|v| v.key == "OPENCODE_PERMISSION").unwrap().value, r#"{"bash":"ask"}"#);
        let mut e = vec![];
        env(AgentKind::Opencode, PermissionMode::FullAccess, &mut e);
        assert!(e.is_empty());
        let mut yours = vec![EnvVar { key: "OPENCODE_PERMISSION".into(), value: "{}".into(), in_keychain: false }];
        env(AgentKind::Opencode, PermissionMode::Ask, &mut yours);
        assert_eq!(yours.len(), 1);
        let mut e = vec![];
        env(AgentKind::Grok, PermissionMode::Ask, &mut e);
        assert!(e.iter().any(|v| v.key == "GROK_DISABLE_AUTOUPDATER"));
        let mut e = vec![];
        env(AgentKind::Claude, PermissionMode::Ask, &mut e);
        assert_eq!((e[0].key.as_str(), e[0].value.as_str()), ("CLAUDE_CODE_FORCE_SESSION_PERSISTENCE", "1"));
    }

    /// The real CLIs take these flags and draw their interface in a terminal, on this
    /// computer: `cargo test -p oah-agents real_clis_open -- --ignored --nocapture`. Nothing
    /// is said in them, so none saves a conversation under the id it was given.
    #[test]
    #[ignore]
    #[cfg(unix)]
    fn real_clis_open() {
        use oah_transport::pty::{command_spec, Pty, PtyEvent};
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64;
        for kind in [AgentKind::Claude, AgentKind::Grok] {
            if oah_transport::find_local(kind.default_binary()).is_none() {
                println!("{}: not installed, skipped", kind.label());
                continue;
            }
            let dir = std::env::temp_dir().join(format!("oah-real-cli-{}-{nanos}", kind.default_binary()));
            std::fs::create_dir_all(&dir).unwrap();
            let id = format!("{:08x}-0000-4000-8000-{:012x}", std::process::id(), nanos & 0xffff_ffff_ffff);
            let o = CliOptions { kind, session: Session::New(id.clone()), model: None, effort: None, permission: PermissionMode::Ask };
            let mut e = vec![];
            env(kind, o.permission, &mut e);
            let spec =
                Spec { location: &Location::Local, cwd: dir.to_str().unwrap(), program: kind.default_binary(), args: args(&o), env: &e };
            let (tx, rx) = std::sync::mpsc::channel();
            let pty = Pty::spawn(&command_spec(&spec).unwrap(), 100, 30, move |e| drop(tx.send(e))).unwrap();
            // It draws, and is still there a few seconds on (a flag it didn't know ends it at once).
            let (mut text, until) = (String::new(), std::time::Instant::now() + Duration::from_secs(8));
            while let Ok(e) = rx.recv_timeout(until.saturating_duration_since(std::time::Instant::now())) {
                match e {
                    PtyEvent::Data(d) => text.push_str(&d),
                    PtyEvent::Exit(code) => panic!("{} exited ({code:?}): {text}", kind.label()),
                }
            }
            assert!(text.len() > 40, "{} drew nothing: {text:?}", kind.label());
            println!("{}: {} bytes drawn for {:?}", kind.label(), text.len(), args(&o));
            drop(pty);
            // Claude Code saves a conversation with its first message; Grok Build makes the
            // session's folder as it starts (an empty one resumes fine).
            let saved = session_saved(kind, &Location::Local, &e, &id);
            assert_eq!(saved, Some(kind == AgentKind::Grok), "{}", kind.label());
            let _ = std::fs::remove_dir_all(&dir);
            // What Grok kept for the folder that's gone.
            let kept = oah_core::paths::home_dir().join(".grok/sessions");
            for f in std::fs::read_dir(kept).into_iter().flatten().flatten() {
                if f.file_name().to_string_lossy().contains(&format!("oah-real-cli-grok-{nanos}")) {
                    let _ = std::fs::remove_dir_all(f.path());
                }
            }
        }
    }

    /// One live turn through a driver: what the agent said, and its session.
    #[cfg(unix)]
    fn live_turn(t: &Target, model: Option<&str>, resume: Option<&str>, ask: &str) -> (String, String) {
        let (said, session, failed) = live_turn_or_error(t, model, resume, ask);
        if let Some(why) = failed {
            panic!("{why}");
        }
        (said, session)
    }

    /// [`live_turn`], with what went wrong (the agent's error, when the turn ended in one)
    /// instead of a panic.
    #[cfg(unix)]
    fn live_turn_or_error(t: &Target, model: Option<&str>, resume: Option<&str>, ask: &str) -> (String, String, Option<String>) {
        use crate::driver::{start, Prompt, StartOptions};
        let (tx, rx) = std::sync::mpsc::channel();
        let sink: crate::driver::Sink = std::sync::Arc::new(move |e| drop(tx.send(e)));
        let opts = StartOptions {
            kind: t.kind,
            cwd: t.cwd.into(),
            location: t.location.clone(),
            binary: t.binary.into(),
            model: model.map(String::from),
            effort: None,
            permission: PermissionMode::Ask,
            resume: resume.map(String::from),
            env: t.env.to_vec(),
        };
        let mut d = start(opts, sink).expect("the agent starts");
        d.send(Prompt { id: "p".into(), text: ask.into(), images: vec![], files: vec![] }).unwrap();
        let (mut said, mut session, mut parts, mut failed) = (String::new(), String::new(), std::collections::BTreeMap::new(), None);
        loop {
            match rx.recv_timeout(Duration::from_secs(180)).expect("the turn ends") {
                ChatEvent::Started { session_id: Some(id), .. } => session = id,
                ChatEvent::TextDelta { text, .. } => said.push_str(&text),
                ChatEvent::Text { item_id, text } => drop(parts.insert(item_id, text)),
                ChatEvent::TurnFinished { .. } => break,
                ChatEvent::Error { message } => {
                    failed = Some(message);
                    break;
                }
                ChatEvent::Exited { message, .. } => {
                    failed = Some(format!("the agent exited: {message:?}"));
                    break;
                }
                _ => {}
            }
        }
        d.stop();
        (format!("{said} {}", parts.into_values().collect::<Vec<_>>().join(" ")), session, failed)
    }

    /// A terminal with a CLI in it, for a live test: what it has drawn, and keys for it.
    #[cfg(unix)]
    struct Live {
        pty: std::sync::Arc<oah_transport::pty::Pty>,
        screen: std::sync::Arc<std::sync::Mutex<String>>,
        ended: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    #[cfg(unix)]
    impl Live {
        fn open(t: &Target, args: Vec<String>) -> Live {
            use oah_transport::pty::{command_spec, Pty, PtyEvent};
            let program = if t.binary.is_empty() { t.kind.default_binary() } else { t.binary };
            let spec = command_spec(&Spec { location: t.location, cwd: t.cwd, program, args, env: t.env }).unwrap();
            let screen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
            let ended = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (s, e) = (screen.clone(), ended.clone());
            // What a terminal answers when asked what it is (xterm.js does in the app).
            let (answers_tx, answers) = std::sync::mpsc::channel::<&'static [u8]>();
            let pty = Pty::spawn(&spec, 120, 40, move |ev| match ev {
                PtyEvent::Data(d) => {
                    if d.contains("\x1b[c") {
                        let _ = answers_tx.send(b"\x1b[?62;22c");
                    }
                    s.lock().unwrap().push_str(&d);
                }
                PtyEvent::Exit(_) => e.store(true, std::sync::atomic::Ordering::SeqCst),
            })
            .unwrap();
            let pty = std::sync::Arc::new(pty);
            {
                let pty = pty.clone();
                std::thread::spawn(move || {
                    for answer in answers {
                        let _ = pty.write(answer);
                    }
                });
            }
            Live { pty, screen, ended }
        }

        /// Waits until `done`, answering the questions a CLI asks of a folder it hasn't seen.
        fn until(&self, what: &str, secs: u64, done: impl Fn() -> bool) {
            let end = std::time::Instant::now() + Duration::from_secs(secs);
            let mut trusted = false;
            while !done() {
                let screen = self.screen.lock().unwrap().clone();
                assert!(!self.ended.load(std::sync::atomic::Ordering::SeqCst), "the CLI ended before {what}: {screen}");
                assert!(std::time::Instant::now() < end, "never {what}: {}", &screen[screen.len().saturating_sub(1500)..]);
                // "Is this a project you trust?": Yes, whichever line it's on (No comes first
                // in Claude Code's, Yes in Antigravity's).
                let lower = screen.to_lowercase();
                if !trusted && lower.contains("trust") && lower.contains("enter") && lower.contains("yes,") {
                    trusted = true;
                    std::thread::sleep(Duration::from_millis(500));
                    if screen.find("No,").zip(screen.find("Yes,")).is_some_and(|(no, yes)| no < yes) {
                        self.pty.write(b"\x1b[B").unwrap();
                        std::thread::sleep(Duration::from_millis(300));
                    }
                    self.pty.write(b"\r").unwrap();
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }

        /// Types a line as a person would: the text, then Enter.
        fn say(&self, line: &str) {
            self.pty.write(line.as_bytes()).unwrap();
            std::thread::sleep(Duration::from_millis(600));
            self.pty.write(b"\r").unwrap();
        }
    }

    /// What was said, a line a message (an answer that came in pieces is one line).
    #[cfg(unix)]
    fn texts(events: &[ChatEvent]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut streaming = None::<String>;
        for e in events {
            match e {
                ChatEvent::TextDelta { item_id, text } if streaming.as_ref() == Some(item_id) => out.last_mut().unwrap().push_str(text),
                ChatEvent::TextDelta { item_id, text } => {
                    streaming = Some(item_id.clone());
                    out.push(format!("agent: {text}"));
                }
                ChatEvent::UserMessage { text, .. } => out.push(format!("you: {text}")),
                ChatEvent::Text { text, .. } => out.push(format!("agent: {text}")),
                _ => {}
            }
        }
        out
    }

    /// The whole handover with the real Claude Code, on this computer (a few short turns on
    /// its cheapest model): `cargo test -p oah-agents real_claude_handover -- --ignored --nocapture`.
    /// A chat's driver says something, the CLI opens on its session and more is said there,
    /// a `/clear` in the CLI starts another conversation, and afterwards: what was said in the
    /// CLI reads back as the chat's, the driver resuming the first session knows both halves,
    /// and the conversation the CLI ended on is the one the chat is moved to.
    #[test]
    #[ignore]
    #[cfg(unix)]
    fn real_claude_handover() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64;
        let dir = std::env::temp_dir().join(format!("oah-real-handover-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let cwd = dir.to_str().unwrap();
        let mut e = vec![];
        env(AgentKind::Claude, PermissionMode::Ask, &mut e);
        let place = place();
        let t = Target { kind: AgentKind::Claude, location: &place, cwd, binary: "", env: &e };

        let (said, session) = live_turn(&t, Some("haiku"), None, "Remember the code word PLUM-7. Reply with just OK.");
        println!("driver: {said:?} on {session}, at {place:?}");
        assert!(!session.is_empty());

        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        let m = mark(&t, Some(&session), now);
        let w = watch(AgentKind::Claude, &place, &format!("live{nanos}")).unwrap();
        let o = CliOptions {
            kind: AgentKind::Claude,
            session: Session::Resume(session.clone()),
            model: Some("haiku".into()),
            effort: None,
            permission: PermissionMode::Ask,
        };
        let cli = Live::open(&t, args(&o).into_iter().chain(w.args.clone()).collect());
        let seen = || visits(&std::fs::read_to_string(&w.log).unwrap_or_default());
        let said_in =
            |v: &Visit| claude_session(&t, v).map(|text| texts(&crate::claude::session_events(&text, v.at.max(now)))).unwrap_or_default();
        cli.until("it notes the session it resumed", 60, || !seen().is_empty());
        assert_eq!(seen()[0].session, session, "it resumed the chat's session");
        std::thread::sleep(Duration::from_secs(3));
        cli.say("Also remember a second code word, KIWI-3. Reply with just OK.");
        cli.until("it answers in the CLI", 120, || said_in(&seen()[0]).iter().any(|l| l.starts_with("agent: ")));
        // Another conversation, from inside the CLI.
        cli.say("/clear");
        cli.until("it notes the new session", 60, || seen().len() > 1);
        std::thread::sleep(Duration::from_secs(2));
        cli.say("Remember the code word FIG-9. Reply with just OK.");
        cli.until("it answers in the new session", 120, || {
            seen().last().is_some_and(|v| said_in(v).iter().any(|l| l.starts_with("agent: ")))
        });
        cli.say("/exit");
        let end = std::time::Instant::now() + Duration::from_secs(15);
        while !cli.ended.load(std::sync::atomic::Ordering::SeqCst) && std::time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(100));
        }
        println!("left with /exit: {}", cli.ended.load(std::sync::atomic::Ordering::SeqCst));
        drop(cli);

        let left = outcome(&t, &session, &m, Some(&w));
        let read = texts(&left.events.clone().expect("its conversations are read back"));
        println!("read back: {read:#?}\nmoved to: {:?}", left.moved_to);
        assert_eq!(read.iter().filter(|l| l.starts_with("you: ")).count(), 2, "{read:?}");
        assert!(read[0].contains("KIWI-3") && read.iter().any(|l| l.starts_with("you: ") && l.contains("FIG-9")), "{read:?}");
        assert!(!read.iter().any(|l| l.contains("PLUM-7")), "what the driver said isn't read twice: {read:?}");
        assert_eq!(read.iter().filter(|l| l.starts_with("agent: ")).count(), 2, "{read:?}");
        let moved = left.moved_to.expect("the /clear is followed");
        assert_ne!(moved, session);
        assert!(!std::path::Path::new(&w.log).exists());

        // The chat's driver, back on the first session, has what was said in the CLI too.
        let (both, again) =
            live_turn(&t, Some("haiku"), Some(&session), "Which code words did I ask you to remember? Reply with just the words.");
        println!("resumed {again}: {both:?}");
        assert!(both.contains("PLUM-7") && both.contains("KIWI-3"), "{both}");
        // And the conversation the chat moved to resumes as its own.
        let (third, _) = live_turn(&t, Some("haiku"), Some(&moved), "Which code word did I ask you to remember? Reply with just the word.");
        println!("moved: {third:?}");
        assert!(third.contains("FIG-9") && !third.contains("PLUM-7"), "{third}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A handover with a real agent whose conversations are read back whole afterwards: a
    /// chat's driver says something, the CLI opens on its session and more is said there;
    /// then what was said in the CLI reads back as the chat's (and nothing of before), and
    /// the driver resuming the session knows both halves.
    #[cfg(unix)]
    fn handover(t: &Target, model: Option<&str>) {
        let (said, session) = live_turn(t, model, None, "Remember the code word PLUM-7. Reply with just OK.");
        println!("driver: {said:?} on {session}");
        let m = mark(t, Some(&session), 0);
        println!("as the CLI opens: {m:?}");
        assert!(matches!(m, Mark::Past(n) if n >= 1), "the driver's turn is counted: {m:?}");

        let o = CliOptions {
            kind: t.kind,
            session: Session::Resume(session.clone()),
            model: model.map(String::from),
            effort: None,
            permission: PermissionMode::Ask,
        };
        let also = "Also remember a second code word, KIWI-3. Reply with just OK.";
        {
            let cli = Live::open(t, args(&o));
            cli.until("it draws", 60, || cli.screen.lock().unwrap().len() > 200);
            // Ready for typing once it has stopped drawing (it reads the conversation in first),
            // and done answering once it has again. What it saved is read when it's gone, as the
            // app does.
            let settled = |what: &str, quiet: u64| {
                let drawn = std::cell::Cell::new((0usize, std::time::Instant::now()));
                cli.until(what, 180, || {
                    let len = cli.screen.lock().unwrap().len();
                    if len != drawn.get().0 {
                        drawn.set((len, std::time::Instant::now()));
                    }
                    drawn.get().1.elapsed() > Duration::from_secs(quiet)
                });
            };
            settled("it settles", 4);
            let before = cli.screen.lock().unwrap().len();
            cli.say(also);
            cli.until("it takes the message", 30, || cli.screen.lock().unwrap().len() > before + 40);
            settled("it answers in the CLI", 12);
            if std::env::var("OAH_TEST_SHOW").is_ok() {
                let screen = cli.screen.lock().unwrap().clone();
                println!("the CLI's screen at the end:\n{}", &screen[screen.len().saturating_sub(2500)..]);
            }
            drop(cli);
            std::thread::sleep(Duration::from_secs(2));
        }
        let left = outcome(t, &session, &m, None);
        let read = texts(&left.events.expect("its conversations are read back"));
        println!("read back: {read:#?}");
        assert!(read[0].starts_with("you: ") && read[0].contains("KIWI-3"), "{read:?}");
        assert_eq!(read.iter().filter(|l| l.starts_with("you: ")).count(), 1, "{read:?}");
        assert!(!read.iter().any(|l| l.contains("PLUM-7")), "what the driver said isn't read twice: {read:?}");
        assert_eq!(left.moved_to, None);

        let (both, again) = live_turn(t, model, Some(&session), "Which code words did I ask you to remember? Reply with just the words.");
        println!("resumed {again}: {both:?}");
        assert_eq!(again, session);
        assert!(both.contains("PLUM-7") && both.contains("KIWI-3"), "{both}");
    }

    /// Where the live tests run their agent: this computer, or the SSH host
    /// `OAH_TEST_SSH_HOST` names (one that's this computer too: the tests make their folder
    /// here), so the driver, the CLI's terminal and the reading back all go over ssh.
    #[cfg(unix)]
    fn place() -> Location {
        match std::env::var("OAH_TEST_SSH_HOST") {
            Ok(host) => Location::Ssh { host },
            Err(_) => Location::Local,
        }
    }

    #[cfg(unix)]
    fn scratch(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("oah-real-{name}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The handover with the real OpenCode, on this computer, on one of its free models:
    /// `OAH_TEST_OPENCODE=<opencode> [OAH_TEST_HOME=<a scratch home>] cargo test -p oah-agents
    /// real_opencode_handover -- --ignored --nocapture`.
    #[test]
    #[ignore]
    #[cfg(unix)]
    fn real_opencode_handover() {
        let Ok(binary) = std::env::var("OAH_TEST_OPENCODE") else { return println!("set OAH_TEST_OPENCODE to the opencode to run") };
        let model = std::env::var("OAH_TEST_OPENCODE_MODEL").unwrap_or_else(|_| "opencode/glm-5.3-flash-free".into());
        let dir = scratch("opencode");
        let mut e: Vec<EnvVar> =
            std::env::var("OAH_TEST_HOME").into_iter().map(|h| EnvVar { key: "HOME".into(), value: h, in_keychain: false }).collect();
        env(AgentKind::Opencode, PermissionMode::Ask, &mut e);
        handover(
            &Target { kind: AgentKind::Opencode, location: &place(), cwd: dir.to_str().unwrap(), binary: &binary, env: &e },
            Some(&model),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The real Codex, on this computer: `OAH_TEST_CODEX=<codex> [OAH_TEST_CODEX_HOME=<a scratch
    /// home>] cargo test -p oah-agents real_codex_session -- --ignored --nocapture`. Signed in,
    /// it's the whole handover. Not signed in (a scratch home), Codex still starts the thread
    /// and records what's said to it before the model refuses, which is enough to see that the
    /// driver's thread is the one its CLI resumes, that the CLI takes oah's flags, and that
    /// what's said through the CLI reads back from the file Codex really writes.
    #[test]
    #[ignore]
    #[cfg(unix)]
    fn real_codex_session() {
        let Ok(binary) = std::env::var("OAH_TEST_CODEX") else { return println!("set OAH_TEST_CODEX to the codex to run") };
        let dir = scratch("codex");
        let e: Vec<EnvVar> = std::env::var("OAH_TEST_CODEX_HOME")
            .into_iter()
            .map(|h| EnvVar { key: "CODEX_HOME".into(), value: h, in_keychain: false })
            .collect();
        let place = place();
        let t = Target { kind: AgentKind::Codex, location: &place, cwd: dir.to_str().unwrap(), binary: &binary, env: &e };

        let (said, session, failed) = live_turn_or_error(&t, None, None, "Remember the code word PLUM-7. Reply with just OK.");
        println!("driver: {said:?} on {session} ({})", failed.as_deref().unwrap_or("answered"));
        assert!(!session.is_empty(), "the thread starts either way");
        let signed_in = failed.is_none();
        let m = mark(&t, Some(&session), 0);
        println!("as the CLI opens: {m:?}");
        assert!(matches!(m, Mark::Past(n) if n >= 2), "the thread's file is found and counted: {m:?}");
        // Nothing of the driver's turn is the CLI's.
        assert_eq!(outcome(&t, &session, &m, None).events, Some(vec![]));

        // Its interface opens on the thread with oah's flags (a flag it didn't know ends it at once).
        let o = CliOptions {
            kind: AgentKind::Codex,
            session: Session::Resume(session.clone()),
            model: None,
            effort: Some("low".into()),
            permission: PermissionMode::Ask,
        };
        let cli = Live::open(&t, args(&o));
        cli.until("it draws", 60, || cli.screen.lock().unwrap().len() > 200);
        std::thread::sleep(Duration::from_secs(5));
        assert!(!cli.ended.load(std::sync::atomic::Ordering::SeqCst), "it's still up: {}", cli.screen.lock().unwrap());
        drop(cli);

        // Something said through the CLI on the same thread (its one-prompt mode: typing into
        // its interface needs the sign-in).
        let also = "Also remember a second code word, KIWI-3. Reply with just OK.";
        let said = ["exec", "--skip-git-repo-check", "resume", &session, also].map(String::from).to_vec();
        let out = oah_transport::run(
            &Spec { location: t.location, cwd: t.cwd, program: &binary, args: said, env: t.env },
            None,
            Duration::from_secs(180),
        )
        .unwrap();
        println!("the CLI, asked once: exit {:?} {:?}", out.code, out.stdout.trim());
        let read = texts(&outcome(&t, &session, &m, None).events.expect("its threads are read back"));
        println!("read back: {read:#?}");
        assert_eq!(read.iter().filter(|l| l.starts_with("you: ")).collect::<Vec<_>>(), [&format!("you: {also}")], "{read:?}");
        assert!(!read.iter().any(|l| l.contains("PLUM-7")), "{read:?}");
        assert_eq!(read.iter().any(|l| l.starts_with("agent: ")), signed_in, "an answer only when signed in: {read:?}");

        // The driver resumes the same thread.
        let (both, again, failed) =
            live_turn_or_error(&t, None, Some(&session), "Which code words did I ask you to remember? Reply with just the words.");
        println!("resumed {again}: {both:?} ({})", failed.as_deref().unwrap_or("answered"));
        assert_eq!(again, session);
        if signed_in {
            assert!(both.contains("PLUM-7") && both.contains("KIWI-3"), "{both}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The handover with the real Grok Build, signed in on this computer (three short turns):
    /// `cargo test -p oah-agents real_grok_handover -- --ignored --nocapture`.
    #[test]
    #[ignore]
    #[cfg(unix)]
    fn real_grok_handover() {
        let dir = scratch("grok");
        let mut e = vec![];
        env(AgentKind::Grok, PermissionMode::Ask, &mut e);
        handover(&Target { kind: AgentKind::Grok, location: &Location::Local, cwd: dir.to_str().unwrap(), binary: "", env: &e }, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The handover with the real Antigravity, signed in on this computer (three short turns):
    /// `cargo test -p oah-agents real_antigravity_handover -- --ignored --nocapture`.
    #[test]
    #[ignore]
    #[cfg(unix)]
    fn real_antigravity_handover() {
        let dir = scratch("agy");
        let mut e = vec![];
        env(AgentKind::Antigravity, PermissionMode::Ask, &mut e);
        handover(
            &Target { kind: AgentKind::Antigravity, location: &Location::Local, cwd: dir.to_str().unwrap(), binary: "", env: &e },
            None,
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The CLI's side of the handover on Windows itself, with a stand-in for Claude Code: a
    /// `.cmd` shim (as npm installs CLIs) around a script that prints its arguments, answers
    /// what's typed and saves the conversation as Claude Code does. It runs in a pseudo
    /// console with oah's arguments, and what's said in it reads back from where it was saved:
    /// `cargo test -p oah-agents real_windows_cli_round_trip -- --ignored --nocapture`.
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn real_windows_cli_round_trip() {
        use oah_transport::pty::{command_spec, Pty, PtyEvent};
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("oah cli {nanos}"));
        let work = dir.join("work");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(
            dir.join("fake.ps1"),
            r#"[Console]::Out.WriteLine("args:" + ($args -join " "))
$sid = $args[1]
$dir = Join-Path $env:CLAUDE_CONFIG_DIR "projects\p"
while ($true) {
  $line = [Console]::In.ReadLine()
  if ($null -eq $line -or $line -eq "/exit") { exit 0 }
  if ($line -eq "") { continue }
  [void][IO.Directory]::CreateDirectory($dir)
  $ts = [DateTime]::UtcNow.ToString("yyyy-MM-ddTHH:mm:ss.fffZ")
  $n = [guid]::NewGuid().ToString()
  $u = '{"type":"user","uuid":"u' + $n + '","timestamp":"' + $ts + '","message":{"role":"user","content":"' + $line + '"}}'
  $a = '{"type":"assistant","uuid":"a' + $n + '","timestamp":"' + $ts + '","message":{"id":"m","content":[{"type":"text","text":"heard: ' + $line + '"}]}}'
  [IO.File]::AppendAllText((Join-Path $dir ($sid + ".jsonl")), $u + "`n" + $a + "`n")
  [Console]::Out.WriteLine("said:" + $line)
}
"#,
        )
        .unwrap();
        let fake = dir.join("fake-cli.cmd");
        std::fs::write(
            &fake,
            "@echo off\r\npowershell -NoProfile -ExecutionPolicy Bypass -File \"%~dp0fake.ps1\" %*\r\nexit /b %ERRORLEVEL%\r\n",
        )
        .unwrap();
        let mut e =
            vec![EnvVar { key: "CLAUDE_CONFIG_DIR".into(), value: dir.join("claude").to_string_lossy().into_owned(), in_keychain: false }];
        env(AgentKind::Claude, PermissionMode::Ask, &mut e);
        let binary = fake.to_string_lossy().into_owned();
        let t = Target { kind: AgentKind::Claude, location: &Location::Local, cwd: work.to_str().unwrap(), binary: &binary, env: &e };
        // No watch where there's no POSIX shell to keep it: nothing only one could run is added.
        assert_eq!(watch(AgentKind::Claude, &Location::Local, "abc"), None);

        // One visit to the CLI on `session`: what its screen showed (escape codes out), and how it ended.
        let visit = |session: Session, say: &str| {
            let o = CliOptions { kind: AgentKind::Claude, session, model: None, effort: None, permission: PermissionMode::Ask };
            let spec = command_spec(&Spec { location: t.location, cwd: t.cwd, program: t.binary, args: args(&o), env: t.env }).unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            let pty = Pty::spawn(&spec, 160, 30, move |ev| drop(tx.send(ev))).unwrap();
            let (mut raw, mut asked, mut said, mut left) = (String::new(), false, false, false);
            let end = std::time::Instant::now() + Duration::from_secs(120);
            let code = loop {
                match rx
                    .recv_timeout(end.saturating_duration_since(std::time::Instant::now()))
                    .unwrap_or_else(|_| panic!("the CLI never ended: {raw:?}"))
                {
                    PtyEvent::Data(d) => raw.push_str(&d),
                    PtyEvent::Exit(code) => break code,
                }
                // The pseudo console asks where the cursor is before it shows anything.
                if !asked && raw.contains("\x1b[6n") {
                    asked = true;
                    pty.write(b"\x1b[1;1R").unwrap();
                }
                if !said && raw.contains("--permission-mode default") {
                    said = true;
                    pty.write(format!("{say}\r").as_bytes()).unwrap();
                }
                if !left && raw.contains(&format!("said:{say}")) {
                    left = true;
                    pty.write(b"/exit\r").unwrap();
                }
            };
            (raw, code)
        };
        let now = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;

        // The chat's session, resumed; what's said reads back.
        let m = mark(&t, Some("sess-1"), now());
        let (screen, code) = visit(Session::Resume("sess-1".into()), "remember this");
        assert!(screen.contains("args:--resume sess-1 --permission-mode default"), "{screen:?}");
        assert_eq!(code, Some(0));
        let left = outcome(&t, "sess-1", &m, None);
        let ev = left.events.expect("read back");
        assert!(matches!(&ev[0], ChatEvent::UserMessage { text, .. } if text == "remember this"), "{ev:?}");
        assert!(matches!(&ev[2], ChatEvent::Text { text, .. } if text == "heard: remember this"), "{ev:?}");
        assert_eq!((ev.len(), left.moved_to), (4, None));
        // A second visit reads back only what's new.
        let m = mark(&t, Some("sess-1"), now());
        visit(Session::Resume("sess-1".into()), "and this");
        let ev = outcome(&t, "sess-1", &m, None).events.unwrap();
        assert!(matches!(&ev[0], ChatEvent::UserMessage { text, .. } if text == "and this"), "{ev:?}");
        assert_eq!(ev.len(), 4);
        // A session oah picks: saved once something is said in it.
        let id = "0f0f0f0f-0000-4000-8000-000000000001";
        assert_eq!(session_saved(AgentKind::Claude, &Location::Local, &e, id), Some(false));
        let (screen, _) = visit(Session::New(id.into()), "hello");
        assert!(screen.contains(&format!("args:--session-id {id}")), "{screen:?}");
        assert_eq!(session_saved(AgentKind::Claude, &Location::Local, &e, id), Some(true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn watches_claude_where_a_shell_can_note_it() {
        let w = watch(AgentKind::Claude, &Location::Ssh { host: "box".into() }, "abc-1").unwrap();
        assert_eq!(w.log, "/tmp/oah-cli-abc-1.log");
        assert_eq!(w.args[0], "--settings");
        let settings: serde_json::Value = serde_json::from_str(&w.args[1]).unwrap();
        let command = settings["hooks"]["SessionStart"][0]["hooks"][0]["command"].as_str().unwrap();
        assert!(command.ends_with(">> /tmp/oah-cli-abc-1.log"), "{command}");
        assert!(watch(AgentKind::Codex, &Location::Local, "abc-1").is_none());
        assert!(watch(AgentKind::Claude, &Location::Local, "a b; rm").is_none());
    }

    /// The hook's command, run by a real shell as Claude Code runs it, writes lines the log
    /// reader takes: the time, then the session it was told of.
    #[test]
    #[cfg(unix)]
    fn the_watch_notes_each_conversation() {
        let tag = format!("test-{}", std::process::id());
        let w = watch(AgentKind::Claude, &Location::Local, &tag).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&w.args[1]).unwrap();
        let command = settings["hooks"]["SessionStart"][0]["hooks"][0]["command"].as_str().unwrap().to_string();
        for (id, source) in [("aaaa-1", "resume"), ("bbbb-2", "clear")] {
            use std::io::Write;
            let mut sh = std::process::Command::new("sh").arg("-c").arg(&command).stdin(std::process::Stdio::piped()).spawn().unwrap();
            let said = serde_json::json!({"session_id": id, "transcript_path": format!("/x/{id}.jsonl"), "hook_event_name": "SessionStart", "source": source});
            sh.stdin.take().unwrap().write_all(said.to_string().as_bytes()).unwrap();
            assert!(sh.wait().unwrap().success());
        }
        let t = Target { kind: AgentKind::Claude, location: &Location::Local, cwd: "/", binary: "", env: &[] };
        let seen = visits(&take_file(&t, &w.log).unwrap());
        assert_eq!(seen.iter().map(|v| v.session.as_str()).collect::<Vec<_>>(), ["aaaa-1", "bbbb-2"]);
        assert_eq!(seen[1].file.as_deref(), Some("/x/bbbb-2.jsonl"));
        assert!(seen[0].at > 1_700_000_000_000 && seen[0].at.is_multiple_of(1000));
        assert!(!std::path::Path::new(&w.log).exists(), "read once, then gone");
        assert!(visits("garbage\n12 {not json}\n").is_empty());
    }

    /// Claude Code's conversations, as files in its config folder: what was said after the
    /// CLI opened is read back, and a `/clear` or `/resume` there moves the chat to where the
    /// CLI ended, unless nothing was said in that one.
    #[test]
    fn reads_back_what_claude_saved_and_follows_it() {
        let dir = std::env::temp_dir().join(format!("oah-cli-out-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("projects/-w")).unwrap();
        let line = |ts: &str, kind: &str, uuid: &str, content: serde_json::Value| {
            serde_json::json!({"type": kind, "uuid": uuid, "timestamp": ts, "message": {"id": uuid, "content": content}}).to_string()
        };
        let text = |t: &str| serde_json::json!([{"type": "text", "text": t}]);
        let first = [
            line("2026-11-11T00:00:01.000Z", "user", "u0", serde_json::json!("from the chat")),
            line("2026-11-11T00:00:02.000Z", "assistant", "a0", text("before")),
            line("2026-11-11T00:10:01.000Z", "user", "u1", serde_json::json!("said in the CLI")),
            line("2026-11-11T00:10:02.000Z", "assistant", "a1", text("heard")),
        ];
        std::fs::write(dir.join("projects/-w/sess-1.jsonl"), first.join("\n")).unwrap();
        let second = [
            line("2026-11-11T00:20:01.000Z", "user", "u2", serde_json::json!("after the clear")),
            line("2026-11-11T00:20:02.000Z", "assistant", "a2", text("fresh")),
        ];
        std::fs::write(dir.join("projects/-w/sess-2.jsonl"), second.join("\n")).unwrap();
        let env = [EnvVar { key: "CLAUDE_CONFIG_DIR".into(), value: dir.to_string_lossy().into_owned(), in_keychain: false }];
        let t = Target { kind: AgentKind::Claude, location: &Location::Local, cwd: "/w", binary: "", env: &env };
        let said = |o: &Outcome| {
            o.events
                .clone()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|e| match e {
                    ChatEvent::UserMessage { text, .. } | ChatEvent::Text { text, .. } => Some(text),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let at = |ts: &str| crate::claude::iso_ms(ts).unwrap();
        // Not watched: the session it was opened on, from this computer's clock.
        let plain = outcome(&t, "sess-1", &Mark::Since(at("2026-11-11T00:10:00.000Z")), None);
        assert_eq!(said(&plain), ["said in the CLI", "heard"]);
        assert_eq!(plain.moved_to, None);
        // Watched: it resumed sess-1, was cleared into sess-2, then into one nothing was said in.
        let log = dir.join("watch.log");
        let entry = |secs: u64, id: &str| {
            format!("{secs} {}", serde_json::json!({"session_id": id, "transcript_path": dir.join(format!("projects/-w/{id}.jsonl"))}))
        };
        let (t1, t2) = (at("2026-11-11T00:10:00.000Z") / 1000, at("2026-11-11T00:20:00.000Z") / 1000);
        std::fs::write(&log, [entry(t1, "sess-1"), entry(t2, "sess-2"), entry(t2 + 60, "sess-3")].join("\n")).unwrap();
        let w = Watch { log: log.to_string_lossy().into_owned(), args: vec![] };
        let moved = outcome(&t, "sess-1", &Mark::Since(0), Some(&w));
        assert_eq!(said(&moved), ["said in the CLI", "heard", "after the clear", "fresh"]);
        assert_eq!(moved.moved_to.as_deref(), Some("sess-2"), "the last one with something in it");
        // A session that was never saved reads as nothing to show, not as an empty conversation.
        assert_eq!(outcome(&t, "sess-9", &Mark::Since(0), None), Outcome::default());
        assert_eq!(mark(&t, Some("sess-1"), 5), Mark::Since(5));
        assert_eq!(outcome(&t, "sess-1", &Mark::Unread, None), Outcome::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Codex's threads, as rollout files under its home folder: found by thread id, counted
    /// as the CLI opens, and read from there when it ends.
    #[test]
    fn reads_back_what_codex_saved() {
        let dir = std::env::temp_dir().join(format!("oah-cli-codex-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        let day = dir.join("sessions/2026/11/11");
        std::fs::create_dir_all(&day).unwrap();
        let file = day.join("rollout-2026-11-11T00-00-00-th-1.jsonl");
        let item = |kind: &str, message: &str| {
            serde_json::json!({"timestamp": "t", "type": "event_msg", "payload": {"type": kind, "message": message}}).to_string()
        };
        std::fs::write(&file, [item("user_message", "from the chat"), item("agent_message", "before")].join("\n") + "\n").unwrap();
        let env = [EnvVar { key: "CODEX_HOME".into(), value: dir.to_string_lossy().into_owned(), in_keychain: false }];
        let t = Target { kind: AgentKind::Codex, location: &Location::Local, cwd: "/w", binary: "", env: &env };
        let m = mark(&t, Some("th-1"), 0);
        assert_eq!(m, Mark::Past(2));
        let more = [item("user_message", "said in the CLI"), item("agent_message", "heard")].join("\n");
        std::fs::OpenOptions::new().append(true).open(&file).unwrap().write_all(more.as_bytes()).unwrap();
        let o = outcome(&t, "th-1", &m, None);
        let ev = o.events.unwrap();
        assert!(matches!(&ev[0], ChatEvent::UserMessage { text, .. } if text == "said in the CLI"));
        assert!(matches!(&ev[2], ChatEvent::Text { text, .. } if text == "heard"));
        assert_eq!(ev.len(), 4);
        assert_eq!(outcome(&t, "th-none", &Mark::Past(0), None).events, None);
        // A thread whose file isn't found as the CLI opens: nothing is taken for the CLI's
        // afterwards, rather than the whole thread.
        assert_eq!(mark(&t, Some("th-none"), 0), Mark::Unread);
        assert_eq!(outcome(&t, "th-1", &Mark::Unread, None), Outcome::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Grok Build's sessions, as folders under its home: its history is counted as the CLI
    /// opens (a session it's about to begin has none) and read from there when it ends.
    #[test]
    fn reads_back_what_grok_saved() {
        let dir = std::env::temp_dir().join(format!("oah-cli-grok-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        let session = dir.join("sessions/%2Fw/s-1");
        std::fs::create_dir_all(&session).unwrap();
        let user = |text: &str| {
            serde_json::json!({"type": "user", "content": [{"type": "text", "text": format!("<user_query>\n{text}\n</user_query>")}], "prompt_index": 0}).to_string()
        };
        let agent = |text: &str| serde_json::json!({"type": "assistant", "content": text}).to_string();
        std::fs::write(session.join("chat_history.jsonl"), [user("from the chat"), agent("before")].join("\n") + "\n").unwrap();
        let env = [EnvVar { key: "GROK_HOME".into(), value: dir.to_string_lossy().into_owned(), in_keychain: false }];
        let t = Target { kind: AgentKind::Grok, location: &Location::Local, cwd: "/w", binary: "", env: &env };
        let m = mark(&t, Some("s-1"), 0);
        assert_eq!(m, Mark::Past(2));
        assert_eq!(mark(&t, None, 0), Mark::Past(0));
        assert_eq!(mark(&t, Some("s-none"), 0), Mark::Unread);
        let more = [user("said in the CLI"), agent("heard")].join("\n");
        std::fs::OpenOptions::new().append(true).open(session.join("chat_history.jsonl")).unwrap().write_all(more.as_bytes()).unwrap();
        let ev = outcome(&t, "s-1", &m, None).events.unwrap();
        assert!(matches!(&ev[0], ChatEvent::UserMessage { text, .. } if text == "said in the CLI"));
        assert!(matches!(&ev[2], ChatEvent::Text { text, .. } if text == "heard"));
        assert_eq!(ev.len(), 4);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// An export in the form `opencode export` prints (1.19).
    #[test]
    fn reads_what_was_said_in_an_opencode_export() {
        let msg = |id: &str, role: &str, parts: serde_json::Value| serde_json::json!({"info": {"id": id, "role": role}, "parts": parts});
        let export = serde_json::json!({"info": {"id": "ses_1"}, "messages": [
            msg("m0", "user", serde_json::json!([{"type": "text", "text": "from the chat", "id": "p0"}])),
            msg("m1", "assistant", serde_json::json!([{"type": "text", "text": "before", "id": "p1"}])),
            msg("m2", "user", serde_json::json!([{"type": "text", "text": "list the files", "id": "p2"}, {"type": "text", "text": "<file contents>", "synthetic": true, "id": "p3"}])),
            msg("m3", "assistant", serde_json::json!([
                {"type": "step-start", "id": "p4"},
                {"type": "reasoning", "text": "I'll list them.", "id": "p5"},
                {"type": "tool", "tool": "bash", "callID": "c1", "id": "p6", "state": {"status": "completed", "input": {"command": "ls"}, "output": "a.rs\nb.rs", "title": "ls"}},
                {"type": "tool", "tool": "read", "id": "p7", "state": {"status": "error", "input": {"filePath": "/w/c.rs"}, "error": "no such file"}},
                {"type": "text", "text": "Two files.", "id": "p8"},
                {"type": "step-finish", "id": "p9"},
            ])),
        ]});
        let ev = opencode_events(&export, 2);
        let kinds: Vec<String> = ev.iter().map(|e| serde_json::to_value(e).unwrap()["type"].as_str().unwrap().to_string()).collect();
        assert_eq!(
            kinds,
            [
                "userMessage",
                "turnStarted",
                "reasoning",
                "toolStarted",
                "toolFinished",
                "toolStarted",
                "toolFinished",
                "text",
                "turnFinished"
            ]
        );
        assert!(matches!(&ev[0], ChatEvent::UserMessage { id, text, .. } if id == "m2" && text == "list the files"));
        assert!(matches!(&ev[3], ChatEvent::ToolStarted { tool, .. } if tool.kind == ToolKind::Bash && tool.title == "ls"));
        assert!(matches!(&ev[4], ChatEvent::ToolFinished { ok: true, output, .. } if output == "a.rs\nb.rs"));
        assert!(
            matches!(&ev[5], ChatEvent::ToolStarted { tool, .. } if tool.kind == ToolKind::Read && tool.path.as_deref() == Some("/w/c.rs"))
        );
        assert!(matches!(&ev[6], ChatEvent::ToolFinished { ok: false, output, .. } if output == "no such file"));
        assert!(opencode_events(&export, 4).is_empty());
    }

    #[test]
    fn tells_a_saved_session_from_one_that_never_was() {
        let dir = std::env::temp_dir().join(format!("oah-cli-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("projects/-home-me-code")).unwrap();
        std::fs::write(dir.join("projects/-home-me-code/1111-aaaa.jsonl"), "{}\n").unwrap();
        let env = [EnvVar { key: "CLAUDE_CONFIG_DIR".into(), value: dir.to_string_lossy().into_owned(), in_keychain: false }];
        assert_eq!(session_saved(AgentKind::Claude, &Location::Local, &env, "1111-aaaa"), Some(true));
        assert_eq!(session_saved(AgentKind::Claude, &Location::Local, &env, "2222-bbbb"), Some(false));
        assert_eq!(session_saved(AgentKind::Claude, &Location::Local, &env, "../x"), Some(false));
        assert_eq!(session_saved(AgentKind::Codex, &Location::Local, &env, "1111-aaaa"), None);
        // The same check as the script a WSL distro or an SSH host runs.
        #[cfg(unix)]
        {
            let script = |id: &str| {
                format!(r#"for f in "${{CLAUDE_CONFIG_DIR:-$HOME/.claude}}"/projects/*/{id}.jsonl; do [ -e "$f" ] && exit 0; done; exit 1"#)
            };
            let run = |id: &str| {
                std::process::Command::new("sh").arg("-c").arg(script(id)).env("CLAUDE_CONFIG_DIR", &dir).status().unwrap().code()
            };
            assert_eq!(run("1111-aaaa"), Some(0));
            assert_eq!(run("2222-bbbb"), Some(1));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
