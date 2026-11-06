//! Model Context Protocol client: stdio and Streamable HTTP transports.
//!
//! Servers are configured in `~/.nar/mcp.json`, `./.nar/mcp.json` or `./.mcp.json`:
//!
//! ```json
//! { "mcpServers": {
//!     "fs":   { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "."] },
//!     "docs": { "url": "https://example.com/mcp", "headers": { "Authorization": "Bearer …" } }
//! } }
//! ```
//!
//! Each server's tools are exposed to the model as `mcp__<server>__<tool>`.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{oneshot, Mutex, RwLock};

const PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
pub struct ServerConfig {
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: Option<String>,
    pub url: Option<String>,
    pub headers: HashMap<String, String>,
    pub disabled: bool,
    #[serde(rename = "type")]
    pub kind: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ConfigFile {
    #[serde(rename = "mcpServers", alias = "servers", alias = "mcp_servers")]
    servers: HashMap<String, ServerConfig>,
}

pub fn load_config(cwd: &Path) -> Vec<(String, ServerConfig, String)> {
    let files = [
        crate::config::home_dir().join("mcp.json"),
        cwd.join(".nar").join("mcp.json"),
        cwd.join(".mcp.json"),
    ];
    let mut out: Vec<(String, ServerConfig, String)> = Vec::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let Ok(cfg) = serde_json::from_str::<ConfigFile>(&text) else { continue };
        let mut names: Vec<_> = cfg.servers.into_iter().collect();
        names.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, sc) in names {
            out.retain(|(n, _, _)| n != &name);
            out.push((name, sc, crate::config::short_path(&f)));
        }
    }
    out.into_iter().filter(|(_, c, _)| !c.disabled).collect()
}

#[derive(Clone, Debug)]
pub struct McpTool {
    pub server: String,
    pub name: String,
    pub exposed: String,
    pub description: String,
    pub schema: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerState {
    Starting,
    Ready,
    Failed(String),
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

enum Transport {
    Stdio {
        stdin: Mutex<tokio::process::ChildStdin>,
        pending: Pending,
        _child: Mutex<tokio::process::Child>,
    },
    Http {
        http: reqwest::Client,
        url: String,
        headers: HashMap<String, String>,
        session: Mutex<Option<String>>,
    },
}

pub struct Server {
    pub name: String,
    pub source: String,
    pub state: RwLock<ServerState>,
    pub tools: RwLock<Vec<McpTool>>,
    transport: RwLock<Option<Arc<Transport>>>,
    next_id: AtomicU64,
}

pub fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
}

pub fn exposed_name(server: &str, tool: &str) -> String {
    let mut n = format!("mcp__{}__{}", sanitize(server), sanitize(tool));
    if n.len() > 64 {
        n.truncate(64);
    }
    n
}

impl Server {
    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let t = self.transport.read().await.clone().ok_or_else(|| anyhow!("server not running"))?;
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let msg = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let resp = match &*t {
            Transport::Stdio { stdin, pending, .. } => {
                let (tx, rx) = oneshot::channel();
                pending.lock().await.insert(id, tx);
                {
                    let mut w = stdin.lock().await;
                    let mut line = serde_json::to_vec(&msg)?;
                    line.push(b'\n');
                    w.write_all(&line).await?;
                    w.flush().await?;
                }
                match tokio::time::timeout(Duration::from_secs(600), rx).await {
                    Ok(Ok(v)) => v,
                    Ok(Err(_)) => bail!("server closed the connection"),
                    Err(_) => {
                        pending.lock().await.remove(&id);
                        bail!("request timed out")
                    }
                }
            }
            Transport::Http { http, url, headers, session } => {
                let mut req = http
                    .post(url)
                    .header("Content-Type", "application/json")
                    .header("Accept", "application/json, text/event-stream")
                    .header("MCP-Protocol-Version", PROTOCOL_VERSION);
                for (k, v) in headers {
                    req = req.header(k, v);
                }
                if let Some(s) = session.lock().await.clone() {
                    req = req.header("Mcp-Session-Id", s);
                }
                let r = req.json(&msg).send().await?;
                if let Some(sid) = r.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
                    *session.lock().await = Some(sid.to_string());
                }
                let status = r.status();
                let ct = r.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                let body = r.text().await?;
                if !status.is_success() {
                    bail!("HTTP {status}: {}", body.chars().take(300).collect::<String>());
                }
                if ct.contains("text/event-stream") {
                    let mut found = Value::Null;
                    for block in body.split("\n\n") {
                        let data: String = block
                            .lines()
                            .filter_map(|l| l.strip_prefix("data:").map(|d| d.trim_start()))
                            .collect::<Vec<_>>()
                            .join("\n");
                        if let Ok(v) = serde_json::from_str::<Value>(&data) {
                            if v.get("id").and_then(|i| i.as_u64()) == Some(id) {
                                found = v;
                                break;
                            }
                        }
                    }
                    found
                } else {
                    serde_json::from_str(&body)?
                }
            }
        };
        if let Some(err) = resp.get("error") {
            bail!("{}", err.get("message").and_then(|m| m.as_str()).unwrap_or("MCP error"));
        }
        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn notify(&self, method: &str) -> Result<()> {
        let t = self.transport.read().await.clone().ok_or_else(|| anyhow!("server not running"))?;
        let msg = json!({"jsonrpc":"2.0","method":method});
        match &*t {
            Transport::Stdio { stdin, .. } => {
                let mut w = stdin.lock().await;
                let mut line = serde_json::to_vec(&msg)?;
                line.push(b'\n');
                w.write_all(&line).await?;
                w.flush().await?;
            }
            Transport::Http { http, url, headers, session } => {
                let mut req = http
                    .post(url)
                    .header("Content-Type", "application/json")
                    .header("Accept", "application/json, text/event-stream")
                    .header("MCP-Protocol-Version", PROTOCOL_VERSION);
                for (k, v) in headers {
                    req = req.header(k, v);
                }
                if let Some(s) = session.lock().await.clone() {
                    req = req.header("Mcp-Session-Id", s);
                }
                let _ = req.json(&msg).send().await;
            }
        }
        Ok(())
    }

    async fn start(self: &Arc<Self>, cfg: ServerConfig, cwd: &Path, http: reqwest::Client) -> Result<()> {
        let transport = if let Some(url) = cfg.url.clone() {
            Transport::Http { http, url, headers: cfg.headers.clone(), session: Mutex::new(None) }
        } else {
            let command = cfg.command.clone().ok_or_else(|| anyhow!("missing \"command\" or \"url\""))?;
            let mut cmd = tokio::process::Command::new(&command);
            cmd.args(&cfg.args)
                .envs(&cfg.env)
                .current_dir(cfg.cwd.as_deref().map(Path::new).unwrap_or(cwd))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            let mut child = cmd.spawn().map_err(|e| anyhow!("failed to start `{command}`: {e}"))?;
            let stdin = child.stdin.take().unwrap();
            let stdout = child.stdout.take().unwrap();
            let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
            let reader_pending = pending.clone();
            let me = Arc::downgrade(self);
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                    let has_method = v.get("method").is_some();
                    match (v.get("id").and_then(|i| i.as_u64()), has_method) {
                        (Some(id), false) => {
                            if let Some(tx) = reader_pending.lock().await.remove(&id) {
                                let _ = tx.send(v);
                            }
                        }
                        (Some(_), true) => {
                            // Server → client request (ping, roots/list…). Answer minimally.
                            if let Some(server) = me.upgrade() {
                                let reply = match v["method"].as_str() {
                                    Some("ping") => json!({"jsonrpc":"2.0","id":v["id"],"result":{}}),
                                    Some("roots/list") => json!({"jsonrpc":"2.0","id":v["id"],"result":{"roots":[]}}),
                                    _ => json!({"jsonrpc":"2.0","id":v["id"],"error":{"code":-32601,"message":"method not found"}}),
                                };
                                if let Some(t) = server.transport.read().await.clone() {
                                    if let Transport::Stdio { stdin, .. } = &*t {
                                        let mut w = stdin.lock().await;
                                        let mut b = serde_json::to_vec(&reply).unwrap_or_default();
                                        b.push(b'\n');
                                        let _ = w.write_all(&b).await;
                                        let _ = w.flush().await;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                // EOF: fail outstanding requests.
                reader_pending.lock().await.clear();
                if let Some(server) = me.upgrade() {
                    let mut st = server.state.write().await;
                    if *st == ServerState::Ready || *st == ServerState::Starting {
                        *st = ServerState::Failed("process exited".into());
                    }
                }
            });
            Transport::Stdio { stdin: Mutex::new(stdin), pending, _child: Mutex::new(child) }
        };
        *self.transport.write().await = Some(Arc::new(transport));
        let init = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {"name": "nar", "version": crate::config::VERSION}
                }),
            )
            .await?;
        let _ = init;
        self.notify("notifications/initialized").await?;
        self.refresh_tools().await?;
        *self.state.write().await = ServerState::Ready;
        Ok(())
    }

    async fn refresh_tools(&self) -> Result<()> {
        let mut cursor: Option<String> = None;
        let mut tools = Vec::new();
        for _ in 0..50 {
            let params = match &cursor {
                Some(c) => json!({"cursor": c}),
                None => json!({}),
            };
            let r = self.request("tools/list", params).await?;
            for t in r["tools"].as_array().cloned().unwrap_or_default() {
                let name = t["name"].as_str().unwrap_or("").to_string();
                if name.is_empty() {
                    continue;
                }
                let mut schema = t.get("inputSchema").cloned().unwrap_or(json!({"type":"object"}));
                if schema.get("type").is_none() {
                    schema["type"] = json!("object");
                }
                if schema.get("properties").is_none() {
                    schema["properties"] = json!({});
                }
                tools.push(McpTool {
                    exposed: exposed_name(&self.name, &name),
                    server: self.name.clone(),
                    description: t["description"].as_str().unwrap_or("").chars().take(1024).collect(),
                    schema,
                    name,
                });
            }
            cursor = r["nextCursor"].as_str().map(String::from);
            if cursor.is_none() {
                break;
            }
        }
        *self.tools.write().await = tools;
        Ok(())
    }

    pub async fn call(&self, tool: &str, args: Value) -> Result<(String, bool)> {
        let r = self.request("tools/call", json!({"name": tool, "arguments": args})).await?;
        let is_error = r["isError"].as_bool().unwrap_or(false);
        let mut parts = Vec::new();
        for c in r["content"].as_array().cloned().unwrap_or_default() {
            match c["type"].as_str() {
                Some("text") => parts.push(c["text"].as_str().unwrap_or("").to_string()),
                Some("image") => parts.push(format!("[image {}]", c["mimeType"].as_str().unwrap_or(""))),
                Some("resource") => {
                    let res = &c["resource"];
                    parts.push(res["text"].as_str().map(String::from).unwrap_or_else(|| format!("[resource {}]", res["uri"])));
                }
                Some("resource_link") => parts.push(format!("[resource {}]", c["uri"].as_str().unwrap_or(""))),
                _ => parts.push(c.to_string()),
            }
        }
        if parts.is_empty() {
            if let Some(sc) = r.get("structuredContent") {
                parts.push(sc.to_string());
            }
        }
        Ok((parts.join("\n"), is_error))
    }
}

#[derive(Default, Clone)]
pub struct McpManager {
    pub servers: Arc<RwLock<Vec<Arc<Server>>>>,
}

impl McpManager {
    /// Start every configured server in the background.
    pub fn start(cwd: &Path, http: reqwest::Client, on_change: impl Fn() + Send + Sync + 'static) -> McpManager {
        let mgr = McpManager::default();
        let configs = load_config(cwd);
        let on_change = Arc::new(on_change);
        let servers: Vec<Arc<Server>> = configs
            .iter()
            .map(|(name, _, src)| {
                Arc::new(Server {
                    name: name.clone(),
                    source: src.clone(),
                    state: RwLock::new(ServerState::Starting),
                    tools: RwLock::new(vec![]),
                    transport: RwLock::new(None),
                    next_id: AtomicU64::new(1),
                })
            })
            .collect();
        let list = mgr.servers.clone();
        let cwd = cwd.to_path_buf();
        tokio::spawn(async move {
            *list.write().await = servers.clone();
            for (server, (_, cfg, _)) in servers.into_iter().zip(configs) {
                let http = http.clone();
                let cwd = cwd.clone();
                let on_change = on_change.clone();
                tokio::spawn(async move {
                    let r = tokio::time::timeout(Duration::from_secs(60), server.start(cfg, &cwd, http)).await;
                    let failed = match r {
                        Ok(Ok(())) => None,
                        Ok(Err(e)) => Some(e.to_string()),
                        Err(_) => Some("timed out during startup".into()),
                    };
                    if let Some(e) = failed {
                        *server.state.write().await = ServerState::Failed(e);
                    }
                    on_change();
                });
            }
        });
        mgr
    }

    pub async fn tools(&self) -> Vec<McpTool> {
        let mut out = Vec::new();
        for s in self.servers.read().await.iter() {
            if *s.state.read().await == ServerState::Ready {
                out.extend(s.tools.read().await.iter().cloned());
            }
        }
        out
    }

    pub async fn specs(&self) -> Vec<Value> {
        self.tools()
            .await
            .into_iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "name": t.exposed,
                    "description": format!("[MCP {}] {}", t.server, t.description),
                    "parameters": t.schema,
                    "strict": false,
                })
            })
            .collect()
    }

    pub async fn call(&self, exposed: &str, args: Value) -> Result<(String, bool)> {
        for s in self.servers.read().await.iter() {
            let found = s.tools.read().await.iter().find(|t| t.exposed == exposed).map(|t| t.name.clone());
            if let Some(name) = found {
                return s.call(&name, args).await;
            }
        }
        bail!("unknown MCP tool {exposed}")
    }

    /// (name, state, tool count, source) for `/mcp`.
    pub async fn summary(&self) -> Vec<(String, ServerState, Vec<String>, String)> {
        let mut out = Vec::new();
        for s in self.servers.read().await.iter() {
            let tools = s.tools.read().await.iter().map(|t| t.name.clone()).collect();
            out.push((s.name.clone(), s.state.read().await.clone(), tools, s.source.clone()));
        }
        out
    }
}
