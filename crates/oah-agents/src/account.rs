//! Who each CLI is signed in as, and its plan limits.
//!
//! - Sign-in: `claude auth status --json` and `codex login status` (run where the project is).
//! - Claude limits: `GET https://api.anthropic.com/api/oauth/usage` with the CLI's own OAuth
//!   token from `~/.claude/.credentials.json` (this computer only).
//! - Codex limits: `account/read` and `account/rateLimits/read` from a short-lived
//!   `codex app-server`.

use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

use oah_core::settings::{AgentKind, Location};
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
        AgentKind::Opencode => vec!["auth".into(), "list".into()],
        AgentKind::Acp => return Account { error: Some("sign in with the agent's own command".into()), ..Default::default() },
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
        AgentKind::Codex => parse_codex_status(&format!("{}\n{}", out.stdout, out.stderr)),
        AgentKind::Opencode | AgentKind::Acp => parse_opencode_auth(&format!("{}\n{}", out.stdout, out.stderr)),
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
pub fn claude_limits(location: &Location) -> Limits {
    let text = match location {
        Location::Local => std::fs::read_to_string(oah_core::paths::home_dir().join(".claude").join(".credentials.json")).ok(),
        _ => {
            let args = vec!["-c".into(), r#"cat "$HOME/.claude/.credentials.json" 2>/dev/null"#.into()];
            oah_transport::run(&Spec { location, cwd: "~", program: "sh", args, env: &[] }, None, Duration::from_secs(20))
                .ok()
                .filter(|o| o.ok())
                .map(|o| o.stdout)
        }
    };
    let token = text
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["claudeAiOauth"]["accessToken"].as_str().map(String::from));
    let Some(token) = token else {
        return Limits {
            windows: vec![],
            error: Some("Sign in to Claude Code with a Claude plan to see its limits.".into()),
            extra: vec![],
        };
    };
    let agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).build().new_agent();
    let resp = agent
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", &format!("Bearer {token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("User-Agent", "claude-code/2.1.0")
        .call();
    match resp {
        Ok(mut r) => match r.body_mut().read_json::<Value>() {
            Ok(v) => Limits { windows: parse_claude_usage(&v), error: None, extra: vec![] },
            Err(e) => Limits { windows: vec![], error: Some(format!("unexpected answer: {e}")), extra: vec![] },
        },
        Err(ureq::Error::StatusCode(401)) => {
            Limits { windows: vec![], error: Some("Claude's sign-in expired. Run Claude Code once to refresh it.".into()), extra: vec![] }
        }
        Err(e) => Limits { windows: vec![], error: Some(format!("couldn't load Claude limits: {e}")), extra: vec![] },
    }
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
fn reset_ms(v: &Value) -> Option<u64> {
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
pub fn opencode_go_limits(location: &Location) -> Limits {
    let fail = |e: String| Limits { windows: vec![], error: Some(e), extra: vec![] };
    let Some(key) = opencode_go_key(location) else {
        return fail("Sign in to OpenCode Go (`opencode auth login`, pick OpenCode Go) to see its limits.".into());
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
        },
        Err(e) => fail(e),
    }
}

/// `{"usage": {"rolling": {percent, resetsAt}, "weekly": …, "monthly": …}}` → windows; the
/// account's problems in plain words.
pub fn opencode_go_answer(status: u16, body: &str, v: &Value) -> Result<Vec<LimitWindow>, String> {
    let kind = v["error"]["type"].as_str().unwrap_or("");
    if status == 403 || kind == "EntitlementError" {
        return Err("This OpenCode account has no OpenCode Go subscription.".into());
    }
    if status == 401 || kind == "AuthError" || body.trim_start().starts_with('<') {
        return Err("OpenCode Go turned the key down. Sign in again with `opencode auth login`.".into());
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

/// The `opencode-go` API key: `$OPENCODE_AUTH_CONTENT`, else `auth.json` in OpenCode's data
/// folder (`$XDG_DATA_HOME/opencode`, or `~/.local/share/opencode`).
fn opencode_go_key(location: &Location) -> Option<String> {
    let text = match location {
        Location::Local => std::env::var("OPENCODE_AUTH_CONTENT").ok().or_else(|| {
            let data = std::env::var_os("XDG_DATA_HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| oah_core::paths::home_dir().join(".local").join("share"));
            std::fs::read_to_string(data.join("opencode").join("auth.json")).ok()
        }),
        _ => {
            let args = vec!["-c".into(), r#"cat "${XDG_DATA_HOME:-$HOME/.local/share}/opencode/auth.json" 2>/dev/null"#.into()];
            oah_transport::run(&Spec { location, cwd: "~", program: "sh", args, env: &[] }, None, Duration::from_secs(20))
                .ok()
                .filter(|o| o.ok())
                .map(|o| o.stdout)
        }
    }?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let e = &v["opencode-go"];
    (e["type"] == "api").then(|| e["key"].as_str().map(String::from)).flatten().filter(|k| !k.is_empty())
}

// ---- Codex -----------------------------------------------------------------------------------

pub fn codex_limits(binary: &str, location: &Location) -> Limits {
    match codex_once(&program(AgentKind::Codex, binary), location, "account/rateLimits/read", Value::Null) {
        Ok(v) => Limits { windows: parse_codex_limits(&v), error: None, extra: vec![] },
        Err(e) => Limits { windows: vec![], error: Some(e), extra: vec![] },
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
        AgentKind::Opencode => format!("{prog} auth login"),
        AgentKind::Acp => return Err("sign in with the agent's own command".into()),
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
        assert!(opencode_go_answer(403, "", &Value::Null).unwrap_err().contains("no OpenCode Go subscription"));
        assert!(opencode_go_answer(200, "", &json!({"error":{"type":"EntitlementError"}})).unwrap_err().contains("subscription"));
        assert!(opencode_go_answer(401, "", &Value::Null).unwrap_err().contains("turned the key down"));
        assert!(opencode_go_answer(200, "<html>sign in</html>", &Value::Null).unwrap_err().contains("turned the key down"));
        assert!(opencode_go_answer(200, "{}", &json!({"usage":{"rolling":{"percent":1}}})).is_err());
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
