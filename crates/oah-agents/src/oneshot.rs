//! One question to an agent CLI, answered as plain text, with its cheapest model and no tools:
//! for small jobs like writing a commit message. The question goes on stdin.
//!
//! - Claude Code: `claude -p --model haiku --tools "" --no-session-persistence`
//! - Codex: `codex exec -m <its fast, affordable model> -s read-only --ephemeral` at low effort
//! - OpenCode: `opencode run -m <a free model>`
//! - Antigravity: `agy --print= --sandbox --model <Flash> --effort low`, stream-json in and out
//! - Grok Build: a session over ACP at its lowest effort, deleted after (`grok -p` takes the
//!   question only as an argument, too short for a diff on Windows)

use std::sync::Mutex;
use std::time::Duration;

use oah_core::settings::{AgentKind, EnvVar, Location};
use oah_transport::Spec;

use crate::driver::DriverResult;
use crate::event::ModelOption;

/// The model a one-shot uses, and what to call it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cheap {
    pub model: Option<String>,
    /// The effort to ask at (Antigravity picks it apart from the model).
    pub effort: Option<String>,
    pub label: String,
}

/// Remembered per agent and machine: picking a model costs a process start.
static PICKED: Mutex<Vec<(AgentKind, Location, Cheap)>> = Mutex::new(Vec::new());

/// The cheapest model `kind` offers on `location`.
pub fn cheapest(kind: AgentKind, binary: &str, location: &Location, cwd: &str, env: &[EnvVar]) -> Cheap {
    if let Some((_, _, c)) = PICKED.lock().unwrap().iter().find(|(k, l, _)| *k == kind && l == location) {
        return c.clone();
    }
    let fallback = || Cheap { model: None, label: "default model".into(), ..Default::default() };
    // `listed`: the models could be asked for. A lookup that failed (the CLI slow to start,
    // offline) isn't remembered, or every later answer would use the default model.
    let (c, listed) = match kind {
        AgentKind::Claude => (Cheap { model: Some("haiku".into()), label: "Haiku".into(), ..Default::default() }, true),
        AgentKind::Codex => {
            let prog = program(kind, binary);
            match crate::account::codex_once(&prog, location, env, "model/list", serde_json::json!({"limit": 100, "includeHidden": false}))
            {
                Ok(v) => (pick_codex(&crate::codex::parse_models(&v)), true),
                Err(_) => (pick_codex(&[]), false),
            }
        }
        AgentKind::Opencode | AgentKind::Opencode2 => match crate::acp::opencode_models(binary, location, cwd, env) {
            Ok(m) => (pick_opencode(&m), true),
            Err(_) => (pick_opencode(&[]), false),
        },
        AgentKind::Antigravity => match crate::antigravity::models(binary, location, cwd, env) {
            Ok(m) => (
                crate::antigravity::pick_cheap(&m)
                    .map(|(model, effort, label)| Cheap { model: Some(model), effort, label })
                    .unwrap_or_else(fallback),
                true,
            ),
            Err(_) => (fallback(), false),
        },
        AgentKind::Grok => match crate::grok::models(binary, location, env) {
            Ok(m) => (
                crate::grok::pick_cheap(&m)
                    .map(|(model, effort, label)| Cheap { model: Some(model), effort, label })
                    .unwrap_or_else(fallback),
                true,
            ),
            Err(_) => (fallback(), false),
        },
    };
    if listed {
        PICKED.lock().unwrap().push((kind, location.clone(), c.clone()));
    }
    c
}

/// Codex's "fast and affordable" model: the first (newest) whose name or description says so.
pub fn pick_codex(models: &[ModelOption]) -> Cheap {
    let cheap = |m: &&ModelOption| {
        let t = format!("{} {} {}", m.id, m.name, m.description).to_lowercase();
        ["affordable", "mini", "luna", "fast", "efficient", "nano"].iter().any(|w| t.contains(w))
    };
    match models.iter().find(cheap) {
        Some(m) => Cheap { model: Some(m.id.clone()), label: m.name.clone(), ..Default::default() },
        None => Cheap { model: None, label: "default model".into(), ..Default::default() },
    }
}

/// A free OpenCode model (`opencode/…-free`), else its default.
pub fn pick_opencode(models: &[ModelOption]) -> Cheap {
    let free = models.iter().find(|m| m.id.starts_with("opencode/") && m.id.contains("free"));
    match free {
        Some(m) => Cheap { model: Some(m.id.clone()), label: m.name.clone(), ..Default::default() },
        None => Cheap { model: None, label: "default model".into(), ..Default::default() },
    }
}

fn program(kind: AgentKind, binary: &str) -> String {
    if binary.trim().is_empty() {
        kind.default_binary().into()
    } else {
        binary.to_string()
    }
}

/// Asks `question` (on stdin) and returns the answer.
pub fn ask(
    kind: AgentKind,
    binary: &str,
    location: &Location,
    cwd: &str,
    env: &[EnvVar],
    cheap: &Cheap,
    question: &str,
) -> DriverResult<String> {
    if kind == AgentKind::Grok {
        let opts = crate::driver::StartOptions {
            kind,
            cwd: cwd.to_string(),
            location: location.clone(),
            binary: binary.to_string(),
            model: cheap.model.clone(),
            effort: cheap.effort.clone(),
            permission: oah_core::settings::PermissionMode::Ask,
            resume: None,
            env: env.to_vec(),
        };
        return crate::acp::ask_once(opts, question, Duration::from_secs(180));
    }
    let prog = program(kind, binary);
    let mut args: Vec<String> = match kind {
        AgentKind::Claude => ["-p", "--tools", "", "--no-session-persistence", "--output-format", "text"].map(String::from).to_vec(),
        AgentKind::Codex => {
            ["exec", "-s", "read-only", "--skip-git-repo-check", "--ephemeral", "--color", "never", "-c", "model_reasoning_effort=low"]
                .map(String::from)
                .to_vec()
        }
        AgentKind::Opencode | AgentKind::Opencode2 => vec!["run".into()],
        AgentKind::Antigravity => crate::antigravity::oneshot_args(cheap.model.as_deref(), cheap.effort.as_deref()),
        AgentKind::Grok => unreachable!("asked over ACP above"),
    };
    if let Some(m) = cheap.model.as_ref().filter(|_| kind != AgentKind::Antigravity) {
        args.extend(["--model".into(), m.clone()]);
    }
    if kind == AgentKind::Codex {
        // The question comes on stdin.
        args.push("-".into());
    }
    // OpenCode answers read-only, like the others: no edits, commands or fetches, never shared.
    // The provider's own environment (its account, config folder, key) first.
    let mut env: Vec<EnvVar> = env.to_vec();
    if kind.is_opencode() {
        env.retain(|v| v.key != "OPENCODE_PERMISSION" && v.key != "OPENCODE_DISABLE_SHARE");
        env.extend(
            [("OPENCODE_PERMISSION", r#"{"edit":"deny","bash":"deny","webfetch":"deny"}"#), ("OPENCODE_DISABLE_SHARE", "1")]
                .map(|(k, v)| EnvVar { key: k.into(), value: v.into(), in_keychain: false }),
        );
    }
    // Antigravity reads the question as stream-json and answers in it.
    let input = if kind == AgentKind::Antigravity {
        crate::antigravity::acp_env(&mut env);
        crate::antigravity::oneshot_input(question)
    } else {
        question.to_string()
    };
    let out =
        oah_transport::run(&Spec { location, cwd, program: &prog, args, env: &env }, Some(input.as_bytes()), Duration::from_secs(180))?;
    if !out.ok() {
        let why = out.stderr.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("no answer").to_string();
        return Err(if out.code == Some(127) { format!("{prog} isn't installed there") } else { why });
    }
    if kind == AgentKind::Antigravity {
        return crate::antigravity::oneshot_answer(&out.stdout);
    }
    Ok(out.stdout)
}

/// A reply cut to the message itself: no code fences, quotes or "Commit message:" lead-in, and
/// no trailing blank lines.
pub fn clean_message(reply: &str) -> String {
    let mut t = reply.trim().to_string();
    if t.starts_with("```") {
        t = t.lines().skip(1).take_while(|l| !l.starts_with("```")).collect::<Vec<_>>().join("\n");
    }
    for lead in ["Commit message:", "commit message:", "Message:"] {
        if let Some(rest) = t.strip_prefix(lead) {
            t = rest.trim_start().to_string();
        }
    }
    let t = t.trim();
    let t = t.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(t);
    t.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: &str, name: &str, description: &str) -> ModelOption {
        ModelOption { id: id.into(), name: name.into(), description: description.into(), ..Default::default() }
    }

    #[test]
    fn picks_cheap_models() {
        let codex = [
            m("gpt-6.1-sol", "GPT-6.1-Sol", "Latest workhorse model"),
            m("gpt-6-luna", "GPT-6-Luna", "Fast and affordable model for easier tasks."),
            m("gpt-5.6-luna", "GPT-5.6-Luna", "Older fast and efficient model."),
        ];
        assert_eq!(pick_codex(&codex).model.as_deref(), Some("gpt-6-luna"));
        assert_eq!(pick_codex(&[m("big", "Big", "Frontier")]).model, None);
        let oc = [m("anthropic/claude", "Claude", "anthropic"), m("opencode/glm-5.3-flash-free", "GLM Flash Free", "opencode")];
        assert_eq!(pick_opencode(&oc).model.as_deref(), Some("opencode/glm-5.3-flash-free"));
    }

    #[test]
    fn cleans_replies() {
        assert_eq!(clean_message("Fix the loop\n\n"), "Fix the loop");
        assert_eq!(clean_message("```\nFix the loop\n\nMore detail.\n```"), "Fix the loop\n\nMore detail.");
        assert_eq!(clean_message("Commit message: \"Fix the loop\""), "Fix the loop");
    }
}
