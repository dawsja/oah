//! MCP over a byte stream: newline-delimited JSON-RPC 2.0, as on stdio (the bridge passes
//! stdio through unchanged). Each request runs on its own thread, so a long wait doesn't hold up
//! pings or other calls; replies share one writer.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::tools::{self, Ctx};

/// Protocol versions this server speaks, newest first.
const VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "oah runs coding agents (Claude Code, Codex, OpenCode, ACP agents) in the user's projects; each chat is a session, shown live in oah's window. \
Typical flow: list_projects → (list_agents / list_models for models and efforts) → create_session with a prompt → read the report it returns. \
A report's `state` says what's next: `needsApproval` (answer_approval), `needsQuestion` (answer_question), `running` (wait_for_session), `idle` (send_message to go on). \
get_changes and get_diff show what a session changed; close_session when done. The user may answer or type in oah too.";

pub const OFF: &str = "MCP is off in oah: turn it on in Settings → MCP.";

type Writer = Arc<Mutex<Box<dyn Write + Send>>>;

fn send(w: &Writer, v: &Value) {
    let mut line = serde_json::to_vec(v).unwrap_or_default();
    line.push(b'\n');
    let mut w = w.lock().unwrap();
    let _ = w.write_all(&line);
    let _ = w.flush();
}

/// Counts open MCP connections (for Settings → MCP).
pub static CLIENTS: AtomicUsize = AtomicUsize::new(0);

/// Serves one MCP client until it hangs up.
pub fn serve(ctx: Arc<Ctx>, reader: impl Read, writer: impl Write + Send + 'static) {
    CLIENTS.fetch_add(1, Ordering::SeqCst);
    let writer: Writer = Arc::new(Mutex::new(Box::new(writer)));
    let cancelled: Arc<Mutex<HashSet<String>>> = Arc::default();
    let mut threads = Vec::new();
    for line in BufReader::new(reader).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                send(&writer, &json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}}));
                continue;
            }
        };
        // A batch (older clients) is handled one by one.
        let msgs = match msg {
            Value::Array(a) => a,
            m => vec![m],
        };
        for m in msgs {
            let method = m.get("method").and_then(Value::as_str).unwrap_or("").to_string();
            if method == "notifications/cancelled" {
                if let Some(id) = m.pointer("/params/requestId") {
                    cancelled.lock().unwrap().insert(id.to_string());
                }
                continue;
            }
            let Some(id) = m.get("id").cloned() else { continue }; // other notifications, responses
            if method.is_empty() {
                continue; // a response to something we never ask
            }
            let (ctx, writer, cancelled) = (ctx.clone(), writer.clone(), cancelled.clone());
            threads.push(std::thread::spawn(move || {
                let params = m.get("params").cloned().unwrap_or(Value::Null);
                let reply = match handle(&ctx, &method, params, &writer) {
                    Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                    Err((code, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
                };
                // A cancelled request gets no reply.
                if !cancelled.lock().unwrap().remove(&id.to_string()) {
                    send(&writer, &reply);
                }
            }));
            threads.retain(|t| !t.is_finished());
        }
    }
    CLIENTS.fetch_sub(1, Ordering::SeqCst);
}

fn handle(ctx: &Ctx, method: &str, params: Value, writer: &Writer) -> Result<Value, (i64, String)> {
    let enabled = ctx.host.settings().mcp.enabled;
    match method {
        "initialize" => {
            if !enabled {
                return Err((-32000, OFF.into()));
            }
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(VERSIONS[0]);
            let version = VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(VERSIONS[0]);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "oah", "title": "oah", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools::list(&ctx.host.settings())})),
        "tools/call" => {
            if !enabled {
                return Ok(json!({"content": [{"type": "text", "text": OFF}], "isError": true}));
            }
            let name = params.get("name").and_then(Value::as_str).ok_or((-32602, "tools/call needs a name".to_string()))?;
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            let token = params.pointer("/_meta/progressToken").cloned();
            let mut n = 0u64;
            let mut progress = |message: String| {
                if let Some(t) = &token {
                    n += 1;
                    send(
                        writer,
                        &json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": {"progressToken": t, "progress": n, "message": message}}),
                    );
                }
            };
            Ok(match tools::call(ctx, name, args, &mut progress) {
                Ok(text) => json!({"content": [{"type": "text", "text": text}]}),
                Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
            })
        }
        // Clients may probe for these; there are none.
        "resources/list" => Ok(json!({"resources": []})),
        "resources/templates/list" => Ok(json!({"resourceTemplates": []})),
        "prompts/list" => Ok(json!({"prompts": []})),
        "logging/setLevel" => Ok(json!({})),
        m => Err((-32601, format!("no method {m}"))),
    }
}

// ---- The local socket ------------------------------------------------------------------------

use interprocess::local_socket::{prelude::*, ListenerOptions, Stream};

#[cfg(windows)]
fn name(path: &str) -> std::io::Result<interprocess::local_socket::Name<'_>> {
    path.to_ns_name::<interprocess::local_socket::GenericNamespaced>()
}

#[cfg(not(windows))]
fn name(path: &str) -> std::io::Result<interprocess::local_socket::Name<'_>> {
    path.to_fs_name::<interprocess::local_socket::GenericFilePath>()
}

/// Accepts MCP clients on the local socket `path` ([`oah_core::paths::mcp_socket`]) on a
/// background thread, for as long as the app runs. Every connection is served (and told MCP is
/// off while it is), so `oah mcp` can tell "oah isn't running" from "MCP is off".
pub fn listen(path: &str, ctx: Arc<Ctx>) -> std::io::Result<()> {
    #[cfg(unix)]
    if let Some(dir) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(dir)?;
    }
    let listener = ListenerOptions::new().name(name(path)?).try_overwrite(true).create_sync()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Only this user may connect.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    std::thread::Builder::new().name("oah-mcp".into()).spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { continue };
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let conn = Shared(Arc::new(conn));
                serve(ctx, conn.clone(), conn);
            });
        }
    })?;
    Ok(())
}

/// One connection, read on one thread and written on others (`&Stream` is both).
#[derive(Clone)]
pub struct Shared(pub Arc<Stream>);

impl Read for Shared {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        (&*self.0).read(buf)
    }
}

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        (&*self.0).write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        (&*self.0).flush()
    }
}

/// Connects to the app's socket.
pub fn connect(path: &str) -> std::io::Result<Stream> {
    Stream::connect(name(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::*;
    use crate::hub::Hub;
    use oah_agents::driver::Prompt;
    use oah_agents::event::{ChatEvent, Decision, ModelOption, SlashCommand};
    use oah_core::chats::ChatMeta;
    use oah_core::settings::{AgentKind, Location, PermissionMode, Project, Settings};
    use std::collections::BTreeMap;

    /// A host with one project whose "agent" answers every message in the hub straight away.
    struct Fake {
        hub: Arc<Hub>,
        settings: Mutex<Settings>,
        chats: Mutex<Vec<ChatMeta>>,
        events: Mutex<Vec<Value>>,
        sent: Mutex<Vec<(String, Option<PermissionMode>)>>,
    }

    impl Host for Fake {
        fn settings(&self) -> Settings {
            self.settings.lock().unwrap().clone()
        }
        fn worktrees(&self, _: &str) -> HostResult<Vec<WorktreeInfo>> {
            Ok(vec![WorktreeInfo { path: "/p".into(), branch: Some("main".into()), main: true }])
        }
        fn create_worktree(&self, _: &str, branch: &str) -> HostResult<WorktreeInfo> {
            Ok(WorktreeInfo { path: format!("/wt/{branch}"), branch: Some(branch.into()), main: false })
        }
        fn check_agent(&self, _: AgentKind, _: Option<&str>, _: &Location) -> AgentCheck {
            AgentCheck { installed: true, signed_in: Some(true), ..Default::default() }
        }
        fn catalog(&self, _: AgentKind, _: Option<&str>) -> HostResult<(Vec<ModelOption>, Vec<SlashCommand>)> {
            Ok((
                vec![ModelOption {
                    id: "opus".into(),
                    name: "Opus".into(),
                    efforts: vec!["low".into(), "high".into()],
                    default_effort: Some("high".into()),
                    ..Default::default()
                }],
                vec![],
            ))
        }
        fn chats(&self) -> Vec<ChatMeta> {
            self.chats.lock().unwrap().clone()
        }
        fn running(&self) -> Vec<String> {
            self.chats.lock().unwrap().iter().map(|c| c.id.clone()).collect()
        }
        fn create(&self, c: NewChat) -> HostResult<ChatMeta> {
            let m = ChatMeta {
                id: format!("c{}", self.chats.lock().unwrap().len()),
                project_id: c.project_id,
                agent: c.agent.unwrap_or(AgentKind::Claude),
                cwd: c.cwd.unwrap_or_else(|| "/p".into()),
                ..Default::default()
            };
            self.chats.lock().unwrap().push(m.clone());
            Ok(m)
        }
        fn send(&self, id: &str, p: Prompt, perm: Option<PermissionMode>) -> HostResult<()> {
            self.sent.lock().unwrap().push((p.text.clone(), perm));
            let evs = vec![
                ChatEvent::UserMessage { id: p.id, text: p.text.clone(), images: vec![], files: vec![] },
                ChatEvent::Text { item_id: "a".into(), text: format!("You said {}", p.text) },
            ];
            for e in &evs {
                self.hub.push(id, e);
                self.events.lock().unwrap().push(serde_json::to_value(e).unwrap());
            }
            let (hub, id, ask) = (self.hub.clone(), id.to_string(), p.text.contains("ask"));
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(20));
                if ask {
                    hub.push(&id, &serde_json::from_value(json!({"type":"approval","request":{"requestId":"r1","itemId":null,"kind":"command","title":"Run a command","detail":"ls","diff":null,"canRemember":true}})).unwrap());
                } else {
                    hub.push(
                        &id,
                        &ChatEvent::TurnFinished {
                            turn_id: "t".into(),
                            usage: None,
                            cost_usd: None,
                            duration_ms: Some(5),
                            interrupted: false,
                        },
                    );
                }
            });
            Ok(())
        }
        fn interrupt(&self, _: &str) -> HostResult<()> {
            Ok(())
        }
        fn answer(&self, id: &str, r: &str, d: Decision) -> HostResult<()> {
            self.hub.push(id, &ChatEvent::ApprovalResolved { request_id: r.into(), decision: d });
            Ok(())
        }
        fn answer_question(&self, _: &str, _: &str, _: BTreeMap<String, Vec<String>>) -> HostResult<()> {
            Ok(())
        }
        fn set_model(&self, _: &str, _: &str) -> HostResult<()> {
            Ok(())
        }
        fn set_effort(&self, _: &str, _: &str) -> HostResult<()> {
            Ok(())
        }
        fn set_permission(&self, _: &str, _: PermissionMode) -> HostResult<()> {
            Ok(())
        }
        fn compact(&self, _: &str, _: &str) -> HostResult<()> {
            Ok(())
        }
        fn clear(&self, _: &str) -> HostResult<()> {
            Ok(())
        }
        fn rename(&self, id: &str, title: &str) -> HostResult<ChatMeta> {
            let mut chats = self.chats.lock().unwrap();
            let m = chats.iter_mut().find(|c| c.id == id).ok_or("no such chat")?;
            m.title = title.into();
            Ok(m.clone())
        }
        fn close(&self, _: &str) {}
        fn delete(&self, _: &str) -> HostResult<()> {
            Ok(())
        }
        fn events(&self, _: &str) -> HostResult<Vec<Value>> {
            Ok(self.events.lock().unwrap().clone())
        }
        fn git_status(&self, _: &str, cwd: Option<&str>) -> HostResult<Value> {
            Ok(json!({"cwd": cwd}))
        }
        fn git_diff(&self, _: &str, _: Option<&str>, _: &str) -> HostResult<String> {
            Ok(String::new())
        }
    }

    fn fake(enabled: bool) -> (Arc<Ctx>, Arc<Fake>) {
        let hub = Arc::new(Hub::new());
        let mut settings = Settings::default();
        settings.mcp.enabled = enabled;
        settings.projects.push(Project { id: "p1".into(), name: "demo".into(), path: "/p".into(), ..Default::default() });
        let host = Arc::new(Fake {
            hub: hub.clone(),
            settings: Mutex::new(settings),
            chats: Mutex::default(),
            events: Mutex::default(),
            sent: Mutex::default(),
        });
        (Arc::new(Ctx::new(host.clone(), hub)), host)
    }

    /// Plays `requests` to a server over pipes; returns every line it wrote.
    fn talk(ctx: Arc<Ctx>, requests: &[Value]) -> Vec<Value> {
        let input: String = requests.iter().map(|r| format!("{r}\n")).collect();
        #[derive(Clone, Default)]
        struct Out(Arc<Mutex<Vec<u8>>>);
        impl Write for Out {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let out = Out::default();
        serve(ctx, std::io::Cursor::new(input.into_bytes()), out.clone());
        // Requests run on their own threads; wait for their replies.
        let want = requests.iter().filter(|r| r.get("id").is_some()).count();
        for _ in 0..200 {
            let n = String::from_utf8_lossy(&out.0.lock().unwrap())
                .lines()
                .filter(|l| l.contains("\"result\"") || l.contains("\"error\""))
                .count();
            if n >= want {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        text.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    fn reply(lines: &[Value], id: i64) -> Value {
        lines.iter().find(|l| l["id"] == id).cloned().unwrap_or(Value::Null)
    }

    fn call(id: i64, name: &str, args: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}})
    }

    fn text(r: &Value) -> String {
        r["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
    }

    #[test]
    fn initializes_and_lists_tools() {
        let (ctx, _) = fake(true);
        let lines = talk(
            ctx,
            &[
                json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}}),
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
                json!({"jsonrpc": "2.0", "id": 3, "method": "nope"}),
            ],
        );
        let init = reply(&lines, 1);
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(init["result"]["serverInfo"]["name"], "oah");
        let tools = reply(&lines, 2)["result"]["tools"].as_array().unwrap().clone();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"create_session") && names.contains(&"list_models"));
        assert!(!names.contains(&"delete_session"), "delete is off by default");
        assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));
        assert_eq!(reply(&lines, 3)["error"]["code"], -32601);
    }

    #[test]
    fn serves_over_the_local_socket() {
        use std::io::BufRead;
        let (ctx, _) = fake(true);
        let path = if cfg!(windows) {
            format!("oah-mcp-test-{}", std::process::id())
        } else {
            std::env::temp_dir().join(format!("oah-mcp-test-{}.sock", std::process::id())).to_string_lossy().into_owned()
        };
        listen(&path, ctx).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let conn = Shared(Arc::new(connect(&path).unwrap()));
        let mut w = conn.clone();
        let mut lines = std::io::BufReader::new(conn).lines();
        for (id, method) in [(1, "initialize"), (2, "ping")] {
            writeln!(w, "{}", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": {"protocolVersion": "2024-11-05"}})).unwrap();
            let reply: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
            assert_eq!(reply["id"], id);
            assert!(reply.get("result").is_some(), "{reply}");
        }
        writeln!(w, "{}", call(3, "list_projects", json!({}))).unwrap();
        let reply: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        assert!(text(&reply).contains("\"id\": \"p1\""), "{reply}");
    }

    #[test]
    fn refuses_while_off() {
        let (ctx, _) = fake(false);
        let lines =
            talk(ctx, &[json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}), call(2, "list_projects", json!({}))]);
        assert_eq!(reply(&lines, 1)["error"]["message"], OFF);
        assert_eq!(reply(&lines, 2)["result"]["isError"], true);
    }

    #[test]
    fn creates_a_session_and_runs_a_turn() {
        let (ctx, host) = fake(true);
        let lines = talk(
            ctx.clone(),
            &[call(
                1,
                "create_session",
                json!({"project_id": "p1", "worktree": "feature", "title": "Fix", "prompt": "hello", "permission": "acceptEdits"}),
            )],
        );
        let t = text(&reply(&lines, 1));
        assert!(t.starts_with("Session c0 created."), "{t}");
        assert!(t.contains("\"reply\": \"You said hello\""), "{t}");
        assert!(t.contains("\"state\": \"idle\""), "{t}");
        let chat = host.chats().remove(0);
        assert_eq!(chat.cwd, "/wt/feature");
        assert_eq!(chat.title, "Fix");
        assert_eq!(host.sent.lock().unwrap()[0], ("hello".to_string(), Some(PermissionMode::AcceptEdits)));

        // A turn that asks first returns with the approval, which can then be answered.
        let lines = talk(
            ctx.clone(),
            &[
                json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "send_message", "arguments": {"session_id": "c0", "text": "ask me"}, "_meta": {"progressToken": "p"}}}),
            ],
        );
        let t = text(&reply(&lines, 2));
        assert!(t.contains("\"state\": \"needsApproval\"") && t.contains("\"requestId\": \"r1\""), "{t}");
        let lines = talk(ctx.clone(), &[call(3, "answer_approval", json!({"session_id": "c0", "request_id": "r1", "decision": "allow"}))]);
        assert!(text(&reply(&lines, 3)).starts_with("Answered"));

        let lines = talk(ctx, &[call(4, "get_transcript", json!({"session_id": "c0", "last_turns": 0}))]);
        let md = text(&reply(&lines, 4));
        assert!(md.contains("## User\nhello") && md.contains("You said ask me"), "{md}");
    }

    #[test]
    fn guards_full_access_and_bad_input() {
        let (ctx, _) = fake(true);
        let lines = talk(
            ctx,
            &[
                call(1, "create_session", json!({"project_id": "p1", "permission": "fullAccess"})),
                call(2, "send_message", json!({"session_id": "zz", "text": "hi"})),
                call(3, "list_models", json!({"agent": "claude"})),
                call(4, "create_session", json!({"project_id": "p1", "agent": "nobody"})),
            ],
        );
        let r1 = reply(&lines, 1);
        assert_eq!(r1["result"]["isError"], true);
        assert!(text(&r1).contains("Full access isn't allowed"));
        assert!(text(&reply(&lines, 2)).contains("no session zz"));
        assert!(text(&reply(&lines, 3)).contains("\"efforts\""));
        assert!(text(&reply(&lines, 4)).starts_with("bad arguments"));
    }
}
