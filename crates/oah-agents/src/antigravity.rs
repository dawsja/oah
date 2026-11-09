//! Google Antigravity's CLI (`agy`): what differs from the other ACP agent (OpenCode).
//!
//! - Chats run over ACP (`agy acp`, see [`crate::acp`]).
//! - Models: `agy models` prints `id<TAB>name` once per effort (`claude-opus-5-5-high`,
//!   "Claude Opus 5.5 (High)"); a session's model option has the base model and a separate
//!   thinking level, so a model with more than one effort is listed once, with its efforts.
//! - Sign-in: `agy acp --login` in a terminal. Who: the CLI's OAuth token, which names the
//!   account (`id_token_email`): the file `~/.gemini/antigravity-cli/antigravity-oauth-token`
//!   (in WSL, over SSH, or without a keyring), else Windows Credential Manager (`gemini:antigravity`)
//!   or the Linux keyring (`secret-tool`, service `gemini`, user `antigravity`). Only the email is
//!   read out of it; the token never leaves the CLI's machine.
//! - Limits: `agy -p /usage --output-format json`, a built-in command (1.1.11 and newer), not a
//!   prompt: per model group, a weekly and a 5-hour window.
//! - One-off questions: `agy --print= --sandbox` with stream-json in and out.

use std::sync::Mutex;
use std::time::Duration;

use oah_core::settings::{EnvVar, Location};
use oah_transport::Spec;
use serde_json::{json, Value};

use crate::account::{Account, LimitWindow, Limits};
use crate::driver::{DriverResult, StartOptions};
use crate::event::{ModelOption, SlashCommand};

pub const NOT_SIGNED_IN: &str =
    "Antigravity isn't signed in. Run `agy` in a terminal and sign in with Google (Settings → Providers → Antigravity → Sign in).";

/// Effort suffixes `agy` puts on model ids, lowest first.
const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// Chats need `session/resume` and `session/list` in `agy acp` (1.5.2).
pub const MINIMUM: &str = "1.5.2";
/// Older `agy` takes `/usage` in print mode as a prompt (and bills it).
const USAGE_MINIMUM: &str = "1.1.11";

fn var(key: &str, value: &str) -> EnvVar {
    EnvVar { key: key.into(), value: value.into(), in_keychain: false }
}

/// The environment every `agy` run gets: no self-update mid-run (it wants the word `true`), and
/// no Gemini API key from the surroundings, which switches `agy` to key sign-in that its models
/// refuse (401). A key set in Settings → Providers → Antigravity is kept.
pub fn acp_env(env: &mut Vec<EnvVar>) {
    let set = |env: &Vec<EnvVar>, k: &str| env.iter().any(|e| e.key == k);
    if !set(env, "AGY_CLI_DISABLE_AUTO_UPDATE") {
        env.push(var("AGY_CLI_DISABLE_AUTO_UPDATE", "true"));
    }
    for k in ["GEMINI_API_KEY", "GOOGLE_API_KEY"] {
        if !set(env, k) {
            env.push(var(k, ""));
        }
    }
}

fn program(binary: &str) -> &str {
    if binary.trim().is_empty() {
        "agy"
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

// ---- Sign-in ---------------------------------------------------------------------------------

/// The terminal command that signs in.
pub fn login_line(prog: &str) -> String {
    format!("{prog} acp --login")
}

/// Who `agy` is signed in as on `location`, from its own token store.
pub fn account(location: &Location, env: &[EnvVar]) -> Account {
    let found = match location {
        Location::Local => local_token_email(),
        remote => remote_token_email(remote, env),
    };
    match found {
        Ok(Some(Found::Token(email))) => Account { signed_in: true, label: email.or(Some("Google".into())), plan: None, error: None },
        Ok(Some(Found::ApiKey)) => Account { signed_in: true, label: Some("API key".into()), plan: None, error: None },
        Ok(None) => Account { signed_in: false, label: None, plan: None, error: None },
        Err(e) => Account { error: Some(e), ..Default::default() },
    }
}

#[derive(Debug, PartialEq)]
enum Found {
    /// An OAuth sign-in, and its email when the token names it.
    Token(Option<String>),
    /// `settings.json` picks a model provider: an API key, not a Google sign-in.
    ApiKey,
}

/// The token's account email: `id_token_email`, or the verified `email` in its `id_token`.
fn email_of(token: &str) -> Option<String> {
    let v: Value = serde_json::from_str(token.trim()).ok()?;
    if let Some(e) = v["id_token_email"].as_str().filter(|e| e.contains('@')) {
        return Some(e.to_string());
    }
    let jwt = v["id_token"].as_str().or(v["token"]["id_token"].as_str())?;
    use base64::Engine;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(jwt.split('.').nth(1)?.trim_end_matches('=')).ok()?;
    let claims: Value = serde_json::from_slice(&payload).ok()?;
    (claims["email_verified"] == true).then(|| claims["email"].as_str().map(String::from)).flatten()
}

/// A keyring value as go-keyring stores it: `go-keyring-base64:…`, `go-keyring-encoded:<hex>`,
/// or the JSON itself.
fn keyring_value(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if let Some(b) = raw.strip_prefix("go-keyring-base64:") {
        use base64::Engine;
        return base64::engine::general_purpose::STANDARD.decode(b).ok().and_then(|v| String::from_utf8(v).ok());
    }
    if let Some(h) = raw.strip_prefix("go-keyring-encoded:") {
        let bytes: Option<Vec<u8>> = (0..h.len()).step_by(2).map(|i| u8::from_str_radix(h.get(i..i + 2)?, 16).ok()).collect();
        return bytes.and_then(|b| String::from_utf8(b).ok());
    }
    Some(raw.to_string())
}

/// `~/.gemini/antigravity-cli/settings.json` names a model provider (an API key), or is
/// unreadable JSON (as `agy` reads it then).
fn uses_api_key(settings: &str) -> bool {
    match serde_json::from_str::<Value>(settings) {
        Ok(v) => v["modelProvider"].as_str().is_some_and(|p| !p.is_empty()) || v["modelProvider"] == true,
        Err(_) => !settings.trim().is_empty(),
    }
}

fn local_token_email() -> Result<Option<Found>, String> {
    let dir = oah_core::paths::home_dir().join(".gemini").join("antigravity-cli");
    if std::fs::read_to_string(dir.join("settings.json")).is_ok_and(|s| uses_api_key(&s)) {
        return Ok(Some(Found::ApiKey));
    }
    if let Ok(t) = std::fs::read_to_string(dir.join("antigravity-oauth-token")) {
        if !t.trim().is_empty() {
            return Ok(Some(Found::Token(email_of(&t))));
        }
    }
    Ok(keyring_token()?.map(|t| Found::Token(email_of(&t))))
}

/// The token in the OS keyring, where `agy` keeps it on a desktop.
#[cfg(windows)]
fn keyring_token() -> Result<Option<String>, String> {
    let entry = keyring::Entry::new_with_target("gemini:antigravity", "gemini", "antigravity").map_err(|e| e.to_string())?;
    match entry.get_secret() {
        Ok(bytes) => Ok(String::from_utf8(bytes).ok().and_then(|s| keyring_value(&s))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("couldn't read Windows Credential Manager: {e}")),
    }
}

#[cfg(target_os = "linux")]
fn keyring_token() -> Result<Option<String>, String> {
    // `secret-tool` (libsecret) talks to the desktop's keyring; none installed, or no keyring
    // running, means `agy` uses its file, read above.
    let mut cmd = std::process::Command::new("secret-tool");
    cmd.args(["lookup", "service", "gemini", "username", "antigravity"]).stdin(std::process::Stdio::null());
    match cmd.output() {
        Ok(o) if o.status.success() => {
            Ok(String::from_utf8(o.stdout).ok().filter(|s| !s.trim().is_empty()).and_then(|s| keyring_value(&s)))
        }
        _ => Ok(None),
    }
}

#[cfg(target_os = "macos")]
fn keyring_token() -> Result<Option<String>, String> {
    let o = std::process::Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", "gemini", "-a", "antigravity", "-w"])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned()).and_then(|s| keyring_value(&s)))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn keyring_token() -> Result<Option<String>, String> {
    Ok(None)
}

/// In WSL or over SSH `agy` keeps its token in the file. Only the email comes back.
const REMOTE_ACCOUNT: &str = r#"d="${HOME}/.gemini/antigravity-cli"
if [ -f "$d/settings.json" ] && grep -Eq '"modelProvider"[[:space:]]*:[[:space:]]*(true|"[^"])' "$d/settings.json"; then echo api-key; exit 0; fi
f="$d/antigravity-oauth-token"
[ -s "$f" ] || { echo none; exit 0; }
e=$(grep -o '"id_token_email"[[:space:]]*:[[:space:]]*"[^"]*"' "$f" | head -n 1 | sed 's/.*"\([^"]*\)"$/\1/')
echo "token $e""#;

fn remote_token_email(location: &Location, env: &[EnvVar]) -> Result<Option<Found>, String> {
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
        l => l.strip_prefix("token").map(|e| Found::Token(Some(e.trim().to_string()).filter(|e| e.contains('@')))),
    }
}

// ---- Limits ----------------------------------------------------------------------------------

/// Where `/usage` came back as a billed prompt: never asked there again while oah runs.
static PROMPTED: Mutex<Vec<Location>> = Mutex::new(Vec::new());

/// The plan's windows on `location`, from `agy`'s `/usage`.
pub fn limits(binary: &str, location: &Location, env: &[EnvVar]) -> Limits {
    let fail = |e: String| Limits { error: Some(e), ..Default::default() };
    if PROMPTED.lock().unwrap().contains(location) {
        return fail(
            "This Antigravity CLI answers /usage as a prompt, so oah stopped asking. Update it (`agy update`) and restart oah.".into(),
        );
    }
    let prog = program(binary);
    let cwd = home_cwd(location);
    let mut env = env.to_vec();
    acp_env(&mut env);
    let run = |args: &[&str], limit: u64| {
        let args = args.iter().map(|a| a.to_string()).collect();
        oah_transport::run(&Spec { location, cwd: &cwd, program: prog, args, env: &env }, None, Duration::from_secs(limit))
    };
    match run(&["--version"], 10) {
        Ok(o) if o.ok() => {
            if let Some(v) = crate::probe::parse_version(&o.stdout).filter(|v| crate::probe::older(v, USAGE_MINIMUM)) {
                return fail(format!("Antigravity {v} can't report usage. Update to {USAGE_MINIMUM} or newer (`agy update`)."));
            }
        }
        Ok(o) if o.code == Some(127) => return Limits { missing: true, ..Default::default() },
        Ok(_) => {}
        Err(e) => return fail(e),
    }
    let out = match run(&["-p", "/usage", "--output-format", "json", "--print-timeout", "20s"], 35) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    match parse_usage(&out.stdout, &out.stderr) {
        Ok(windows) => Limits { windows, ..Default::default() },
        Err(UsageError::Prompted) => {
            PROMPTED.lock().unwrap().push(location.clone());
            fail("This Antigravity CLI answered /usage as a prompt instead of a command, so oah stopped asking. Update it (`agy update`) and restart oah.".into())
        }
        Err(UsageError::Other(e)) => fail(e),
    }
}

#[derive(Debug, PartialEq)]
pub enum UsageError {
    /// It ran a model turn for it.
    Prompted,
    Other(String),
}

/// `/usage`'s JSON line → windows: per model group, its 5-hour and weekly buckets, as
/// "<group> · 5 hours" and "<group> · this week" (`remaining_fraction` turned into used).
/// Unmetered (`disabled`) buckets are left out.
pub fn parse_usage(stdout: &str, stderr: &str) -> Result<Vec<LimitWindow>, UsageError> {
    let lines: Vec<Value> = stdout.lines().filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok()).collect();
    if let Some(v) = lines.iter().find(|v| v["command"]["name"] == "usage" || v["command"]["data"]["groups"].is_array()) {
        let mut out = Vec::new();
        for g in v["command"]["data"]["groups"].as_array().into_iter().flatten() {
            let group = g["name"].as_str().unwrap_or("").trim();
            let group = group.strip_suffix(" Models").or(group.strip_suffix(" models")).unwrap_or(group);
            let mut buckets: Vec<&Value> = g["buckets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|b| b["disabled"] != true && b["remaining_fraction"].is_number())
                .collect();
            // The short window first, as the other agents list theirs.
            buckets.sort_by_key(|b| {
                if b["window"] == "5h" {
                    0
                } else if b["window"] == "weekly" {
                    1
                } else {
                    2
                }
            });
            for b in buckets {
                let window = match b["window"].as_str() {
                    Some("5h") => "5 hours".to_string(),
                    Some("weekly") => "this week".to_string(),
                    _ => b["name"].as_str().unwrap_or("limit").to_string(),
                };
                let used = (1.0 - b["remaining_fraction"].as_f64().unwrap_or(1.0)) * 100.0;
                out.push(LimitWindow {
                    label: if group.is_empty() { window } else { format!("{group} · {window}") },
                    used_percent: used.round().clamp(0.0, 100.0),
                    resets_at: crate::account::reset_ms(&b["reset_time"]),
                });
            }
        }
        return Ok(out);
    }
    if lines.iter().any(|v| v["num_turns"].as_u64().is_some_and(|n| n > 0) || v["conversation_id"].as_str().is_some_and(|c| !c.is_empty()))
    {
        return Err(UsageError::Prompted);
    }
    let text = format!(
        "{stdout}
{stderr}"
    );
    let lower = text.to_lowercase();
    let status = lines.iter().find_map(|v| v["error"].as_str().map(String::from));
    Err(UsageError::Other(
        if lower.contains("unauthenticated")
            || lower.contains("authentication required")
            || lower.contains("not logged in")
            || lower.contains("401")
        {
            NOT_SIGNED_IN.to_string()
        } else if lower.contains("permission_denied") || lower.contains("403") {
            "Antigravity says this account has no plan with usage limits.".to_string()
        } else if lower.contains("resource_exhausted") || lower.contains("429") {
            "Antigravity is rate limiting requests; try again in a little while.".to_string()
        } else {
            status
                .or_else(|| text.lines().map(str::trim).rfind(|l| !l.is_empty()).map(String::from))
                .unwrap_or_else(|| "Antigravity didn't report usage.".into())
        },
    ))
}

// ---- One-off questions -----------------------------------------------------------------------

/// The quick model for one-off questions: Flash at its lowest effort.
pub fn pick_cheap(models: &[ModelOption]) -> Option<(String, Option<String>, String)> {
    let m = models.iter().find(|m| m.id.contains("flash")).or(models.first())?;
    let effort = m.efforts.first().cloned();
    let label = match &effort {
        Some(e) => format!("{} ({e})", m.name),
        None => m.name.clone(),
    };
    Some((m.id.clone(), effort, label))
}

/// Arguments for a one-off question: print mode, stream-json in and out, sandboxed commands.
pub fn oneshot_args(model: Option<&str>, effort: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> =
        ["--print=", "--input-format=stream-json", "--output-format=stream-json", "--sandbox"].map(String::from).to_vec();
    if let Some(m) = model {
        a.extend(["--model".into(), m.into()]);
    }
    if let Some(e) = effort {
        a.extend(["--effort".into(), e.into()]);
    }
    a
}

/// The question as `agy`'s stream-json input.
pub fn oneshot_input(question: &str) -> String {
    format!(
        "{}
",
        json!({"event": "user", "message": {"role": "user", "content": [{"type": "text", "text": question}]}})
    )
}

/// The answer from the final `result` event.
pub fn oneshot_answer(stdout: &str) -> Result<String, String> {
    for line in stdout.lines().rev() {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
        if v["event"] == "result" {
            let r = &v["result"];
            return match r["status"].as_str() {
                Some("SUCCESS") => Ok(r["response"].as_str().unwrap_or("").to_string()),
                _ => Err(r["error"].as_str().or(r["status"].as_str()).unwrap_or("no answer").to_string()),
            };
        }
    }
    Err("Antigravity gave no answer".into())
}

/// `agy models`: one `id<TAB>name` line per model and effort, after a "Fetching…" line.
pub fn parse_models(out: &str) -> Vec<ModelOption> {
    let mut models: Vec<ModelOption> = Vec::new();
    for line in out.lines() {
        let Some((id, name)) = line.trim().split_once('\t') else { continue };
        let (id, name) = (id.trim(), name.trim());
        if id.is_empty() || id.contains(' ') {
            continue;
        }
        // `claude-opus-5-5-high` / "Claude Opus 5.5 (High)" → base `claude-opus-5-5`, effort high.
        let split = EFFORTS.iter().find_map(|e| {
            let base = id.strip_suffix(e)?.strip_suffix('-')?;
            let shown = name.strip_suffix(')').and_then(|n| n.rsplit_once(" (")).map(|(n, _)| n).unwrap_or(name);
            Some((base, shown, *e))
        });
        match split {
            Some((base, shown, effort)) => match models.iter_mut().find(|m| m.id == base) {
                Some(m) => m.efforts.push(effort.to_string()),
                None => models.push(ModelOption {
                    id: base.to_string(),
                    name: shown.to_string(),
                    efforts: vec![effort.to_string()],
                    // The full id and name, for a model that turns out to have one effort.
                    description: format!("{id}\t{name}"),
                    ..Default::default()
                }),
            },
            None => models.push(ModelOption { id: id.to_string(), name: name.to_string(), ..Default::default() }),
        }
    }
    // A model with one effort is that one id (as `agy`'s own picker lists it).
    for m in models.iter_mut() {
        if m.efforts.len() == 1 {
            if let Some((id, name)) = m.description.split_once('\t') {
                m.id = id.to_string();
                m.name = name.to_string();
            }
            m.efforts.clear();
        }
        m.efforts.sort_by_key(|e| EFFORTS.iter().position(|x| x == e).unwrap_or(99));
        m.description.clear();
    }
    models
}

/// The models `agy` offers where it runs.
pub fn models(binary: &str, location: &Location, cwd: &str, env: &[EnvVar]) -> DriverResult<Vec<ModelOption>> {
    let program = program(binary);
    let mut env = env.to_vec();
    acp_env(&mut env);
    let out = oah_transport::run(&Spec { location, cwd, program, args: vec!["models".into()], env: &env }, None, Duration::from_secs(45))?;
    if !out.ok() {
        let why = out.stderr.lines().chain(out.stdout.lines()).map(str::trim).rfind(|l| !l.is_empty());
        return Err(match why {
            _ if out.code == Some(127) => format!("{program} isn't installed there"),
            Some(w) => w.to_string(),
            None => "agy models failed".into(),
        });
    }
    let models = parse_models(&out.stdout);
    if models.is_empty() {
        return Err(NOT_SIGNED_IN.into());
    }
    Ok(models)
}

/// `agy`'s commands before a chat's first `available_commands_update`. Its skills come once a
/// chat runs.
pub fn commands() -> Vec<SlashCommand> {
    let c = |name: &str, description: &str| SlashCommand {
        name: name.into(),
        description: description.into(),
        hint: String::new(),
        skill: false,
    };
    vec![
        c("goal", "Run until the specified goal is completely finished."),
        c("compact", "Compact the conversation now (summarize history to free up the context window)."),
        c("schedule", "Run an instruction on a recurring schedule or as a one-time timer."),
        c("browser", "Invoke a browser agent for web tasks."),
        c("plan", "Plan carefully before executing a task."),
        c("grill-me", "Interview me to align on a plan."),
        c("teamwork", "Invoke a team of agents to autonomously tackle large projects."),
        c("learn", "Reflect on recent successes or corrections to capture reusable skills or rules."),
        c("boost", "Invoke the Boost multi-agent orchestrator for complex tasks."),
    ]
}

/// The models and commands for a chat that hasn't started. From `agy models`, not a session:
/// `agy` keeps every session it opens as a conversation, even an empty one.
pub fn catalog(opts: &StartOptions) -> DriverResult<(Vec<ModelOption>, Vec<SlashCommand>)> {
    Ok((models(&opts.binary, &opts.location, &opts.cwd, &opts.env)?, commands()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const LISTING: &str = "Fetching available models...\n\
gemini-4-argon-medium\tGemini 4 Argon (Medium)\n\
gemini-4-argon-high\tGemini 4 Argon (High)\n\
gemini-4-argon-low\tGemini 4 Argon (Low)\n\
claude-opus-5-5-low\tClaude Opus 5.5 (Low)\n\
claude-opus-5-5-high\tClaude Opus 5.5 (High)\n\
gpt-oss-120b-medium\tGPT-OSS 120B (Medium)\n";

    #[test]
    fn groups_efforts_under_their_model() {
        let m = parse_models(LISTING);
        let ids: Vec<&str> = m.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["gemini-4-argon", "claude-opus-5-5", "gpt-oss-120b-medium"]);
        assert_eq!(m[0].name, "Gemini 4 Argon");
        assert_eq!(m[0].efforts, ["low", "medium", "high"]);
        assert_eq!(m[1].efforts, ["low", "high"]);
        // One effort: kept whole, as the session's model option has it.
        assert_eq!((m[2].name.as_str(), m[2].efforts.len()), ("GPT-OSS 120B (Medium)", 0));
        assert!(m.iter().all(|m| m.description.is_empty()));
    }

    /// `agy -p /usage --output-format json` from 1.5.3 (Google AI Ultra, nothing used yet).
    const USAGE: &str = r##"{"conversation_id":"","status":"SUCCESS","response":"Gemini Models\tWeekly Limit Remaining\t100%\t2026-11-16T16:14:12Z\nGemini Models\tFive Hour Limit Remaining\t100%\t2026-11-09T21:14:12Z\nClaude and GPT models\tWeekly Limit Remaining\t100%\t2026-11-16T16:14:12Z\nClaude and GPT models\tFive Hour Limit Remaining\t100%\t2026-11-09T21:14:12Z\n","duration_seconds":0,"num_turns":0,"usage":{"input_tokens":0,"output_tokens":0,"thinking_tokens":0,"cache_read_tokens":0,"total_tokens":0},"command":{"name":"usage","data":{"description":"Within each group, models share a weekly limit and a 5-hour limit. Quota is consumed proportionally to the cost of the tokens. Thus, limits will last longer with shorter tasks or using more cost-effective models. The 5-hour limit smooths out aggregate demand to fairly distribute global capacity across all users, while your weekly limit is tied directly to your individual tier.","groups":[{"name":"Gemini Models","description":"Models within this group: Gemini Flash, Gemini Pro","buckets":[{"id":"gemini-weekly","name":"Weekly Limit Remaining","window":"weekly","remaining_fraction":1,"reset_time":"2026-11-16T16:14:12Z"},{"id":"gemini-5h","name":"Five Hour Limit Remaining","window":"5h","remaining_fraction":1,"reset_time":"2026-11-09T21:14:12Z"}]},{"name":"Claude and GPT models","description":"Models within this group: Claude Opus, Claude Sonnet, GPT-OSS","buckets":[{"id":"3p-weekly","name":"Weekly Limit Remaining","window":"weekly","remaining_fraction":1,"reset_time":"2026-11-16T16:14:12Z"},{"id":"3p-5h","name":"Five Hour Limit Remaining","window":"5h","remaining_fraction":1,"reset_time":"2026-11-09T21:14:12Z"}]}]}}}"##;

    #[test]
    fn reads_usage_windows() {
        let w = parse_usage(USAGE, "").unwrap();
        let labels: Vec<&str> = w.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(labels, ["Gemini · 5 hours", "Gemini · this week", "Claude and GPT · 5 hours", "Claude and GPT · this week"]);
        assert!(w.iter().all(|w| w.used_percent == 0.0));
        assert_eq!(w[0].resets_at, Some(1_794_258_852_000)); // 2026-11-09T21:14:12Z
                                                             // Used is what isn't left; unmetered buckets are left out.
        let some = USAGE.replacen("\"remaining_fraction\":1,", "\"remaining_fraction\":0.27,", 1).replacen(
            "\"remaining_fraction\":1,",
            "\"disabled\":true,\"remaining_fraction\":1,",
            1,
        );
        let w = parse_usage(&some, "").unwrap();
        assert_eq!((w.len(), w[0].label.as_str(), w[0].used_percent), (3, "Gemini · this week", 73.0));
    }

    #[test]
    fn usage_problems_in_plain_words() {
        let billed = r#"{"conversation_id":"c-1","status":"SUCCESS","response":"Your usage is…","num_turns":1}"#;
        assert_eq!(parse_usage(billed, "").unwrap_err(), UsageError::Prompted);
        let out = r#"AGY_ERROR:{"status":"UNAUTHENTICATED","error_code":401}"#;
        assert_eq!(parse_usage(out, "").unwrap_err(), UsageError::Other(NOT_SIGNED_IN.into()));
        let denied = r#"{"status":"ERROR","error":"PERMISSION_DENIED: no subscription"}"#;
        assert!(matches!(parse_usage(denied, ""), Err(UsageError::Other(e)) if e.contains("no plan")));
        assert!(matches!(parse_usage("", "boom"), Err(UsageError::Other(e)) if e == "boom"));
    }

    #[test]
    fn reads_who_from_the_token() {
        let file = r#"{"token":{"access_token":"a","token_type":"Bearer","refresh_token":"r","expiry":"2026-11-09T11:08:22-06:00"},"auth_method":"consumer","id_token_email":"me@gmail.com"}"#;
        assert_eq!(email_of(file).as_deref(), Some("me@gmail.com"));
        // Else the verified email in the id_token.
        use base64::Engine;
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"iss":"https://accounts.google.com","email":"you@x.dev","email_verified":true}"#);
        assert_eq!(email_of(&format!(r#"{{"token":{{"id_token":"h.{claims}.s"}}}}"#)).as_deref(), Some("you@x.dev"));
        let unverified = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"email":"you@x.dev","email_verified":false}"#);
        assert_eq!(email_of(&format!(r#"{{"id_token":"h.{unverified}.s"}}"#)), None);
        // go-keyring's encodings.
        let b64 = base64::engine::general_purpose::STANDARD.encode(file);
        assert_eq!(keyring_value(&format!("go-keyring-base64:{b64}")).as_deref(), Some(file));
        let hex: String = "{}".bytes().map(|b| format!("{b:02x}")).collect();
        assert_eq!(keyring_value(&format!("go-keyring-encoded:{hex}")).as_deref(), Some("{}"));
        // API keys, and what the WSL / SSH script prints.
        assert!(uses_api_key(r#"{"modelProvider":"gemini"}"#));
        assert!(!uses_api_key(r#"{"model":"Gemini 4 Argon (Medium)"}"#));
        assert_eq!(parse_remote_account("token me@gmail.com\n"), Some(Found::Token(Some("me@gmail.com".into()))));
        assert_eq!(parse_remote_account("token \n"), Some(Found::Token(None)));
        assert_eq!(parse_remote_account("none\n"), None);
        assert_eq!(parse_remote_account("api-key"), Some(Found::ApiKey));
    }

    #[test]
    fn one_off_questions() {
        let m = parse_models(LISTING);
        assert_eq!(pick_cheap(&m), Some(("gemini-4-argon".into(), Some("low".into()), "Gemini 4 Argon (low)".into())));
        let flash = parse_models("gemini-3.8-flash-high\tGemini 3.8 Flash (High)\ngemini-3.8-flash-low\tGemini 3.8 Flash (Low)\n");
        let m: Vec<ModelOption> = parse_models(LISTING).into_iter().chain(flash).collect();
        assert_eq!(pick_cheap(&m).unwrap().0, "gemini-3.8-flash");
        assert_eq!(oneshot_args(Some("gemini-3.8-flash"), Some("low"))[4..], ["--model", "gemini-3.8-flash", "--effort", "low"]);
        let input: Value = serde_json::from_str(oneshot_input("Hi").trim()).unwrap();
        assert_eq!(input["message"]["content"][0]["text"], "Hi");
        // From agy 1.5.3.
        let out = r#"{"event": "init", "conversation_id": "7207", "init": {"model": "gemini-3.8-flash"}}
{"event": "step_update", "step_update": {"step_index": 1, "state": "ACTIVE", "step_type": "agent_response", "text_delta": "OK"}}
{"event": "result", "result": {"conversation_id": "7207", "status": "SUCCESS", "response": "OK\n", "num_turns": 1}}"#;
        assert_eq!(oneshot_answer(out).as_deref(), Ok("OK\n"));
        assert!(oneshot_answer("").is_err());
    }

    #[test]
    fn clears_a_stray_api_key_but_keeps_yours() {
        let mut env = vec![];
        acp_env(&mut env);
        let get = |env: &Vec<EnvVar>, k: &str| env.iter().find(|e| e.key == k).map(|e| e.value.clone());
        assert_eq!(get(&env, "AGY_CLI_DISABLE_AUTO_UPDATE").as_deref(), Some("true"));
        assert_eq!(get(&env, "GEMINI_API_KEY").as_deref(), Some(""));
        let mut env = vec![EnvVar { key: "GEMINI_API_KEY".into(), value: "k".into(), in_keychain: false }];
        acp_env(&mut env);
        assert_eq!(env.iter().filter(|e| e.key == "GEMINI_API_KEY").count(), 1);
        assert_eq!(get(&env, "GEMINI_API_KEY").as_deref(), Some("k"));
    }

    /// Against the real `agy` here (ignored): who's signed in, the plan's windows, the models.
    #[test]
    #[ignore]
    fn real_account_limits_models() {
        let loc = Location::Local;
        let a = account(&loc, &[]);
        eprintln!("account: signed_in={} label={:?} error={:?}", a.signed_in, a.label, a.error);
        assert!(a.signed_in);
        let l = limits("", &loc, &[]);
        eprintln!("limits: error={:?} missing={}", l.error, l.missing);
        for w in &l.windows {
            eprintln!("  {} {}% resets {:?}", w.label, w.used_percent, w.resets_at);
        }
        assert_eq!(l.windows.len(), 4);
        let m = models("", &loc, "/tmp", &[]).unwrap();
        eprintln!("models: {}", m.iter().map(|m| format!("{} {:?}", m.id, m.efforts)).collect::<Vec<_>>().join(", "));
        assert!(m.iter().any(|m| m.efforts.len() > 1));
    }

    #[test]
    fn nothing_listed_is_nothing() {
        assert!(parse_models("Fetching available models...\n").is_empty());
    }
}
