//! JSON-RPC 2.0 over a child's stdio, one object per line (Codex's app-server and ACP agents):
//! numbered requests whose replies come back through the driver's reader thread.

use std::collections::HashMap;
use std::io::Write;
use std::process::ChildStdin;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::driver::DriverResult;

pub type Replies = Arc<Mutex<HashMap<u64, Sender<Result<Value, String>>>>>;

#[derive(Clone)]
pub struct Rpc {
    pub stdin: Arc<Mutex<Option<ChildStdin>>>,
    pub next: Arc<Mutex<u64>>,
    pub replies: Replies,
    /// "Codex", "OpenCode"… for error messages.
    pub name: String,
}

impl Rpc {
    pub fn new(stdin: Option<ChildStdin>, name: impl Into<String>) -> Rpc {
        Rpc {
            stdin: Arc::new(Mutex::new(stdin)),
            next: Arc::new(Mutex::new(0)),
            replies: Arc::new(Mutex::new(HashMap::new())),
            name: name.into(),
        }
    }

    pub fn write(&self, v: &Value) -> DriverResult<()> {
        let mut g = self.stdin.lock().unwrap();
        let stdin = g.as_mut().ok_or_else(|| format!("{} isn't running", self.name))?;
        let mut line = serde_json::to_vec(v).map_err(|e| e.to_string())?;
        line.push(b'\n');
        stdin.write_all(&line).and_then(|_| stdin.flush()).map_err(|_| format!("{} isn't running", self.name))
    }

    /// Sends a request and waits for its reply.
    pub fn call(&self, method: &str, params: Value, timeout: Duration) -> DriverResult<Value> {
        let id = {
            let mut n = self.next.lock().unwrap();
            *n += 1;
            *n
        };
        let (tx, rx) = channel();
        self.replies.lock().unwrap().insert(id, tx);
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        match rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(_) => {
                self.replies.lock().unwrap().remove(&id);
                Err(format!("{} didn't answer {method}", self.name))
            }
        }
    }

    pub fn notify(&self, method: &str, params: Option<Value>) -> DriverResult<()> {
        let mut m = json!({"jsonrpc": "2.0", "method": method});
        if let Some(p) = params {
            m["params"] = p;
        }
        self.write(&m)
    }

    pub fn reply(&self, id: &Value, result: Value) -> DriverResult<()> {
        self.write(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    pub fn reply_error(&self, id: &Value, code: i64, message: &str) -> DriverResult<()> {
        self.write(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}))
    }

    /// Routes a reply line to its waiting `call`; false if `v` isn't a reply.
    pub fn take_reply(&self, v: &Value) -> bool {
        if v.get("method").is_some() {
            return false;
        }
        let Some(id) = v["id"].as_u64() else { return false };
        if let Some(tx) = self.replies.lock().unwrap().remove(&id) {
            let r = match v.get("error") {
                Some(e) => Err(error_text(e, &self.name)),
                None => Ok(v["result"].clone()),
            };
            let _ = tx.send(r);
        }
        true
    }

    /// Fails every waiting call (the process ended).
    pub fn fail_all(&self) {
        for (_, tx) in self.replies.lock().unwrap().drain() {
            let _ = tx.send(Err(format!("{} exited", self.name)));
        }
    }
}

/// A JSON-RPC error's message, with its data when that says more.
pub fn error_text(e: &Value, name: &str) -> String {
    let msg = e["message"].as_str().unwrap_or("").to_string();
    let data = e["data"]["message"].as_str().or(e["data"].as_str()).map(String::from);
    match (msg.is_empty(), data) {
        (true, None) => format!("{name} error"),
        (false, None) => msg,
        (true, Some(d)) => d,
        (false, Some(d)) if d != msg => format!("{msg}: {d}"),
        (false, Some(_)) => msg,
    }
}
