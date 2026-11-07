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

pub fn claude_limits() -> Limits {
    let path = oah_core::paths::home_dir().join(".claude").join(".credentials.json");
    let token = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["claudeAiOauth"]["accessToken"].as_str().map(String::from));
    let Some(token) = token else {
        return Limits { windows: vec![], error: Some("Sign in to Claude Code with a Claude plan to see its limits.".into()) };
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
            Ok(v) => Limits { windows: parse_claude_usage(&v), error: None },
            Err(e) => Limits { windows: vec![], error: Some(format!("unexpected answer: {e}")) },
        },
        Err(ureq::Error::StatusCode(401)) => {
            Limits { windows: vec![], error: Some("Claude's sign-in expired. Run Claude Code once to refresh it.".into()) }
        }
        Err(e) => Limits { windows: vec![], error: Some(format!("couldn't load Claude limits: {e}")) },
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

// ---- Codex -----------------------------------------------------------------------------------

pub fn codex_limits(binary: &str) -> Limits {
    match codex_once(&program(AgentKind::Codex, binary), &Location::Local, "account/rateLimits/read", Value::Null) {
        Ok(v) => Limits { windows: parse_codex_limits(&v), error: None },
        Err(e) => Limits { windows: vec![], error: Some(e) },
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

/// Opens a terminal running the CLI's sign-in.
pub fn open_login(kind: AgentKind, binary: &str) -> Result<(), String> {
    let prog = program(kind, binary);
    let line = match kind {
        AgentKind::Claude => format!("{prog} auth login"),
        AgentKind::Codex => format!("{prog} login"),
        AgentKind::Opencode => format!("{prog} auth login"),
        AgentKind::Acp => return Err("sign in with the agent's own command".into()),
    };
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
