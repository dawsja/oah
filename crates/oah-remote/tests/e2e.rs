//! A real server on loopback, a mock host, and the real client.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use oah_remote::client::{self, Client};
use oah_remote::server::{Event, Feed, Host, Server};
use oah_remote::Identity;
use serde_json::{json, Value};
use tokio::sync::broadcast;

struct Mock {
    id: Identity,
    devices: Mutex<Vec<(String, [u8; 32])>>,
    events: Mutex<Vec<Event>>,
    tx: broadcast::Sender<Feed>,
    /// chat "c1"'s events so far.
    log: Mutex<Vec<Value>>,
}

impl Mock {
    fn new() -> Arc<Mock> {
        Arc::new(Mock {
            id: Identity::generate(),
            devices: Mutex::new(vec![]),
            events: Mutex::new(vec![]),
            tx: broadcast::channel(256).0,
            log: Mutex::new(vec![]),
        })
    }
    /// Emits a chat event the way the app does: logged and fed under one lock.
    fn emit(&self, event: Value) {
        let mut log = self.log.lock().unwrap();
        log.push(event.clone());
        let _ = self.tx.send(Feed::Chat { chat_id: "c1".into(), seq: log.len() as u64, event: Arc::new(event) });
    }
}

impl Host for Mock {
    fn identity(&self) -> Identity {
        self.id.clone()
    }
    fn desktop_name(&self) -> String {
        "test-pc".into()
    }
    fn device(&self, key: &[u8; 32]) -> Option<String> {
        self.devices.lock().unwrap().iter().find(|(_, k)| k == key).map(|(id, _)| id.clone())
    }
    fn pair(&self, _name: &str, key: &[u8; 32]) -> Result<String, String> {
        let mut d = self.devices.lock().unwrap();
        let id = format!("dev{}", d.len() + 1);
        d.push((id.clone(), *key));
        Ok(id)
    }
    fn call(&self, device: &str, method: &str, params: Value) -> Result<Value, String> {
        match method {
            "chats.list" => Ok(json!({ "metas": [], "running": [], "device": device })),
            "chat.send" => {
                let text = params["prompt"]["text"].as_str().unwrap_or("").to_string();
                if text == "big" {
                    return Ok(json!("x".repeat(200_000)));
                }
                self.emit(json!({ "type": "userMessage", "text": text }));
                Ok(Value::Null)
            }
            _ => Err(format!("mock: {method}")),
        }
    }
    fn open_chat(&self, chat_id: &str) -> Result<(Value, u64), String> {
        if chat_id != "c1" {
            return Err("no such chat".into());
        }
        let log = self.log.lock().unwrap();
        Ok((json!({ "events": *log }), log.len() as u64))
    }
    fn feed(&self) -> broadcast::Receiver<Feed> {
        self.tx.subscribe()
    }
    fn event(&self, e: Event) {
        self.events.lock().unwrap().push(e);
    }
}

async fn start() -> (Arc<Mock>, Server, Vec<String>) {
    let host = Mock::new();
    let server = Server::bind("127.0.0.1:0".parse().unwrap(), host.clone()).await.unwrap();
    let addr = vec![server.local_addr().to_string()];
    (host, server, addr)
}

async fn wait_for(f: impl Fn() -> bool) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out");
}

#[tokio::test(flavor = "multi_thread")]
async fn pairs_by_code_then_connects() {
    let (host, server, addr) = start().await;
    let phone = Identity::generate();

    let e = client::pair(&addr, "AAAA-BBBB", "Pixel", &phone).await.unwrap_err();
    assert!(e.contains("No pairing code"), "{e}");

    let code = server.open_pairing().code;
    // A dead address next to the live one: the live one wins.
    let addrs = vec!["127.0.0.1:1".to_string(), addr[0].clone()];
    let p = client::pair(&addrs, &oah_remote::pairing::format_code(&code).to_lowercase(), "Pixel", &phone).await.unwrap();
    assert_eq!(p.device_id, "dev1");
    assert_eq!(p.desktop_name, "test-pc");
    assert_eq!(p.desktop_key, host.id.public);
    assert_eq!(p.address, addr[0]);
    assert!(server.pairing().is_none(), "the code is used up");
    assert!(client::pair(&addr, &code, "Again", &Identity::generate()).await.is_err());

    let c = Client::connect(&addrs, &phone, &p.desktop_key, |_| {}).await.unwrap();
    assert_eq!(c.desktop_name, "test-pc");
    let hello = c.call("hello", Value::Null).await.unwrap();
    assert_eq!(hello["deviceId"], "dev1");
    assert_eq!(c.call("chats.list", Value::Null).await.unwrap()["device"], "dev1");
    assert!(c.call("settings.set", Value::Null).await.unwrap_err().contains("unknown method"));
    assert!(c.call("chat.rename", Value::Null).await.unwrap_err().contains("mock"), "allowed methods reach the host");
    // Bigger than one Noise message, both ways.
    let big = c.call("chat.send", json!({ "prompt": { "text": "big" } })).await.unwrap();
    assert_eq!(big.as_str().unwrap().len(), 200_000);
    wait_for(|| server.connected() == ["dev1"]).await;

    let ev = host.events.lock().unwrap().clone();
    assert!(ev.contains(&Event::Paired { device_id: "dev1".into(), name: "Pixel".into() }));
    assert!(ev.contains(&Event::Connected { device_id: "dev1".into() }));
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_codes_count_and_close_the_code() {
    let (host, server, addr) = start().await;
    let code = server.open_pairing().code;
    let wrong = format!("{}{}", if code.starts_with('0') { "1" } else { "0" }, &code[1..]);
    for i in 1..=4 {
        let e = client::pair(&addr, &wrong, "x", &Identity::generate()).await.unwrap_err();
        assert!(e.contains("isn't right"), "{e}");
        assert!(host.events.lock().unwrap().contains(&Event::PairFailed { left: 5 - i }));
    }
    let e = client::pair(&addr, &wrong, "x", &Identity::generate()).await.unwrap_err();
    assert!(e.contains("closed"), "{e}");
    // Even the right code is refused now.
    let e = client::pair(&addr, &code, "x", &Identity::generate()).await.unwrap_err();
    assert!(e.contains("No pairing code"), "{e}");
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_and_revoked_phones_are_refused() {
    let (host, server, addr) = start().await;
    let stranger = Identity::generate();
    let e = Client::connect(&addr, &stranger, &host.id.public, |_| {}).await;
    // The handshake completes from the phone's view only if the desktop answers; it refuses.
    assert!(e.is_err());

    // Wrong desktop key: the desktop can't read the handshake.
    let code = server.open_pairing().code;
    let phone = Identity::generate();
    let p = client::pair(&addr, &code, "P", &phone).await.unwrap();
    assert!(Client::connect(&addr, &phone, &Identity::generate().public, |_| {}).await.is_err());

    let c = Client::connect(&addr, &phone, &p.desktop_key, |_| {}).await.unwrap();
    assert!(c.call("hello", Value::Null).await.is_ok());
    host.devices.lock().unwrap().clear();
    server.kick(&p.device_id);
    let why = tokio::time::timeout(Duration::from_secs(5), c.closed()).await.unwrap();
    assert!(!why.is_empty());
    assert!(c.call("hello", Value::Null).await.is_err());
    assert!(Client::connect(&addr, &phone, &p.desktop_key, |_| {}).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_snapshot_then_live_events_without_gaps() {
    let (host, server, addr) = start().await;
    let phone = Identity::generate();
    let code = server.open_pairing().code;
    let p = client::pair(&addr, &code, "P", &phone).await.unwrap();
    let pushes = Arc::new(Mutex::new(Vec::<Value>::new()));
    let pp = pushes.clone();
    let c = Client::connect(&addr, &phone, &p.desktop_key, move |v| pp.lock().unwrap().push(v)).await.unwrap();

    host.emit(json!({ "n": 1 }));
    host.emit(json!({ "n": 2 }));
    // Events keep coming while the phone opens the chat.
    let h = host.clone();
    let spam = tokio::spawn(async move {
        for n in 3..=200 {
            h.emit(json!({ "n": n }));
            if n % 20 == 0 {
                tokio::task::yield_now().await;
            }
        }
    });
    let snap = c.call("chat.open", json!({ "chatId": "c1" })).await.unwrap();
    spam.await.unwrap();
    wait_for(|| {
        let seen = snap["events"].as_array().unwrap().len() + pushes.lock().unwrap().len();
        seen >= 200
    })
    .await;
    let mut ns: Vec<u64> = snap["events"].as_array().unwrap().iter().map(|e| e["n"].as_u64().unwrap()).collect();
    for p in pushes.lock().unwrap().iter() {
        assert_eq!(p["push"], "chat");
        assert_eq!(p["chatId"], "c1");
        ns.push(p["event"]["n"].as_u64().unwrap());
    }
    assert_eq!(ns, (1..=200).collect::<Vec<_>>(), "every event once, in order");

    // Sends that emit reach the phone; a closed chat stops pushing.
    c.call("chat.send", json!({ "chatId": "c1", "prompt": { "text": "hi" } })).await.unwrap();
    wait_for(|| pushes.lock().unwrap().last().is_some_and(|p| p["event"]["text"] == "hi")).await;
    c.call("chat.close", json!({ "chatId": "c1" })).await.unwrap();
    let before = pushes.lock().unwrap().len();
    host.emit(json!({ "n": 999 }));
    let _ = host.tx.send(Feed::ChatsChanged);
    wait_for(|| pushes.lock().unwrap().last().is_some_and(|p| p["push"] == "chats")).await;
    assert_eq!(pushes.lock().unwrap().len(), before + 1);
    assert!(c.call("chat.open", json!({ "chatId": "nope" })).await.is_err());
}
