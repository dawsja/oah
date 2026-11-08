//! A chat's log as short Markdown for another agent to read: what was asked, what the agent
//! said, one line per tool, what was asked of the user and how it was answered.

use std::fmt::Write;

use oah_agents::event::{ChatEvent, Decision};

pub struct Options {
    /// Only the last this many turns (a turn starts with a user message); 0 is all.
    pub last_turns: usize,
    /// Tool output in full (cut at a few thousand characters) instead of only for failures.
    pub tool_output: bool,
}

/// `events` are the chat's logged events (`events.jsonl` lines); lines this build can't read
/// are skipped.
pub fn render(events: &[serde_json::Value], opts: &Options) -> String {
    let events: Vec<ChatEvent> = events.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect();
    let starts: Vec<usize> =
        events.iter().enumerate().filter(|(_, e)| matches!(e, ChatEvent::UserMessage { .. })).map(|(i, _)| i).collect();
    let from = match opts.last_turns {
        0 => 0,
        n if starts.len() > n => starts[starts.len() - n],
        _ => 0,
    };
    let mut out = String::new();
    if from > 0 {
        let _ = writeln!(out, "_({} earlier turns not shown)_\n", starts.iter().filter(|&&s| s < from).count());
    }
    // Tool lines wait for their result; keyed by item id.
    let mut tools: Vec<(String, String)> = Vec::new();
    for e in &events[from..] {
        match e {
            ChatEvent::UserMessage { text, images, .. } => {
                let pics = if images.is_empty() { String::new() } else { format!(" _(+{} image(s))_", images.len()) };
                let _ = write!(out, "\n## User\n{}{pics}\n\n## Agent\n", text.trim());
            }
            ChatEvent::Text { text, .. } | ChatEvent::TextDelta { text, .. } if !text.trim().is_empty() => {
                let _ = writeln!(out, "{}\n", text.trim());
            }
            ChatEvent::ToolStarted { item_id, tool } => {
                let line = format!("{}: {}", tool.name, one_line(&tool.title, 200));
                match tools.iter_mut().find(|t| &t.0 == item_id) {
                    Some(t) => t.1 = line,
                    None => tools.push((item_id.clone(), line)),
                }
            }
            ChatEvent::ToolFinished { item_id, ok, output, diff } => {
                let line = tools.iter().position(|t| &t.0 == item_id).map(|i| tools.remove(i).1).unwrap_or_else(|| "tool".into());
                let stat = diff.as_ref().map(|d| format!(" (+{} −{})", d.added, d.removed)).unwrap_or_default();
                let _ = writeln!(out, "- {} {line}{stat}", if *ok { "✓" } else { "✗" });
                let output = output.trim();
                if !output.is_empty() && (opts.tool_output || !ok) {
                    let max = if opts.tool_output { 4000 } else { 600 };
                    let _ = writeln!(out, "  ```\n{}\n  ```", cut(output, max));
                }
            }
            ChatEvent::Approval { request } => {
                let detail = request.detail.as_deref().map(|d| format!(": `{}`", one_line(d, 200))).unwrap_or_default();
                let _ = writeln!(out, "- ⏸ asked: {}{detail} (request `{}`)", request.title, request.request_id);
            }
            ChatEvent::ApprovalResolved { decision, .. } => {
                let d = match decision {
                    Decision::Allow => "allowed",
                    Decision::AllowAlways => "always allowed",
                    Decision::Deny => "denied",
                };
                let _ = writeln!(out, "  → {d}");
            }
            ChatEvent::Question { request } => {
                for q in &request.questions {
                    let opts: Vec<&str> = q.options.iter().map(|o| o.label.as_str()).collect();
                    let _ = writeln!(out, "- ? asked: {} [{}] (request `{}`)", q.question, opts.join(" / "), request.request_id);
                }
            }
            ChatEvent::QuestionResolved { answers, .. } => {
                let a: Vec<String> = answers.values().map(|v| v.join(", ")).collect();
                let _ = writeln!(out, "  → {}", if a.is_empty() { "skipped".into() } else { a.join("; ") });
            }
            ChatEvent::Plan { items } if !items.is_empty() => {
                let done = items.iter().filter(|i| i.status == oah_agents::event::PlanStatus::Completed).count();
                let _ = writeln!(out, "- ☐ todo list: {done}/{} done", items.len());
            }
            ChatEvent::TurnFinished { usage, cost_usd, duration_ms, interrupted, .. } => {
                let mut bits = vec![];
                if *interrupted {
                    bits.push("stopped".to_string());
                }
                if let Some(ms) = duration_ms {
                    bits.push(format!("{:.1}s", *ms as f64 / 1000.0));
                }
                if let Some(u) = usage {
                    bits.push(format!("{} tokens", u.total()));
                }
                if let Some(c) = cost_usd.filter(|c| *c > 0.0) {
                    bits.push(format!("${c:.3}"));
                }
                let _ = writeln!(out, "\n_— turn ended{}{}_", if bits.is_empty() { "" } else { ": " }, bits.join(", "));
            }
            ChatEvent::Notice { text, .. } => {
                let _ = writeln!(out, "> {}\n", one_line(text, 400));
            }
            ChatEvent::Error { message } => {
                let _ = writeln!(out, "> **Error:** {}\n", one_line(message, 400));
            }
            ChatEvent::Exited { code: Some(c), message } if *c != 0 => {
                let _ = writeln!(
                    out,
                    "> **The agent exited** (code {c}){}\n",
                    message.as_deref().map(|m| format!(": {m}")).unwrap_or_default()
                );
            }
            _ => {}
        }
    }
    // Tools still running (or cut off).
    for (_, line) in tools {
        let _ = writeln!(out, "- … {line}");
    }
    let out = out.trim().to_string();
    if out.is_empty() {
        "_(nothing yet)_".into()
    } else {
        out
    }
}

pub(crate) fn one_line(s: &str, max: usize) -> String {
    cut(&s.split_whitespace().collect::<Vec<_>>().join(" "), max)
}

/// The first `max` bytes (on a char boundary), with a note of how much was left out.
pub(crate) fn cut(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… ({} more characters)", &s[..end], s[end..].chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn log() -> Vec<serde_json::Value> {
        vec![
            json!({"type":"started","sessionId":"s","model":"m","cwd":"/p"}),
            json!({"type":"userMessage","id":"1","text":"first","images":[],"files":[]}),
            json!({"type":"text","itemId":"a","text":"Hi."}),
            json!({"type":"turnFinished","turnId":"t1","usage":null,"costUsd":null,"durationMs":1000,"interrupted":false}),
            json!({"type":"userMessage","id":"2","text":"run the tests","images":[],"files":[]}),
            json!({"type":"toolStarted","itemId":"b","tool":{"kind":"bash","name":"Bash","title":"cargo test","input":{},"path":null}}),
            json!({"type":"toolFinished","itemId":"b","ok":false,"output":"1 failed","diff":null}),
            json!({"type":"approval","request":{"requestId":"r","itemId":null,"kind":"edit","title":"Edit src/a.rs","detail":null,"diff":null,"canRemember":true}}),
            json!({"type":"approvalResolved","requestId":"r","decision":"allow"}),
            json!({"type":"text","itemId":"c","text":"Fixed it."}),
            json!({"type":"turnFinished","turnId":"t2","usage":null,"costUsd":0.02,"durationMs":2500,"interrupted":false}),
            json!({"type":"somethingNew"}),
        ]
    }

    #[test]
    fn renders_the_last_turn() {
        let md = render(&log(), &Options { last_turns: 1, tool_output: false });
        assert!(md.starts_with("_(1 earlier turns not shown)_"));
        assert!(!md.contains("first"));
        assert!(md.contains("## User\nrun the tests"));
        assert!(md.contains("- ✗ Bash: cargo test\n  ```\n1 failed\n  ```"));
        assert!(md.contains("- ⏸ asked: Edit src/a.rs (request `r`)\n  → allowed"));
        assert!(md.contains("Fixed it."));
        assert!(md.ends_with("_— turn ended: 2.5s, $0.020_"));
    }

    #[test]
    fn renders_everything() {
        let md = render(&log(), &Options { last_turns: 0, tool_output: true });
        assert!(md.contains("first") && md.contains("Hi.") && md.contains("Fixed it."));
        assert_eq!(render(&[], &Options { last_turns: 0, tool_output: false }), "_(nothing yet)_");
    }

    #[test]
    fn cuts_long_text() {
        assert_eq!(cut("héllo", 2), "h… (4 more characters)");
        assert_eq!(one_line("a\n  b", 10), "a b");
    }
}
