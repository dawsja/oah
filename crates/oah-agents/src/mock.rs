//! A scripted agent for building and screenshotting the UI without a real CLI
//! (`OAH_MOCK_AGENT=1`). Each prompt plays one turn: thinking, streamed markdown, a read, an
//! edit that asks first, a command, a plan, and usage. A prompt with "plan" in it asks to start
//! on a plan instead, and one with "ask" asks three questions.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::json;

use crate::driver::{Driver, DriverResult, Prompt, Sink, StartOptions};
use crate::event::*;

/// Answers by question id.
type Answers = std::collections::BTreeMap<String, Vec<String>>;

pub struct MockDriver {
    sink: Sink,
    turn: u32,
    interrupted: Arc<AtomicBool>,
    answers: Arc<Mutex<Option<Sender<Decision>>>>,
    replies: Arc<Mutex<Option<Sender<Answers>>>>,
}

impl MockDriver {
    pub fn start(opts: StartOptions, sink: Sink) -> MockDriver {
        sink(ChatEvent::Started {
            session_id: Some(opts.resume.unwrap_or_else(|| "mock-session".into())),
            model: Some(opts.model.unwrap_or_else(|| "mock-model".into())),
            cwd: Some(opts.cwd),
        });
        let c = |name: &str, description: &str, hint: &str| SlashCommand {
            name: name.into(),
            description: description.into(),
            hint: hint.into(),
            skill: false,
        };
        sink(ChatEvent::Commands {
            commands: vec![
                c("init", "Write an AGENTS.md for this project", ""),
                c("review", "Review the uncommitted changes", "[branch]"),
                c("deploy", "Build and ship the current branch", "<environment>"),
            ],
        });
        MockDriver {
            sink,
            turn: 0,
            interrupted: Arc::new(AtomicBool::new(false)),
            answers: Arc::new(Mutex::new(None)),
            replies: Arc::new(Mutex::new(None)),
        }
    }
}

impl Driver for MockDriver {
    fn send(&mut self, prompt: Prompt) -> DriverResult<()> {
        self.turn += 1;
        let n = self.turn;
        let sink = self.sink.clone();
        let stop = self.interrupted.clone();
        stop.store(false, Ordering::SeqCst);
        let (tx, rx) = channel();
        *self.answers.lock().unwrap() = Some(tx);
        sink(ChatEvent::UserMessage { id: prompt.id, text: prompt.text.clone(), images: vec![], files: prompt.files.clone() });
        let lower = prompt.text.to_lowercase();
        if lower.contains("plan") {
            thread::spawn(move || play_plan(n, sink, rx));
        } else if lower.contains("ask") {
            let (qtx, qrx) = channel();
            *self.replies.lock().unwrap() = Some(qtx);
            thread::spawn(move || play_questions(n, sink, qrx));
        } else {
            thread::spawn(move || play(n, &prompt.text, sink, stop, rx));
        }
        Ok(())
    }

    fn interrupt(&mut self) -> DriverResult<()> {
        self.interrupted.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn answer(&mut self, request_id: &str, decision: Decision) -> DriverResult<()> {
        (self.sink)(ChatEvent::ApprovalResolved { request_id: request_id.into(), decision });
        if let Some(tx) = self.answers.lock().unwrap().as_ref() {
            let _ = tx.send(decision);
        }
        Ok(())
    }

    fn answer_question(&mut self, request_id: &str, answers: std::collections::BTreeMap<String, Vec<String>>) -> DriverResult<()> {
        (self.sink)(ChatEvent::QuestionResolved { request_id: request_id.into(), answers: answers.clone() });
        if let Some(tx) = self.replies.lock().unwrap().as_ref() {
            let _ = tx.send(answers);
        }
        Ok(())
    }

    fn stop(&mut self) {
        self.interrupted.store(true, Ordering::SeqCst);
    }
}

const PLAN: &str = "## Save settings safely

1. **Read before write.** `Settings::save` loads the file again and merges the in-memory changes on top.
2. **Atomic write.** Write to `settings.json.tmp`, then rename over the real file.
3. **Tests.** A round-trip test and one that edits the file between load and save.

Files: `crates/oah-core/src/settings.rs`, `crates/oah-core/tests/settings.rs`.";

fn play_plan(n: u32, sink: Sink, answers: Receiver<Decision>) {
    let turn = format!("t{n}");
    sink(ChatEvent::TurnStarted { turn_id: turn.clone() });
    sink(ChatEvent::Text { item_id: format!("{turn}-a"), text: "I looked at how settings are saved. Here's what I'd do.".into() });
    thread::sleep(Duration::from_millis(300));
    sink(ChatEvent::Approval {
        request: ApprovalRequest {
            request_id: format!("{turn}-plan"),
            item_id: None,
            kind: ApprovalKind::Plan,
            title: "Leave plan mode and start on this plan?".into(),
            detail: Some(PLAN.into()),
            diff: None,
            can_remember: false,
        },
    });
    let approved = answers.recv().map(|d| d != Decision::Deny).unwrap_or(false);
    if approved {
        sink(ChatEvent::PermissionChanged { mode: oah_core::settings::PermissionMode::Ask });
    }
    let text = if approved { "Starting on step 1." } else { "OK, I'll keep planning. What should change?" };
    sink(ChatEvent::Text { item_id: format!("{turn}-b"), text: text.into() });
    sink(ChatEvent::TurnFinished { turn_id: turn, usage: None, cost_usd: None, duration_ms: Some(2_100), interrupted: false });
}

fn play_questions(n: u32, sink: Sink, replies: Receiver<Answers>) {
    let turn = format!("t{n}");
    sink(ChatEvent::TurnStarted { turn_id: turn.clone() });
    let o = |label: &str, description: &str| QuestionOption { label: label.into(), description: description.into() };
    sink(ChatEvent::Question {
        request: QuestionRequest {
            request_id: format!("{turn}-q"),
            questions: vec![
                Question {
                    id: "q0".into(),
                    header: "Database".into(),
                    question: "Which database should the cache use?".into(),
                    options: vec![
                        o("SQLite", "One file next to the app"),
                        o("Postgres", "The team's shared server"),
                        o("None", "Keep it in memory"),
                    ],
                    multi: false,
                    other: true,
                    secret: false,
                },
                Question {
                    id: "q1".into(),
                    header: "Tests".into(),
                    question: "Which tests should I add?".into(),
                    options: vec![
                        o("Unit", "Fast, per function"),
                        o("Integration", "Against a real database"),
                        o("End to end", "Through the UI"),
                    ],
                    multi: true,
                    other: false,
                    secret: false,
                },
                Question {
                    id: "q2".into(),
                    header: "Name".into(),
                    question: "What should the module be called?".into(),
                    options: vec![],
                    multi: false,
                    other: true,
                    secret: false,
                },
            ],
        },
    });
    let got = replies.recv().unwrap_or_default();
    let said: Vec<String> = got.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| format!("{k}: {}", v.join(", "))).collect();
    let text = if said.is_empty() { "No answers; I'll wait.".to_string() } else { format!("Got it ({}).", said.join("; ")) };
    sink(ChatEvent::Text { item_id: format!("{turn}-a"), text });
    sink(ChatEvent::TurnFinished { turn_id: turn, usage: None, cost_usd: None, duration_ms: Some(900), interrupted: false });
}

const ANSWER: &str = "Here's what I found in **`src/settings.rs`**: the loader reads the file once at startup and \
never re-checks it, so edits made while the app runs are lost on the next save.\n\n\
I changed it to:\n\n\
1. read the file again right before writing,\n\
2. merge the in-memory changes on top, and\n\
3. write through a temp file so a crash can't leave half a file.\n\n\
```rust\npub fn save(&self, path: &Path) -> Result<()> {\n    let tmp = path.with_extension(\"json.tmp\");\n    std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;\n    std::fs::rename(&tmp, path)?;\n    Ok(())\n}\n```\n\n\
The tests pass (`cargo test -p oah-core`: 5 passed). | Check | Result |\n|---|---|\n| unit tests | 5 passed |\n| clippy | clean |";

fn play(n: u32, text: &str, sink: Sink, stop: Arc<AtomicBool>, answers: Receiver<Decision>) {
    let turn = format!("t{n}");
    let id = |s: &str| format!("{turn}-{s}");
    let pause = |ms: u64| {
        thread::sleep(Duration::from_millis(ms));
        stop.load(Ordering::SeqCst)
    };
    let finish = |interrupted: bool| {
        sink(ChatEvent::TurnFinished {
            turn_id: turn.clone(),
            usage: Some(Usage { input_tokens: 18_420, output_tokens: 1_204, cache_read_tokens: 41_000, cache_write_tokens: 0 }),
            cost_usd: Some(0.0412),
            duration_ms: Some(14_800),
            interrupted,
        });
        let used = 61_000 + n as u64 * 4_000;
        let p = |name: &str, tokens: u64, kind: &str| ContextPart { name: name.into(), tokens, kind: kind.into() };
        sink(ChatEvent::Context {
            used_tokens: used,
            window_tokens: 200_000,
            parts: vec![
                p("Messages", used - 31_000, "used"),
                p("System tools", 18_000, "used"),
                p("Skills", 9_000, "used"),
                p("System prompt", 3_500, "used"),
                p("MCP tools (deferred)", 22_000, "deferred"),
                p("Autocompact buffer", 33_000, "buffer"),
                p("Free space", 200_000 - used - 33_000, "free"),
            ],
        });
    };
    sink(ChatEvent::TurnStarted { turn_id: turn.clone() });
    for chunk in ["The user wants ", "me to look at how settings ", "are saved. I'll read the file first, ", "then fix the write path."] {
        sink(ChatEvent::ReasoningDelta { item_id: id("r"), text: chunk.into() });
        if pause(120) {
            return finish(true);
        }
    }
    sink(ChatEvent::Plan {
        items: vec![
            PlanItem { text: "Read the settings loader".into(), status: PlanStatus::InProgress },
            PlanItem { text: "Make saving atomic".into(), status: PlanStatus::Pending },
            PlanItem { text: "Run the tests".into(), status: PlanStatus::Pending },
        ],
    });
    sink(ChatEvent::ToolStarted {
        item_id: id("read"),
        tool: ToolCall {
            kind: ToolKind::Read,
            name: "Read".into(),
            title: "crates/oah-core/src/settings.rs".into(),
            input: json!({"file_path": "crates/oah-core/src/settings.rs"}),
            path: Some("crates/oah-core/src/settings.rs".into()),
        },
    });
    if pause(500) {
        return finish(true);
    }
    sink(ChatEvent::ToolFinished { item_id: id("read"), ok: true, output: "312 lines".into(), diff: None });
    sink(ChatEvent::ToolStarted {
        item_id: id("grep"),
        tool: ToolCall {
            kind: ToolKind::Search,
            name: "Grep".into(),
            title: "fn save".into(),
            input: json!({"pattern": "fn save"}),
            path: None,
        },
    });
    if pause(350) {
        return finish(true);
    }
    sink(ChatEvent::ToolFinished { item_id: id("grep"), ok: true, output: "crates/oah-core/src/settings.rs:241".into(), diff: None });

    let diff = FileDiff::from_patch(
        "crates/oah-core/src/settings.rs",
        "@@ -238,7 +238,9 @@ impl Settings {\n     pub fn save(&self, path: &Path) -> Result<()> {\n-        std::fs::write(path, serde_json::to_string_pretty(self)?)?;\n+        let tmp = path.with_extension(\"json.tmp\");\n+        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;\n+        std::fs::rename(&tmp, path)?;\n         Ok(())\n     }\n",
    );
    sink(ChatEvent::ToolStarted {
        item_id: id("edit"),
        tool: ToolCall {
            kind: ToolKind::Edit,
            name: "Edit".into(),
            title: "crates/oah-core/src/settings.rs".into(),
            input: json!({}),
            path: Some(diff.path.clone()),
        },
    });
    let request_id = id("approve");
    sink(ChatEvent::Approval {
        request: ApprovalRequest {
            request_id: request_id.clone(),
            item_id: Some(id("edit")),
            kind: ApprovalKind::Edit,
            title: "Edit crates/oah-core/src/settings.rs".into(),
            detail: Some("Write settings through a temp file, then rename it over the old one.".into()),
            diff: Some(diff.clone()),
            can_remember: true,
        },
    });
    let decision = loop {
        match answers.recv_timeout(Duration::from_millis(100)) {
            Ok(d) => break d,
            Err(RecvTimeoutError::Timeout) if !stop.load(Ordering::SeqCst) => continue,
            _ => return finish(true),
        }
    };
    if decision == Decision::Deny {
        sink(ChatEvent::ToolFinished { item_id: id("edit"), ok: false, output: "The user declined this edit.".into(), diff: None });
        sink(ChatEvent::Text { item_id: id("a"), text: "Okay, I left the file as it was.".into() });
        return finish(false);
    }
    sink(ChatEvent::ToolFinished { item_id: id("edit"), ok: true, output: String::new(), diff: Some(diff) });
    sink(ChatEvent::ToolStarted {
        item_id: id("bash"),
        tool: ToolCall {
            kind: ToolKind::Bash,
            name: "Bash".into(),
            title: "cargo test -p oah-core".into(),
            input: json!({"command": "cargo test -p oah-core"}),
            path: None,
        },
    });
    if pause(900) {
        return finish(true);
    }
    sink(ChatEvent::ToolFinished {
        item_id: id("bash"),
        ok: true,
        output: "running 5 tests\ntest ssh::tests::quotes_and_validates ... ok\ntest settings::tests::round_trips_and_fills_defaults ... ok\n\ntest result: ok. 5 passed; 0 failed".into(),
        diff: None,
    });
    sink(ChatEvent::Plan {
        items: vec![
            PlanItem { text: "Read the settings loader".into(), status: PlanStatus::Completed },
            PlanItem { text: "Make saving atomic".into(), status: PlanStatus::Completed },
            PlanItem { text: "Run the tests".into(), status: PlanStatus::Completed },
        ],
    });
    let answer = if text.trim().is_empty() { ANSWER.to_string() } else { ANSWER.replace("| Check", "\n\n| Check") };
    let mut sent = 0;
    while sent < answer.len() {
        let mut end = (sent + 24).min(answer.len());
        while !answer.is_char_boundary(end) {
            end += 1;
        }
        sink(ChatEvent::TextDelta { item_id: id("a"), text: answer[sent..end].into() });
        sent = end;
        if pause(25) {
            return finish(true);
        }
    }
    finish(false);
}
