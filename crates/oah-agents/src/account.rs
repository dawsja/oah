//! Who each CLI is signed in as, and its plan limits.
//!
//! - Sign-in: `claude auth status --json` and `codex login status` (run where the project is);
//!   a ChatGPT sign-in's email and plan come from `codex app-server`'s `account/read`.
//! - Claude limits: `GET https://api.anthropic.com/api/oauth/usage` with the CLI's own OAuth
//!   token from `~/.claude/.credentials.json` (this computer only).
//! - Codex limits: `account/read` and `account/rateLimits/read` from a short-lived
//!   `codex app-server`.
//! - Antigravity: see [`crate::antigravity`].

use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

use oah_core::settings::{AgentKind, EnvVar, Location};
use oah_transport::Spec;
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub signed_in: bool,
    /// Email, or how it's signed in ("API key").
    pub label: Option<String>,
    /// The plan: "max", "pro"…
    pub plan: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitWindow {
    pub label: String,
    pub used_percent: f64,
    /// Unix milliseconds.
    pub resets_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    pub windows: Vec<LimitWindow>,
    pub error: Option<String>,
    /// Rows that aren't windows: a balance, or why it isn't shown.
    #[serde(default)]
    pub extra: Vec<LimitExtra>,
    /// The agent's CLI isn't installed there: nothing to show for it.
    #[serde(default)]
    pub missing: bool,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LimitExtra {
    pub label: String,
    /// `None`: unavailable (see `note`).
    pub value: Option<String>,
    pub note: Option<String>,
}

fn program(kind: AgentKind, binary: &str) -> String {
    if binary.trim().is_empty() {
        kind.default_binary().into()
    } else {
        binary.trim().into()
    }
}

/// Sign-in state from the CLI itself.
pub fn account(kind: AgentKind, binary: &str, location: &Location) -> Account {
    let prog = program(kind, binary);
    let args: Vec<String> = match kind {
        AgentKind::Claude => vec!["auth".into(), "status".into(), "--json".into()],
        AgentKind::Codex => vec!["login".into(), "status".into()],
        AgentKind::Opencode | AgentKind::Opencode2 => vec!["auth".into(), "list".into()],
        // `agy` has no status command: its token store says who.
        AgentKind::Antigravity => return crate::antigravity::account(location, &[]),
    };
    let cwd = if matches!(location, Location::Local) { oah_core::paths::home_dir().to_string_lossy().into_owned() } else { "~".into() };
    let out = match oah_transport::run(&Spec { location, cwd: &cwd, program: &prog, args, env: &[] }, None, Duration::from_secs(20)) {
        Ok(o) => o,
        Err(e) => return Account { error: Some(e), ..Default::default() },
    };
    match kind {
        AgentKind::Claude => parse_claude_status(&out.stdout).unwrap_or_else(|| Account {
            error: Some(first_line(&out.stderr).unwrap_or("couldn't read `claude auth status`").into()),
            ..Default::default()
        }),
        AgentKind::Codex => {
            let mut a = parse_codex_status(&format!("{}\n{}", out.stdout, out.stderr));
            // `login status` only says "ChatGPT"; the app-server knows which account.
            if a.signed_in && a.label.as_deref() == Some("ChatGPT") {
                if let Ok(v) = codex_once(&prog, location, "account/read", json!({})) {
                    apply_codex_account(&mut a, &v);
                }
            }
            a
        }
        AgentKind::Opencode | AgentKind::Opencode2 => parse_opencode_auth(&format!("{}\n{}", out.stdout, out.stderr)),
        AgentKind::Antigravity => unreachable!("answered above"),
    }
}

fn first_line(s: &str) -> Option<&str> {
    s.lines().map(str::trim).find(|l| !l.is_empty())
}

pub fn parse_claude_status(stdout: &str) -> Option<Account> {
    let v: Value = serde_json::from_str(stdout.trim()).ok()?;
    let signed_in = v["loggedIn"].as_bool().unwrap_or(false);
    let label = v["email"].as_str().map(String::from).or_else(|| match v["authMethod"].as_str() {
        Some("api_key") | Some("apiKey") => Some("API key".into()),
        _ => None,
    });
    Some(Account { signed_in, label, plan: v["subscriptionType"].as_str().map(String::from), error: None })
}

pub fn parse_codex_status(text: &str) -> Account {
    let line = first_line(text).unwrap_or("");
    let lower = line.to_lowercase();
    if lower.starts_with("logged in") {
        let how = line.split(" using ").nth(1).map(|s| s.trim().to_string());
        Account { signed_in: true, label: how, plan: None, error: None }
    } else {
        Account {
            signed_in: false,
            label: None,
            plan: None,
            error: (!line.is_empty() && !lower.contains("not logged in")).then(|| line.to_string()),
        }
    }
}

/// `account/read`'s ChatGPT email and plan, over `login status`'s "ChatGPT".
pub fn apply_codex_account(a: &mut Account, v: &Value) {
    let acct = &v["account"];
    if let Some(email) = acct["email"].as_str().filter(|e| e.contains('@')) {
        a.label = Some(email.to_string());
    }
    if let Some(plan) = acct["planType"].as_str() {
        a.plan = Some(plan.to_string());
    }
}

/// `opencode auth list`: one line per signed-in provider (and API keys from the environment).
pub fn parse_opencode_auth(text: &str) -> Account {
    // The list is coloured and draws boxes and bullets around provider names; keep the names.
    let text = strip_ansi(text);
    let providers: Vec<String> = text
        .lines()
        .map(|l| l.trim_matches(|c: char| !c.is_alphanumeric() && c != ' ' && c != '-' && c != '.').trim().to_string())
        .filter(|l| {
            !l.is_empty()
                && !l.to_lowercase().contains("credential")
                && !l.to_lowercase().contains("environment")
                && !l.contains('/')
                && !l.contains('\\')
        })
        .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
        .filter(|w| !w.is_empty() && w.chars().next().is_some_and(|c| c.is_alphabetic()))
        .collect();
    if providers.is_empty() {
        Account { signed_in: false, label: None, plan: None, error: Some("No providers signed in".into()) }
    } else {
        Account { signed_in: true, label: Some(providers.join(", ")), plan: None, error: None }
    }
}

/// Text without terminal colour codes (`ESC [ … letter`).
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for d in chars.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

// ---- Claude ----------------------------------------------------------------------------------

/// Claude's plan limits for the sign-in on `location` (this computer's credentials file, or the
/// one in a WSL distro or on an SSH host).
pub fn claude_limits(location: &Location, env: &[EnvVar]) -> Limits {
    let token = claude_usage_token(location, env);
    let Some(token) = token else {
        return Limits {
            error: Some("Claude plan usage is unavailable: no readable Claude plan credential was found for this location.".into()),
            ..Default::default()
        };
    };
    let agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).build().new_agent();
    // Claude may rotate the on-disk token while a usage request is in flight. Retry only if
    // it actually changed; never refresh or overwrite the CLI's credentials ourselves.
    let mut token = token;
    for attempt in 0..2 {
        let resp = agent
            .get("https://api.anthropic.com/api/oauth/usage")
            .header("Authorization", &format!("Bearer {token}"))
            .header("anthropic-beta", "oauth-2025-04-20")
            .header("User-Agent", "claude-code/2.1.0")
            .call();
        return match resp {
            Ok(mut r) => match r.body_mut().read_json::<Value>() {
                Ok(v) => Limits { windows: parse_claude_usage(&v), error: None, extra: vec![], missing: false },
                Err(_) => Limits { error: Some("Claude's usage endpoint returned an unexpected response.".into()), ..Default::default() },
            },
            Err(ureq::Error::StatusCode(status)) => {
                if status == 401 && attempt == 0 {
                    if let Some(next) = claude_usage_token(location, env).filter(|next| next != &token) {
                        token = next;
                        continue;
                    }
                }
                Limits { error: Some(claude_usage_error(status)), ..Default::default() }
            }
            Err(e) => Limits { error: Some(format!("couldn't load Claude limits: {e}")), ..Default::default() },
        };
    }
    unreachable!("a second usage attempt always returns")
}

fn claude_usage_error(status: u16) -> String {
    match status {
        401 | 403 => {
            format!("Claude's usage endpoint rejected access (HTTP {status}). Chat sign-in is checked separately.")
        }
        _ => format!("Claude's usage request failed (HTTP {status})."),
    }
}

/// Match the provider environment without changing the desktop process's environment.
fn local_env(env: &[EnvVar], key: &str) -> Option<String> {
    env.iter().rev().find(|v| v.key == key).map(|v| v.value.clone()).or_else(|| std::env::var(key).ok())
}

fn credential_home(env: &[EnvVar]) -> std::path::PathBuf {
    local_env(env, "HOME").filter(|s| !s.is_empty()).map(Into::into).unwrap_or_else(oah_core::paths::home_dir)
}

fn claude_usage_token(location: &Location, env: &[EnvVar]) -> Option<String> {
    let text = match location {
        Location::Local => {
            let dir = local_env(env, "CLAUDE_CONFIG_DIR")
                .filter(|s| !s.is_empty())
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| credential_home(env).join(".claude"));
            std::fs::read_to_string(dir.join(".credentials.json")).ok()
        }
        _ => {
            let args = vec!["-c".into(), r#"cat "${CLAUDE_CONFIG_DIR:-$HOME/.claude}/.credentials.json" 2>/dev/null"#.into()];
            oah_transport::run(&Spec { location, cwd: "~", program: "sh", args, env }, None, Duration::from_secs(20))
                .ok()
                .filter(|o| o.ok())
                .map(|o| o.stdout)
        }
    };
    text.and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["claudeAiOauth"]["accessToken"].as_str().map(String::from))
        .filter(|s| !s.is_empty())
}

pub fn parse_claude_usage(v: &Value) -> Vec<LimitWindow> {
    let mut out = Vec::new();
    let mut push = |label: &str, w: &Value| {
        let pct = w["utilization"].as_f64().or(w["used_percentage"].as_f64());
        if let Some(p) = pct {
            out.push(LimitWindow { label: label.into(), used_percent: p.clamp(0.0, 100.0), resets_at: reset_ms(&w["resets_at"]) });
        }
    };
    push("Current session (5 hours)", &v["five_hour"]);
    push("This week, all models", &v["seven_day"]);
    for (k, label) in
        [("seven_day_opus", "This week, Opus"), ("seven_day_sonnet", "This week, Sonnet"), ("seven_day_fable", "This week, Fable")]
    {
        push(label, &v[k]);
    }
    for l in v["limits"].as_array().into_iter().flatten() {
        if l["kind"] == "weekly_scoped" {
            if let Some(name) = l["scope"]["model"]["display_name"].as_str() {
                let label = format!("This week, {name}");
                if !out.iter().any(|w| w.label == label) {
                    if let Some(p) = l["percent"].as_f64() {
                        out.push(LimitWindow { label, used_percent: p.clamp(0.0, 100.0), resets_at: reset_ms(&l["resets_at"]) });
                    }
                }
            }
        }
    }
    out
}

/// Seconds, milliseconds or an ISO date → Unix milliseconds.
pub(crate) fn reset_ms(v: &Value) -> Option<u64> {
    let n = v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()));
    if let Some(n) = n {
        return Some(if n > 1e10 { n as u64 } else { (n * 1000.0) as u64 });
    }
    v.as_str().and_then(iso_ms)
}

/// `2026-11-07T05:00:00.123+00:00` / `…Z` → Unix ms (UTC offsets only, which is what the API sends).
fn iso_ms(s: &str) -> Option<u64> {
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let time = time.trim_end_matches('Z');
    let (time, offset) = match time.rfind(['+', '-']) {
        Some(i) if i > 0 => (&time[..i], &time[i..]),
        _ => (time, "+00:00"),
    };
    let mut t = time.split(':');
    let (hh, mm) = (t.next()?.parse::<i64>().ok()?, t.next()?.parse::<i64>().ok()?);
    let ss = t.next().and_then(|x| x.parse::<f64>().ok()).unwrap_or(0.0);
    let sign = if offset.starts_with('-') { -1 } else { 1 };
    let mut o = offset[1..].split(':');
    let off =
        sign * (o.next().and_then(|x| x.parse::<i64>().ok()).unwrap_or(0) * 60 + o.next().and_then(|x| x.parse::<i64>().ok()).unwrap_or(0));
    // Days from the civil date (Howard Hinnant's algorithm).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let secs = days * 86400 + hh * 3600 + mm * 60 - off * 60;
    Some((secs as f64 * 1000.0 + ss * 1000.0) as u64)
}

// ---- OpenCode Go -----------------------------------------------------------------------------

/// OpenCode Go's plan limits: `GET https://opencode.ai/zen/go/v1/usage` with the `opencode-go`
/// key from OpenCode's `auth.json` (on this computer, or on `location`).
pub fn opencode_go_limits(location: &Location, env: &[EnvVar]) -> Limits {
    let fail = |e: String| Limits { windows: vec![], error: Some(e), extra: vec![], missing: false };
    let Some(key) = opencode_go_key(location, env) else {
        return fail("No OpenCode Go key found for this location. OpenCode can still use other providers.".into());
    };
    let agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(15))).http_status_as_error(false).build().new_agent();
    let resp = agent
        .get("https://opencode.ai/zen/go/v1/usage")
        .header("Authorization", &format!("Bearer {key}"))
        .header("Accept", "application/json")
        .call();
    let mut r = match resp {
        Ok(r) => r,
        Err(e) => return fail(format!("couldn't load OpenCode Go limits: {e}")),
    };
    let status = r.status().as_u16();
    let body = r.body_mut().read_to_string().unwrap_or_default();
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    match opencode_go_answer(status, &body, &v) {
        Ok(windows) => Limits {
            windows,
            error: None,
            // With a key the answer has no Zen balance (that needs the console's sign-in).
            extra: vec![LimitExtra {
                label: "Zen balance".into(),
                value: None,
                note: Some("The balance isn't included in this usage answer.".into()),
            }],
            missing: false,
        },
        Err(e) => fail(e),
    }
}

/// `{"usage": {"rolling": {percent, resetsAt}, "weekly": …, "monthly": …}}` → windows; the
/// account's problems in plain words.
pub fn opencode_go_answer(status: u16, body: &str, v: &Value) -> Result<Vec<LimitWindow>, String> {
    let kind = v["error"]["type"].as_str().unwrap_or("");
    if kind == "EntitlementError" {
        return Err("OpenCode Go's usage endpoint reports no subscription for this key.".into());
    }
    if body.trim_start().starts_with('<') {
        return Err(format!("OpenCode Go usage is unavailable: the endpoint returned a web page (HTTP {status})."));
    }
    if status == 401 || status == 403 || kind == "AuthError" {
        return Err("OpenCode Go's usage endpoint rejected access. Chat sign-in is checked separately.".into());
    }
    if !(200..300).contains(&status) {
        return Err(format!("OpenCode Go's usage request failed ({status})."));
    }
    let u = &v["usage"];
    let mut out = Vec::new();
    for (k, label) in [("rolling", "Current session (5 hours)"), ("weekly", "This week"), ("monthly", "This month")] {
        if let Some(p) = u[k]["percent"].as_f64() {
            out.push(LimitWindow { label: label.into(), used_percent: p.clamp(0.0, 100.0), resets_at: reset_ms(&u[k]["resetsAt"]) });
        }
    }
    if out.len() < 2 {
        return Err("OpenCode Go answered in a way oah doesn't understand.".into());
    }
    Ok(out)
}

/// Where 2.x keeps credentials (`opencode.db`, table `credential`): the `opencode-go` one's
/// value (`{"type": "api", "key": …}`), read with `sqlite3`, else Python, wherever OpenCode is.
const GO_KEY_FROM_DB: &str = r#"db="${OPENCODE_DB:-${XDG_DATA_HOME:-$HOME/.local/share}/opencode/opencode.db}"
[ -f "$db" ] || exit 0
q="SELECT value FROM credential WHERE integration_id='opencode-go' ORDER BY active DESC, time_created DESC LIMIT 1"
if command -v sqlite3 >/dev/null 2>&1; then sqlite3 -readonly "$db" "$q"
elif command -v python3 >/dev/null 2>&1; then python3 -I -c 'import sqlite3, sys
c = sqlite3.connect("file:" + sys.argv[1] + "?mode=ro", uri=True)
r = c.execute(sys.argv[2]).fetchone()
print(r[0] if r else "")' "$db" "$q"
fi"#;

/// The `opencode-go` API key: `$OPENCODE_AUTH_CONTENT`; else 2.x's `opencode.db`; else 1.x's
/// `auth.json` (both in OpenCode's data folder, `$XDG_DATA_HOME/opencode` or
/// `~/.local/share/opencode`); else `$OPENCODE_API_KEY`.
fn opencode_go_key(location: &Location, env: &[EnvVar]) -> Option<String> {
    let from_auth = |text: &str| -> Option<String> {
        let v: Value = serde_json::from_str(text).ok()?;
        let e = &v["opencode-go"];
        (e["type"] == "api").then(|| e["key"].as_str().map(String::from)).flatten().filter(|k| !k.is_empty())
    };
    let from_db = |text: &str| -> Option<String> {
        let v: Value = serde_json::from_str(text.trim()).ok()?;
        v["key"].as_str().filter(|k| !k.is_empty()).map(String::from)
    };
    let sh = |script: &str| -> Option<String> {
        let args = vec!["-c".into(), script.into()];
        oah_transport::run(&Spec { location, cwd: "~", program: "sh", args, env }, None, Duration::from_secs(20))
            .ok()
            .filter(|o| o.ok() && !o.stdout.trim().is_empty())
            .map(|o| o.stdout)
    };
    match location {
        Location::Local => {
            if let Some(k) = local_env(env, "OPENCODE_AUTH_CONTENT").and_then(|t| from_auth(&t)) {
                return Some(k);
            }
            if let Some(k) = cfg!(unix).then(|| sh(GO_KEY_FROM_DB)).flatten().and_then(|t| from_db(&t)) {
                return Some(k);
            }
            let data = local_env(env, "XDG_DATA_HOME")
                .filter(|s| !s.is_empty())
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| credential_home(env).join(".local").join("share"));
            if let Some(k) = std::fs::read_to_string(data.join("opencode").join("auth.json")).ok().and_then(|t| from_auth(&t)) {
                return Some(k);
            }
            local_env(env, "OPENCODE_API_KEY").filter(|k| !k.is_empty())
        }
        _ => {
            if let Some(k) =
                sh(r#"[ "${OPENCODE_AUTH_CONTENT+x}" = x ] && printf '%s' "$OPENCODE_AUTH_CONTENT""#).and_then(|t| from_auth(&t))
            {
                return Some(k);
            }
            if let Some(k) = sh(GO_KEY_FROM_DB).and_then(|t| from_db(&t)) {
                return Some(k);
            }
            if let Some(k) = sh(r#"cat "${XDG_DATA_HOME:-$HOME/.local/share}/opencode/auth.json" 2>/dev/null"#).and_then(|t| from_auth(&t))
            {
                return Some(k);
            }
            sh(r#"printf '%s' "$OPENCODE_API_KEY""#).map(|k| k.trim().to_string()).filter(|k| !k.is_empty())
        }
    }
}

// ---- Codex -----------------------------------------------------------------------------------

pub fn codex_limits(binary: &str, location: &Location) -> Limits {
    match codex_once(&program(AgentKind::Codex, binary), location, "account/rateLimits/read", Value::Null) {
        Ok(v) => Limits { windows: parse_codex_limits(&v), error: None, extra: vec![], missing: false },
        Err(e) => Limits { windows: vec![], error: Some(e), extra: vec![], missing: false },
    }
}

/// One request to a short-lived `codex app-server` (where `location` says).
pub fn codex_once(prog: &str, location: &Location, method: &str, params: Value) -> Result<Value, String> {
    let home = if matches!(location, Location::Local) { oah_core::paths::home_dir().to_string_lossy().into_owned() } else { "~".into() };
    let mut child = oah_transport::command(&Spec { location, cwd: &home, program: prog, args: vec!["app-server".into()], env: &[] })?
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                if v.get("method").is_none() && v["id"].is_u64() && tx.send(v).is_err() {
                    break;
                }
            }
        }
    });
    let mut send = |v: Value| writeln!(stdin, "{v}").map_err(|e| e.to_string());
    send(
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"clientInfo": {"name": "oah", "title": "oah", "version": env!("CARGO_PKG_VERSION")}, "capabilities": null}}),
    )?;
    let result = (|| {
        rx.recv_timeout(Duration::from_secs(20)).map_err(|_| "Codex didn't start".to_string())?;
        send(json!({"jsonrpc": "2.0", "method": "initialized"}))?;
        send(json!({"jsonrpc": "2.0", "id": 2, "method": method, "params": params}))?;
        loop {
            let v = rx.recv_timeout(Duration::from_secs(20)).map_err(|_| format!("Codex didn't answer {method}"))?;
            if v["id"] == 2 {
                return match v.get("error") {
                    Some(e) => Err(e["message"].as_str().unwrap_or("Codex couldn't answer").to_string()),
                    None => Ok(v["result"].clone()),
                };
            }
        }
    })();
    let _ = child.kill();
    let _ = child.wait();
    result
}

pub fn parse_codex_limits(v: &Value) -> Vec<LimitWindow> {
    let rl = &v["rateLimits"];
    let mut out = Vec::new();
    for key in ["primary", "secondary"] {
        let w = &rl[key];
        if let Some(p) = w["usedPercent"].as_f64() {
            let mins = w["windowDurationMins"].as_u64().unwrap_or(0);
            let label = match mins {
                300 => "5 hour window".to_string(),
                10080 => "Weekly window".to_string(),
                m if m >= 1440 => format!("{} day window", m / 1440),
                m if m > 0 => format!("{} hour window", (m as f64 / 60.0).round()),
                _ => "Window".to_string(),
            };
            out.push(LimitWindow { label, used_percent: p.clamp(0.0, 100.0), resets_at: reset_ms(&w["resetsAt"]) });
        }
    }
    out
}

/// Opens a terminal running the CLI's sign-in: on this computer, or inside a WSL distro (the
/// browser step works there too: WSL2 forwards the sign-in's localhost callback).
pub fn open_login(kind: AgentKind, binary: &str, location: &Location) -> Result<(), String> {
    let prog = program(kind, binary);
    let line = match kind {
        AgentKind::Claude => format!("{prog} auth login"),
        AgentKind::Codex => format!("{prog} login"),
        AgentKind::Opencode | AgentKind::Opencode2 => format!("{prog} auth login"),
        AgentKind::Antigravity => crate::antigravity::login_line(&prog),
    };
    match location {
        Location::Local => {}
        Location::Wsl { distro } => return open_wsl_login(distro, &prog, &line),
        Location::Ssh { host } => return Err(format!("sign in on {host} with `{line}` in a terminal there")),
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("cmd")
            .raw_arg(format!("/c start \"Sign in\" cmd /k {line}"))
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        for term in ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"] {
            let mut c = std::process::Command::new(term);
            if term == "gnome-terminal" {
                c.args(["--", "sh", "-c", &format!("{line}; exec $SHELL")]);
            } else {
                c.args(["-e", "sh", "-c", &format!("{line}; exec $SHELL")]);
            }
            if c.spawn().is_ok() {
                return Ok(());
            }
        }
        Err(format!("couldn't open a terminal; run `{line}` yourself"))
    }
}

/// A console window running `<line>` in `distro`, left open on a login shell afterwards.
fn open_wsl_login(distro: &str, prog: &str, line: &str) -> Result<(), String> {
    if !oah_core::wsl::valid_distro(distro) || !prog.chars().all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c)) {
        return Err("can't build that sign-in command".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let wsl = oah_transport::wsl::exe();
        std::process::Command::new("cmd")
            .raw_arg(format!(
                "/c start \"Sign in ({distro})\" \"{}\" -d {distro} -e sh -lc \"{line}; exec ${{SHELL:-sh}} -l\"",
                wsl.display()
            ))
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = line;
        Err("WSL is a Windows feature".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sign_in() {
        let a = parse_claude_status(r#"{"loggedIn":true,"authMethod":"claude.ai","email":"me@x.dev","subscriptionType":"max"}"#).unwrap();
        assert!(a.signed_in);
        assert_eq!(a.label.as_deref(), Some("me@x.dev"));
        assert_eq!(a.plan.as_deref(), Some("max"));
        let c = parse_codex_status("Logged in using ChatGPT\n");
        assert!(c.signed_in && c.label.as_deref() == Some("ChatGPT"));
        let mut c = c;
        apply_codex_account(&mut c, &json!({"account":{"type":"chatgpt","email":"me@x.dev","planType":"pro"}}));
        assert_eq!((c.label.as_deref(), c.plan.as_deref()), (Some("me@x.dev"), Some("pro")));
        let mut k = parse_codex_status("Logged in using an API key\n");
        apply_codex_account(&mut k, &json!({"account":{"type":"apiKey"}}));
        assert_eq!(k.label.as_deref(), Some("an API key"));
        assert!(!parse_codex_status("Not logged in").signed_in);
    }

    #[test]
    fn reads_opencode_sign_in() {
        // Real `opencode auth list` output (1.19.5), colours and all.
        let out = "\u{1b}[0m\n\u{1b}[90m┌\u{1b}[39m  Credentials \u{1b}[90m~\\.local\\share\\opencode\\auth.json\n\u{1b}[90m│\u{1b}[39m\n\u{1b}[34m●\u{1b}[39m  OpenAI \u{1b}[90moauth\n\u{1b}[90m│\u{1b}[39m\n\u{1b}[90m└\u{1b}[39m  1 credentials\n";
        let a = parse_opencode_auth(out);
        assert!(a.signed_in);
        assert_eq!(a.label.as_deref(), Some("OpenAI"));
        assert!(!parse_opencode_auth("┌  Credentials\n└  0 credentials\n").signed_in);
    }

    #[test]
    fn reads_opencode_go_limits() {
        let ok = json!({"usage":{"rolling":{"status":"ok","percent":30,"resetsAt":"2026-09-23T17:00:00.000Z"},"weekly":{"status":"ok","percent":51,"resetsAt":"2026-09-28T00:00:00.000Z"},"monthly":{"status":"rate-limited","percent":100,"resetsAt":"2026-10-01T00:00:00.000Z"}}});
        let w = opencode_go_answer(200, "{}", &ok).unwrap();
        assert_eq!(w.iter().map(|w| w.used_percent).collect::<Vec<_>>(), [30.0, 51.0, 100.0]);
        assert_eq!(w[0].label, "Current session (5 hours)");
        assert!(w[0].resets_at.is_some());
        assert!(opencode_go_answer(403, "", &Value::Null).unwrap_err().contains("rejected access"));
        assert!(opencode_go_answer(200, "", &json!({"error":{"type":"EntitlementError"}})).unwrap_err().contains("subscription"));
        assert!(opencode_go_answer(401, "", &Value::Null).unwrap_err().contains("rejected access"));
        assert!(opencode_go_answer(200, "<html>sign in</html>", &Value::Null).unwrap_err().contains("web page"));
        assert!(opencode_go_answer(200, "{}", &json!({"usage":{"rolling":{"percent":1}}})).is_err());
    }

    #[test]
    fn usage_errors_do_not_infer_chat_sign_in_or_subscription() {
        for status in [401, 403] {
            let claude = claude_usage_error(status);
            assert!(claude.contains("usage endpoint"));
            assert!(!claude.contains("expired"));
            let go = opencode_go_answer(status, "", &Value::Null).unwrap_err();
            assert!(go.contains("usage endpoint"));
            assert!(!go.contains("no subscription"));
            assert!(!go.contains("Sign in again"));
        }
        for status in [200, 403, 502] {
            let html = opencode_go_answer(status, " \n<!DOCTYPE html><title>Gateway</title>", &Value::Null).unwrap_err();
            assert!(html.contains("web page"));
            assert!(!html.contains("rejected access"));
        }
        assert!(opencode_go_answer(429, "", &Value::Null).unwrap_err().contains("429"));
    }

    #[test]
    fn usage_reads_the_configured_provider_credentials() {
        let dir = std::env::temp_dir().join(format!("oah-usage-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".credentials.json"), r#"{"claudeAiOauth":{"accessToken":"test-claude"}}"#).unwrap();
        let var = |key: &str, value: &str| EnvVar { key: key.into(), value: value.into(), in_keychain: false };
        let path = dir.to_str().unwrap();
        assert_eq!(claude_usage_token(&Location::Local, &[var("CLAUDE_CONFIG_DIR", path)]).as_deref(), Some("test-claude"));
        // An explicit inline credential takes precedence without changing the process env.
        let inline = r#"{"opencode-go":{"type":"api","key":"test-inline"}}"#;
        assert_eq!(
            opencode_go_key(&Location::Local, &[var("XDG_DATA_HOME", path), var("OPENCODE_AUTH_CONTENT", inline)]).as_deref(),
            Some("test-inline")
        );
        assert!(opencode_go_key(&Location::Local, &[var("OPENCODE_AUTH_CONTENT", "invalid")]).is_none());
        assert_eq!(credential_home(&[var("HOME", path)]), dir);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reads_limits() {
        let w = parse_claude_usage(
            &json!({"five_hour":{"utilization":12.0,"resets_at":"2026-11-07T05:00:00+00:00"},"seven_day":{"utilization":47.0,"resets_at":1794301200}}),
        );
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].resets_at, Some(1_794_027_600_000));
        assert_eq!(w[1].resets_at, Some(1_794_301_200_000));
        let c = parse_codex_limits(
            &json!({"rateLimits":{"primary":{"usedPercent":45,"windowDurationMins":10080,"resetsAt":1794410242},"secondary":null}}),
        );
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].label, "Weekly window");
    }
}
