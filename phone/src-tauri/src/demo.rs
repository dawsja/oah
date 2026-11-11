//! A desktop to try the app against, inside the app: for someone who doesn't have oah on a
//! computer yet (Apple's reviewers among them), and for driving the real app in a test. It is
//! the real server (`oah_remote::server`) on this phone's loopback with a scripted host behind
//! it, so pairing, the session and every screen work as they do against a real desktop. What it
//! shows is `demo/demo.json`; nothing is saved, and it's gone when the app closes.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oah_remote::client;
use oah_remote::server::{Feed, Server};
use oah_remote::Identity;
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::Desktop;

const DATA: &str = include_str!("../demo/demo.json");
/// Between the pieces of an answer as it streams in.
const TICK: Duration = Duration::from_millis(45);
/// Before a step that isn't text (a tool starting, finishing).
const STEP: Duration = Duration::from_millis(350);

/// The demo while it runs: dropping it stops its server.
pub struct Demo {
    pub desktop: Desktop,
    _server: Server,
}

/// Starts the demo desktop and pairs this phone (`me`) with it, over loopback.
pub async fn start(me: &Identity) -> Result<Demo, String> {
    let host = Arc::new(Host::new());
    let server = Server::bind(([127, 0, 0, 1], 0).into(), host).await.map_err(|e| e.to_string())?;
    let address = server.local_addr().to_string();
    let code = server.open_pairing().code;
    let paired = client::pair(std::slice::from_ref(&address), &code, "This phone", me).await?;
    let desktop = Desktop {
        id: oah_remote::b64(&paired.desktop_key),
        name: paired.desktop_name,
        addresses: vec![address.clone()],
        device_id: paired.device_id,
        paired_at: now(),
        last_address: Some(address),
        demo: true,
    };
    Ok(Demo { desktop, _server: server })
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

struct Chat {
    meta: Value,
    events: Vec<Value>,
    /// What it waits on the person for: "approval" or "question".
    waiting: Option<String>,
    /// A turn is under way (streaming, or waiting on the person).
    busy: bool,
    /// Stop was pressed during this turn.
    stop: bool,
}

struct Inner {
    data: Value,
    chats: Mutex<Vec<Chat>>,
    tx: broadcast::Sender<Feed>,
    next: AtomicU64,
}

impl Inner {
    fn id(&self, prefix: &str) -> String {
        format!("{prefix}{}", self.next.fetch_add(1, Ordering::Relaxed))
    }

    /// Adds an event to a chat and feeds it, numbered, under one lock (as the app does).
    fn emit(&self, chat_id: &str, event: Value) {
        let mut chats = self.chats.lock().unwrap();
        if let Some(c) = chats.iter_mut().find(|c| c.meta["id"] == chat_id) {
            c.events.push(event.clone());
            let _ = self.tx.send(Feed::Chat { chat_id: chat_id.into(), seq: c.events.len() as u64, event: Arc::new(event) });
        }
    }

    /// Changes a chat and tells every session the list changed; returns the chat's meta.
    fn change(&self, chat_id: &str, f: impl FnOnce(&mut Chat)) -> Result<Value, String> {
        let meta = {
            let mut chats = self.chats.lock().unwrap();
            let c = chats.iter_mut().find(|c| c.meta["id"] == chat_id).ok_or("That chat isn't in the demo anymore.")?;
            f(c);
            c.meta.clone()
        };
        let _ = self.tx.send(Feed::ChatsChanged);
        Ok(meta)
    }

    fn stopped(&self, chat_id: &str) -> bool {
        self.chats.lock().unwrap().iter().find(|c| c.meta["id"] == chat_id).is_none_or(|c| c.stop)
    }

    fn list(&self) -> Value {
        let chats = self.chats.lock().unwrap();
        let busy: Vec<&Value> = chats.iter().filter(|c| c.busy).map(|c| &c.meta["id"]).collect();
        let waiting: serde_json::Map<String, Value> =
            chats.iter().filter_map(|c| Some((c.meta["id"].as_str()?.to_string(), json!(c.waiting.as_ref()?)))).collect();
        json!({ "metas": chats.iter().map(|c| &c.meta).collect::<Vec<_>>(), "running": busy, "busy": busy, "waiting": waiting, "cli": [] })
    }

    /// Plays the rest of a turn: each step after a pause, text a few words at a time (Stop ends
    /// it there), then the turn's end.
    async fn play(self: Arc<Self>, chat_id: String, turn: String, steps: Vec<Value>) {
        let started = Instant::now();
        let mut interrupted = false;
        'steps: for step in steps {
            if step["type"] != "text" {
                tokio::time::sleep(STEP).await;
                if self.stopped(&chat_id) {
                    interrupted = true;
                    break;
                }
                self.emit(&chat_id, step);
                continue;
            }
            let (item, text) = (step["itemId"].clone(), step["text"].as_str().unwrap_or_default());
            let mut said = String::new();
            for piece in pieces(text) {
                tokio::time::sleep(TICK).await;
                if self.stopped(&chat_id) {
                    interrupted = true;
                    if !said.is_empty() {
                        self.emit(&chat_id, json!({ "type": "text", "itemId": item, "text": said }));
                    }
                    break 'steps;
                }
                said.push_str(piece);
                self.emit(&chat_id, json!({ "type": "textDelta", "itemId": item, "text": piece }));
            }
            self.emit(&chat_id, json!({ "type": "text", "itemId": item, "text": text }));
        }
        let ms = started.elapsed().as_millis() as u64;
        self.emit(
            &chat_id,
            json!({ "type": "turnFinished", "turnId": turn, "usage": null, "costUsd": null, "durationMs": ms, "interrupted": interrupted }),
        );
        let _ = self.change(&chat_id, |c| {
            c.busy = false;
            c.stop = false;
            c.meta["updatedAt"] = json!(now());
        });
    }
}

/// A few words at a time, the way an answer arrives.
fn pieces(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut words) = (0, 0);
    for (i, c) in text.char_indices() {
        if c == ' ' || c == '\n' {
            words += 1;
            if words == 3 {
                out.push(&text[start..=i]);
                (start, words) = (i + 1, 0);
            }
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// The scripted desktop behind the demo's server.
struct Host {
    identity: Identity,
    /// The one phone it pairs with: this one.
    phone: Mutex<Option<[u8; 32]>>,
    inner: Arc<Inner>,
    runtime: tokio::runtime::Handle,
}

impl Host {
    /// Must be made inside a tokio runtime (its answers stream from tasks on it).
    fn new() -> Host {
        let mut data: Value = serde_json::from_str(DATA).expect("demo.json");
        let started = now();
        let chats = data["chats"]
            .take()
            .as_array_mut()
            .map(std::mem::take)
            .unwrap_or_default()
            .into_iter()
            .map(|mut c| {
                // `ago` (minutes before now) becomes the times a real chat has.
                let at = started.saturating_sub(c["meta"]["ago"].as_u64().unwrap_or(0) * 60_000);
                let mut meta = c["meta"].take();
                if let Some(m) = meta.as_object_mut() {
                    m.remove("ago");
                    m.insert("createdAt".into(), json!(at));
                    m.insert("updatedAt".into(), json!(at));
                }
                let waiting = c["waiting"].as_str().map(String::from);
                let events = c["events"].as_array_mut().map(std::mem::take).unwrap_or_default();
                Chat { meta, events, busy: waiting.is_some(), waiting, stop: false }
            })
            .collect();
        let inner = Inner { data, chats: Mutex::new(chats), tx: broadcast::channel(1024).0, next: AtomicU64::new(1) };
        Host { identity: Identity::generate(), phone: Mutex::new(None), inner: Arc::new(inner), runtime: tokio::runtime::Handle::current() }
    }

    /// Starts the rest of a turn on the runtime.
    fn play(&self, chat_id: &str, turn: String, steps: Vec<Value>) {
        self.runtime.spawn(self.inner.clone().play(chat_id.to_string(), turn, steps));
    }

    /// The demo's steps named `key` (a list of events in demo.json).
    fn steps(&self, key: &str) -> Vec<Value> {
        self.inner.data[key].as_array().cloned().unwrap_or_default()
    }

    fn create(&self, p: &Value) -> Result<Value, String> {
        let inner = &self.inner;
        let project = inner.data["projects"]
            .as_array()
            .and_then(|all| all.iter().find(|x| x["id"] == p["projectId"]))
            .ok_or("That project isn't in the demo.")?;
        let agent = [&p["agent"], &project["defaultAgent"], &inner.data["agents"]["defaultAgent"]]
            .into_iter()
            .find_map(|a| a.as_str())
            .unwrap_or("claude")
            .to_string();
        let default_model = inner.data["agents"]["agents"]
            .as_array()
            .and_then(|all| all.iter().find(|a| a["kind"] == agent.as_str()))
            .map(|a| a["defaultModel"].clone())
            .unwrap_or(json!(""));
        let model = if p["model"].is_string() { p["model"].clone() } else { default_model };
        let at = now();
        let meta = json!({
            "id": inner.id("c-new-"), "projectId": project["id"], "agent": agent, "effort": p["effort"].as_str().unwrap_or(""),
            "model": model, "title": "", "cwd": project["path"], "agentSessionId": null, "createdAt": at, "updatedAt": at,
            "tokens": 0, "costUsd": 0.0, "pinned": false,
        });
        let started = json!({ "type": "started", "sessionId": null, "model": model, "cwd": project["path"] });
        inner.chats.lock().unwrap().push(Chat { meta: meta.clone(), events: vec![started], waiting: None, busy: false, stop: false });
        let _ = inner.tx.send(Feed::ChatsChanged);
        Ok(meta)
    }

    fn send(&self, chat_id: &str, p: &Value) -> Result<Value, String> {
        let text = p["prompt"]["text"].as_str().unwrap_or_default().to_string();
        let images: Vec<&Value> = p["prompt"]["images"].as_array().map(|a| a.iter().map(|i| &i["mime"]).collect()).unwrap_or_default();
        let mut was_busy = false;
        self.inner.change(chat_id, |c| {
            was_busy = c.busy;
            c.busy = true;
            c.meta["updatedAt"] = json!(now());
            if c.meta["title"] == "" {
                c.meta["title"] = json!(text.chars().take(48).collect::<String>());
            }
        })?;
        let id = p["prompt"]["id"].as_str().map(String::from).unwrap_or_else(|| self.inner.id("u"));
        self.inner.emit(chat_id, json!({ "type": "userMessage", "id": id, "text": text, "images": images, "files": [] }));
        // A message sent into a turn that's under way just joins it.
        if !was_busy {
            let turn = self.inner.id("t");
            self.inner.emit(chat_id, json!({ "type": "turnStarted", "turnId": turn }));
            let reply = json!({ "type": "text", "itemId": self.inner.id("a"), "text": self.inner.data["reply"] });
            self.play(chat_id, turn, vec![reply]);
        }
        Ok(Value::Null)
    }

    /// An answer to what the chat was waiting on: the request is resolved and the turn goes on.
    fn resolve(&self, chat_id: &str, kind: &str, resolved: Value, steps: Vec<Value>) -> Result<Value, String> {
        let mut waiting = false;
        self.inner.change(chat_id, |c| {
            waiting = c.waiting.as_deref() == Some(kind);
            if waiting {
                c.waiting = None;
            }
        })?;
        if !waiting {
            return Err("That was already answered.".into());
        }
        self.inner.emit(chat_id, resolved);
        self.play(chat_id, self.inner.id("t"), steps);
        Ok(Value::Null)
    }
}

impl oah_remote::server::Host for Host {
    fn identity(&self) -> Identity {
        self.identity.clone()
    }

    fn desktop_name(&self) -> String {
        self.inner.data["desktopName"].as_str().unwrap_or("Demo computer").to_string()
    }

    fn device(&self, key: &[u8; 32]) -> Option<String> {
        (self.phone.lock().unwrap().as_ref() == Some(key)).then(|| "demo".to_string())
    }

    fn pair(&self, _name: &str, key: &[u8; 32]) -> Result<String, String> {
        *self.phone.lock().unwrap() = Some(*key);
        Ok("demo".into())
    }

    fn call(&self, _device: &str, method: &str, p: Value) -> Result<Value, String> {
        let inner = &self.inner;
        let chat = p["chatId"].as_str().unwrap_or_default();
        match method {
            "projects.list" => Ok(inner.data["projects"].clone()),
            "agents.list" => Ok(inner.data["agents"].clone()),
            "chats.list" => Ok(inner.list()),
            "provider.catalog" => {
                let models = &inner.data["models"][p["kind"].as_str().unwrap_or_default()];
                Ok(json!({ "models": if models.is_array() { models.clone() } else { json!([]) }, "commands": [] }))
            }
            "chat.create" => self.create(&p),
            "chat.send" => self.send(chat, &p),
            "chat.interrupt" => inner.change(chat, |c| c.stop = c.busy && c.waiting.is_none()).map(|_| Value::Null),
            "chat.answer" => {
                let decision = p["decision"].as_str().unwrap_or("deny");
                let resolved = json!({ "type": "approvalResolved", "requestId": p["requestId"], "decision": decision });
                self.resolve(chat, "approval", resolved, self.steps(if decision == "deny" { "afterDeny" } else { "afterAllow" }))
            }
            "chat.answerQuestion" => {
                let resolved = json!({ "type": "questionResolved", "requestId": p["requestId"], "answers": p["answers"] });
                let reply = json!({ "type": "text", "itemId": inner.id("a"), "text": inner.data["afterAnswer"] });
                self.resolve(chat, "question", resolved, vec![reply])
            }
            "chat.setModel" => inner.change(chat, |c| c.meta["model"] = p["model"].clone()),
            "chat.setEffort" => inner.change(chat, |c| c.meta["effort"] = p["effort"].clone()),
            "chat.rename" => inner.change(chat, |c| c.meta["title"] = p["title"].clone()),
            "chat.pin" => inner.change(chat, |c| c.meta["pinned"] = json!(p["pinned"] == true)),
            "chat.setPermission" => {
                inner.change(chat, |c| c.meta["permission"] = p["mode"].clone())?;
                inner.emit(chat, json!({ "type": "permissionChanged", "mode": p["mode"] }));
                Ok(Value::Null)
            }
            "chat.delete" => {
                inner.chats.lock().unwrap().retain(|c| c.meta["id"] != chat);
                let _ = inner.tx.send(Feed::ChatsChanged);
                Ok(Value::Null)
            }
            "chat.clear" | "chat.compact" => {
                inner.emit(chat, json!({ "type": "notice", "level": "info", "text": "The demo keeps its conversation as it is." }));
                Ok(Value::Null)
            }
            "chat.sleep" => Ok(Value::Null),
            "chat.endCli" => Ok(json!(false)),
            "chat.image" | "chat.readImage" => Err("The demo has no images to load.".into()),
            _ => Err(format!("The demo doesn't do {method}.")),
        }
    }

    fn open_chat(&self, chat_id: &str) -> Result<(Value, u64), String> {
        let chats = self.inner.chats.lock().unwrap();
        let c = chats.iter().find(|c| c.meta["id"] == chat_id).ok_or("That chat isn't in the demo anymore.")?;
        let seq = c.events.len() as u64;
        Ok((json!({ "events": c.events, "seq": seq, "live": c.busy, "busy": c.busy }), seq))
    }

    fn feed(&self) -> broadcast::Receiver<Feed> {
        self.inner.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oah_remote::client::Client;
    use tokio::sync::mpsc;

    /// The demo, and this phone connected to it; chat pushes arrive on the receiver.
    async fn connected() -> (Demo, Client, mpsc::UnboundedReceiver<Value>) {
        let me = Identity::generate();
        let demo = start(&me).await.unwrap();
        assert!(demo.desktop.demo);
        let (tx, rx) = mpsc::unbounded_channel();
        let key = oah_remote::key_from_b64(&demo.desktop.id).unwrap();
        let client = Client::connect(&demo.desktop.addresses, &me, &key, move |push| {
            if push["push"] == "chat" {
                let _ = tx.send(push);
            }
        })
        .await
        .unwrap();
        (demo, client, rx)
    }

    /// A chat's pushes up to the end of the turn: their event types, numbered without a gap
    /// from `after`.
    async fn turn(rx: &mut mpsc::UnboundedReceiver<Value>, chat: &str, after: u64) -> Vec<Value> {
        let mut events = Vec::new();
        loop {
            let push = tokio::time::timeout(Duration::from_secs(20), rx.recv()).await.expect("the turn ended").unwrap();
            assert_eq!(push["chatId"], chat);
            assert_eq!(push["seq"], after + events.len() as u64 + 1, "events are numbered in order");
            events.push(push["event"].clone());
            if push["event"]["type"] == "turnFinished" {
                return events;
            }
        }
    }

    fn types(events: &[Value]) -> Vec<&str> {
        let mut out: Vec<&str> = events.iter().filter_map(|e| e["type"].as_str()).collect();
        out.dedup();
        out
    }

    #[test]
    fn splits_an_answer_without_losing_any_of_it() {
        let text = "One two three four five\n\n- six seven\n";
        assert_eq!(pieces(text).concat(), text);
        assert!(pieces(text).len() > 2);
        assert_eq!(pieces(""), Vec::<&str>::new());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lists_what_a_desktop_would() {
        let (_demo, c, _rx) = connected().await;
        assert_eq!(c.desktop_name, "Demo computer");
        assert_eq!(c.call("projects.list", Value::Null).await.unwrap().as_array().unwrap().len(), 2);
        assert_eq!(c.call("agents.list", Value::Null).await.unwrap()["defaultAgent"], "claude");
        let list = c.call("chats.list", Value::Null).await.unwrap();
        assert_eq!(list["metas"].as_array().unwrap().len(), 4);
        assert!(list["metas"][0]["updatedAt"].as_u64().unwrap() > 0 && list["metas"][0].get("ago").is_none());
        assert_eq!(list["waiting"], json!({ "c-checkout": "approval", "c-dates": "question" }));
        assert_eq!(list["busy"], json!(["c-checkout", "c-dates"]));
        let models = c.call("provider.catalog", json!({ "kind": "claude" })).await.unwrap();
        assert_eq!(models["models"][0]["id"], "sonnet");
        assert_eq!(c.call("provider.catalog", json!({ "kind": "grok" })).await.unwrap()["models"], json!([]));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_approval_is_answered_and_the_turn_goes_on() {
        let (_demo, c, mut rx) = connected().await;
        let snap = c.call("chat.open", json!({ "chatId": "c-checkout" })).await.unwrap();
        let seq = snap["seq"].as_u64().unwrap();
        assert_eq!(snap["events"].as_array().unwrap().len() as u64, seq);
        assert_eq!(snap["events"][seq as usize - 1]["type"], "approval");
        assert_eq!(snap["busy"], true);

        c.call("chat.answer", json!({ "chatId": "c-checkout", "requestId": "q1", "decision": "allow" })).await.unwrap();
        let events = turn(&mut rx, "c-checkout", seq).await;
        assert_eq!(types(&events), ["approvalResolved", "toolStarted", "toolFinished", "textDelta", "text", "turnFinished"]);
        assert_eq!(events.last().unwrap()["interrupted"], false);
        let said: String = events.iter().filter(|e| e["type"] == "textDelta").filter_map(|e| e["text"].as_str()).collect();
        assert_eq!(json!(said), events[events.len() - 2]["text"], "the pieces add up to the answer");

        let list = c.call("chats.list", Value::Null).await.unwrap();
        assert_eq!(list["waiting"], json!({ "c-dates": "question" }));
        assert!(c.call("chat.answer", json!({ "chatId": "c-checkout", "requestId": "q1", "decision": "allow" })).await.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_new_chat_answers_and_stops_when_told() {
        let (_demo, c, mut rx) = connected().await;
        let meta = c.call("chat.create", json!({ "projectId": "p-billing" })).await.unwrap();
        let id = meta["id"].as_str().unwrap().to_string();
        assert_eq!((meta["agent"].as_str(), meta["model"].as_str()), (Some("codex"), Some("codex")));
        let snap = c.call("chat.open", json!({ "chatId": id })).await.unwrap();
        assert_eq!((snap["seq"].as_u64(), snap["busy"].as_bool()), (Some(1), Some(false)));

        let prompt = json!({ "id": "m1", "text": "Is this thing on?", "images": [] });
        c.call("chat.send", json!({ "chatId": id, "prompt": prompt })).await.unwrap();
        let events = turn(&mut rx, &id, 1).await;
        assert_eq!(types(&events), ["userMessage", "turnStarted", "textDelta", "text", "turnFinished"]);
        assert_eq!(events[0]["id"], "m1");
        assert!(events[events.len() - 2]["text"].as_str().unwrap().starts_with("This is oah's demo"));
        let list = c.call("chats.list", Value::Null).await.unwrap();
        let titled = list["metas"].as_array().unwrap().iter().find(|m| m["id"] == id.as_str()).unwrap();
        assert_eq!(titled["title"], "Is this thing on?");
        assert_eq!(list["busy"], json!(["c-checkout", "c-dates"]), "the turn is over");

        // Stop, partway through the next answer.
        let seq = 1 + events.len() as u64;
        c.call("chat.send", json!({ "chatId": id, "prompt": { "id": "m2", "text": "And again", "images": [] } })).await.unwrap();
        tokio::time::sleep(TICK * 6).await;
        c.call("chat.interrupt", json!({ "chatId": id })).await.unwrap();
        let events = turn(&mut rx, &id, seq).await;
        assert_eq!(events.last().unwrap()["interrupted"], true);
        let full = events.iter().rfind(|e| e["type"] == "text").unwrap()["text"].as_str().unwrap();
        assert!(!full.is_empty() && full.len() < 200, "what was said so far is kept: {full}");
    }
}
