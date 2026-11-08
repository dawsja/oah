//! The desktop's half: a WebSocket server that pairs phones and serves their sessions, calling
//! a [`Host`] (the app) for everything it knows.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, Notify};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request as HttpRequest, Response as HttpResponse};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use crate::pairing::{self, DesktopHello, Dir, PhoneHello};
use crate::proto::{self, Request, Response};
use crate::wire::{self, Transport};
use crate::Identity;

/// Time to finish the WebSocket upgrade and the first message.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Time for a whole pairing.
const PAIR_TIMEOUT: Duration = Duration::from_secs(30);
const PING_EVERY: Duration = Duration::from_secs(20);
/// A session with nothing from the phone for this long (not even a pong) is gone.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Connections still in their handshake, at most.
const MAX_UNAUTHENTICATED: usize = 16;

/// What the app feeds every session: chat events (numbered per chat, so a session can line them
/// up with a snapshot) and "the chat list changed".
#[derive(Debug, Clone)]
pub enum Feed {
    Chat { chat_id: String, seq: u64, event: Arc<Value> },
    ChatsChanged,
}

/// Something the app may want to show (the Settings page, a toast).
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    PairRequested {
        name: String,
    },
    Paired {
        device_id: String,
        name: String,
    },
    /// A wrong code was tried; `left` more tries before the code closes.
    PairFailed {
        left: u32,
    },
    Connected {
        device_id: String,
    },
    Disconnected {
        device_id: String,
    },
}

/// The app, as the server sees it. Calls that may block (disk, starting an agent) run on
/// tokio's blocking pool.
pub trait Host: Send + Sync + 'static {
    /// The desktop's long-term key.
    fn identity(&self) -> Identity;
    /// What the phone shows for this desktop.
    fn desktop_name(&self) -> String;
    /// The id of the paired device with this key, if there is one.
    fn device(&self, public_key: &[u8; 32]) -> Option<String>;
    /// Saves a newly paired device; returns its id.
    fn pair(&self, name: &str, public_key: &[u8; 32]) -> Result<String, String>;
    /// Answers one of [`proto::HOST_METHODS`].
    fn call(&self, device_id: &str, method: &str, params: Value) -> Result<Value, String>;
    /// A chat as it is now (the phone replays it), and the `seq` of the last event in it.
    fn open_chat(&self, chat_id: &str) -> Result<(Value, u64), String>;
    /// A new receiver of the app's feed.
    fn feed(&self) -> broadcast::Receiver<Feed>;
    fn event(&self, _e: Event) {}
}

struct Conn {
    device_id: String,
    kick: Arc<Notify>,
}

struct Inner {
    host: Arc<dyn Host>,
    pairing: Mutex<Option<pairing::Session>>,
    last_invite: Mutex<Option<std::time::Instant>>,
    conns: Mutex<HashMap<u64, Conn>>,
    unauthenticated: AtomicUsize,
    next_conn: AtomicU64,
}

/// A running server; dropping it (or [`Server::stop`]) stops it and closes every session.
pub struct Server {
    inner: Arc<Inner>,
    addr: SocketAddr,
    accept: tokio::task::JoinHandle<()>,
}

/// The code the desktop shows, and when it stops working (ms since the epoch).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingCode {
    pub code: String,
    pub expires_at: u64,
}

impl Server {
    /// Listens on `addr` (port 0: any free one). Must run inside a tokio runtime.
    pub async fn bind(addr: SocketAddr, host: Arc<dyn Host>) -> std::io::Result<Server> {
        let listener = TcpListener::bind(addr).await?;
        let addr = listener.local_addr()?;
        let inner = Arc::new(Inner {
            host,
            pairing: Mutex::new(None),
            last_invite: Mutex::new(None),
            conns: Mutex::new(HashMap::new()),
            unauthenticated: AtomicUsize::new(0),
            next_conn: AtomicU64::new(1),
        });
        let i = inner.clone();
        let accept = tokio::spawn(async move {
            loop {
                let Ok((stream, _peer)) = listener.accept().await else {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                };
                if i.unauthenticated.load(Ordering::SeqCst) >= MAX_UNAUTHENTICATED {
                    drop(stream);
                    continue;
                }
                let _ = stream.set_nodelay(true);
                tokio::spawn(serve(i.clone(), stream));
            }
        });
        Ok(Server { inner, addr, accept })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Opens a new pairing code (closing any earlier one).
    pub fn open_pairing(&self) -> PairingCode {
        let s = pairing::Session::new();
        let code = PairingCode { code: s.code.clone(), expires_at: s.expires_at };
        *self.inner.pairing.lock().unwrap() = Some(s);
        code
    }

    pub fn cancel_pairing(&self) {
        *self.inner.pairing.lock().unwrap() = None;
    }

    /// The code that's open, if one is and it still works.
    pub fn pairing(&self) -> Option<PairingCode> {
        self.inner
            .pairing
            .lock()
            .unwrap()
            .as_ref()
            .filter(|s| s.open())
            .map(|s| PairingCode { code: s.code.clone(), expires_at: s.expires_at })
    }

    /// Closes the sessions of a device (it was revoked).
    pub fn kick(&self, device_id: &str) {
        for c in self.inner.conns.lock().unwrap().values().filter(|c| c.device_id == device_id) {
            c.kick.notify_one();
        }
    }

    /// Devices with a session open now (one entry per device).
    pub fn connected(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.inner.conns.lock().unwrap().values().map(|c| c.device_id.clone()).collect();
        ids.sort();
        ids.dedup();
        ids
    }

    pub fn stop(self) {}
}

impl Drop for Server {
    fn drop(&mut self) {
        self.accept.abort();
        for c in self.inner.conns.lock().unwrap().values() {
            c.kick.notify_one();
        }
        *self.inner.pairing.lock().unwrap() = None;
    }
}

/// Counts a connection as unauthenticated until dropped.
struct Pending(Arc<Inner>);
impl Drop for Pending {
    fn drop(&mut self) {
        self.0.unauthenticated.fetch_sub(1, Ordering::SeqCst);
    }
}

type Ws = WebSocketStream<TcpStream>;

// tungstenite's handshake callback returns its own (large) error type.
#[allow(clippy::result_large_err)]
async fn serve(inner: Arc<Inner>, stream: TcpStream) {
    inner.unauthenticated.fetch_add(1, Ordering::SeqCst);
    let pending = Pending(inner.clone());
    let config = WebSocketConfig::default().max_message_size(Some(wire::MAX_MESSAGE)).max_frame_size(Some(1 << 20));
    let check_path = |req: &HttpRequest, resp: HttpResponse| -> Result<HttpResponse, ErrorResponse> {
        if req.uri().path() == crate::PATH {
            Ok(resp)
        } else {
            let mut e = ErrorResponse::new(Some("not found".into()));
            *e.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::NOT_FOUND;
            Err(e)
        }
    };
    let first = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        let mut ws = tokio_tungstenite::accept_hdr_async_with_config(stream, check_path, Some(config)).await.ok()?;
        let first = wire::recv(&mut ws).await.ok()?;
        Some((ws, first))
    })
    .await;
    let Ok(Some((mut ws, (kind, body)))) = first else { return };
    match kind {
        wire::INVITE => {
            let name = String::from_utf8(body).ok().filter(|n| !n.trim().is_empty() && n.len() <= 128 && !n.chars().any(char::is_control));
            let allowed = {
                let mut last = inner.last_invite.lock().unwrap();
                if name.is_some() && last.is_none_or(|t| t.elapsed() >= Duration::from_secs(30)) {
                    *last = Some(std::time::Instant::now());
                    true
                } else {
                    false
                }
            };
            if allowed {
                inner.host.event(Event::PairRequested { name: name.unwrap() });
                let _ = wire::send(&mut ws, wire::INVITE, b"ok").await;
            } else {
                wire::fail(&mut ws, "A pairing request was sent recently. Check the desktop, or wait 30 seconds and retry.").await;
            }
            let _ = ws.close(None).await;
        }
        wire::PAIR => {
            let _ = tokio::time::timeout(PAIR_TIMEOUT, pair(&inner, &mut ws, body)).await;
            let _ = ws.close(None).await;
        }
        wire::HANDSHAKE => session(inner, ws, body, pending).await,
        _ => wire::fail(&mut ws, "unexpected message").await,
    }
}

async fn pair(inner: &Arc<Inner>, ws: &mut Ws, spake_phone: Vec<u8>) {
    // Every try counts, before it can be checked, so parallel guesses can't get more than five.
    let code = inner.pairing.lock().unwrap().as_mut().filter(|s| s.open()).map(|s| {
        s.attempts += 1;
        s.code.clone()
    });
    let Some(code) = code else {
        return wire::fail(ws, "No pairing code is open on this desktop. In oah there, open Settings → Phone and choose Pair a phone.")
            .await;
    };
    let (state, spake_desktop) = pairing::start(&code);
    if wire::send(ws, wire::PAIR, &spake_desktop).await.is_err() {
        return;
    }
    let Ok(key) = pairing::finish(state, &spake_phone) else { return wire::fail(ws, "The pairing message didn't read.").await };
    let Ok((wire::PAIR, sealed)) = wire::recv(ws).await else { return };
    let hello = pairing::open(&key, Dir::Phone, &sealed).and_then(|b| serde_json::from_slice::<PhoneHello>(&b).ok());
    let Some(hello) = hello else {
        let left = {
            let mut p = inner.pairing.lock().unwrap();
            let left = p.as_ref().filter(|s| s.code == code).map(|s| pairing::MAX_ATTEMPTS.saturating_sub(s.attempts)).unwrap_or(0);
            if left == 0 {
                *p = None;
            }
            left
        };
        inner.host.event(Event::PairFailed { left });
        let why =
            if left == 0 { "That code isn't right, and it's closed now. Make a new one on the desktop." } else { "That code isn't right." };
        return wire::fail(ws, why).await;
    };
    let Some(phone_key) = crate::key_from_b64(&hello.public_key) else { return wire::fail(ws, "The phone's key didn't read.").await };
    // The code is used up now, whatever happens next.
    let taken = {
        let mut p = inner.pairing.lock().unwrap();
        let ours = p.as_ref().is_some_and(|s| s.code == code);
        if ours {
            *p = None;
        }
        ours
    };
    if !taken {
        return wire::fail(ws, "That code was closed on the desktop. Make a new one there.").await;
    }
    let host = inner.host.clone();
    let name = hello.name.trim().chars().take(60).collect::<String>();
    let name = if name.is_empty() { "Phone".to_string() } else { name };
    let n = name.clone();
    let device_id = match tokio::task::spawn_blocking(move || host.pair(&n, &phone_key)).await {
        Ok(Ok(id)) => id,
        Ok(Err(e)) => return wire::fail(ws, &e).await,
        Err(_) => return wire::fail(ws, "The desktop couldn't save the phone.").await,
    };
    let reply = DesktopHello {
        device_id: device_id.clone(),
        desktop_name: inner.host.desktop_name(),
        public_key: crate::b64(&inner.host.identity().public),
        protocol: crate::PROTOCOL,
    };
    if wire::send(ws, wire::PAIR, &pairing::seal(&key, Dir::Desktop, &serde_json::to_vec(&reply).unwrap())).await.is_ok() {
        inner.host.event(Event::Paired { device_id, name });
    }
}

/// A chat a session opened: its events go to the phone from just after `seq` (until the snapshot
/// is in, they wait in `held`).
struct Sub {
    seq: Option<u64>,
    held: Vec<(u64, Arc<Value>)>,
}

async fn session(inner: Arc<Inner>, mut ws: Ws, msg1: Vec<u8>, pending: Pending) {
    let identity = inner.host.identity();
    let Ok(mut hs) = wire::responder(&identity) else { return };
    let Ok(_payload) = wire::hs_read(&mut hs, &msg1) else {
        return wire::fail(&mut ws, "This phone's pairing is for a different desktop key. Pair it again.").await;
    };
    let Some(phone_key) = hs.get_remote_static().and_then(|k| <[u8; 32]>::try_from(k).ok()) else { return };
    let Some(device_id) = inner.host.device(&phone_key) else {
        return wire::fail(&mut ws, "This phone isn't paired with this desktop anymore. Pair it again.").await;
    };
    let hello = json!({ "desktopName": inner.host.desktop_name(), "protocol": crate::PROTOCOL });
    let Ok(msg2) = wire::hs_write(&mut hs, hello.to_string().as_bytes()) else { return };
    if wire::send(&mut ws, wire::HANDSHAKE, &msg2).await.is_err() {
        return;
    }
    let Ok(transport) = Transport::new(hs) else { return };
    let transport = Arc::new(transport);
    drop(pending);

    let conn_id = inner.next_conn.fetch_add(1, Ordering::SeqCst);
    let kick = Arc::new(Notify::new());
    inner.conns.lock().unwrap().insert(conn_id, Conn { device_id: device_id.clone(), kick: kick.clone() });
    inner.host.event(Event::Connected { device_id: device_id.clone() });

    let (mut sink, mut stream) = ws.split();
    let (out, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let t = transport.clone();
    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(PING_EVERY);
        ping.tick().await;
        loop {
            tokio::select! {
                msg = out_rx.recv() => {
                    let Some(msg) = msg else { break };
                    for f in t.seal(&msg) {
                        if sink.send(Message::Binary(f.into())).await.is_err() {
                            return;
                        }
                    }
                }
                _ = ping.tick() => {
                    if sink.send(Message::Ping(Default::default())).await.is_err() {
                        return;
                    }
                }
            }
        }
        let _ = sink.close().await;
    });

    let subs: Arc<Mutex<HashMap<String, Sub>>> = Arc::new(Mutex::new(HashMap::new()));
    let feeder = {
        let (subs, out, mut feed) = (subs.clone(), out.clone(), inner.host.feed());
        tokio::spawn(async move {
            loop {
                match feed.recv().await {
                    Ok(Feed::Chat { chat_id, seq, event }) => {
                        let mut subs = subs.lock().unwrap();
                        let Some(sub) = subs.get_mut(&chat_id) else { continue };
                        match sub.seq {
                            None => sub.held.push((seq, event)),
                            Some(s) if seq > s => {
                                sub.seq = Some(seq);
                                let _ = out.send(proto::chat_push(&chat_id, &event).to_string().into_bytes());
                            }
                            Some(_) => {}
                        }
                    }
                    Ok(Feed::ChatsChanged) => {
                        let _ = out.send(br#"{"push":"chats"}"#.to_vec());
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // Events were dropped: the phone opens its chats again.
                        subs.lock().unwrap().clear();
                        let _ = out.send(br#"{"push":"resync"}"#.to_vec());
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    };

    loop {
        let frame = tokio::select! {
            _ = kick.notified() => break,
            f = tokio::time::timeout(IDLE_TIMEOUT, stream.next()) => f,
        };
        let msg = match frame {
            Err(_) | Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(Message::Close(_)))) => break,
            Ok(Some(Ok(Message::Binary(b)))) => b,
            Ok(Some(Ok(_))) => continue,
        };
        let Some((&wire::DATA, body)) = msg.split_first() else { break };
        let whole = match transport.open(body) {
            Ok(Some(m)) => m,
            Ok(None) => continue,
            Err(_) => break,
        };
        let Ok(req) = serde_json::from_slice::<Request>(&whole) else { break };
        handle(&inner, &device_id, req, &out, &subs);
    }

    feeder.abort();
    drop(out);
    let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;
    inner.conns.lock().unwrap().remove(&conn_id);
    inner.host.event(Event::Disconnected { device_id });
}

fn respond(out: &mpsc::UnboundedSender<Vec<u8>>, id: u64, r: Result<Value, String>) {
    let _ = out.send(serde_json::to_vec(&Response::from(id, r)).unwrap());
}

fn chat_id(params: &Value) -> Result<String, String> {
    params.get("chatId").and_then(Value::as_str).map(String::from).ok_or_else(|| "chatId is missing".to_string())
}

fn handle(
    inner: &Arc<Inner>,
    device_id: &str,
    req: Request,
    out: &mpsc::UnboundedSender<Vec<u8>>,
    subs: &Arc<Mutex<HashMap<String, Sub>>>,
) {
    let Request { id, method, params } = req;
    match method.as_str() {
        proto::HELLO => {
            respond(out, id, Ok(json!({ "desktopName": inner.host.desktop_name(), "protocol": crate::PROTOCOL, "deviceId": device_id })))
        }
        proto::CHAT_OPEN => {
            let chat = match chat_id(&params) {
                Ok(c) => c,
                Err(e) => return respond(out, id, Err(e)),
            };
            subs.lock().unwrap().insert(chat.clone(), Sub { seq: None, held: vec![] });
            let (host, out, subs) = (inner.host.clone(), out.clone(), subs.clone());
            tokio::spawn(async move {
                let c = chat.clone();
                let r = tokio::task::spawn_blocking(move || host.open_chat(&c)).await.unwrap_or_else(|_| Err("the desktop failed".into()));
                // The answer, then whatever came in meanwhile, in order, under the lock the feed
                // takes too.
                let mut subs = subs.lock().unwrap();
                match r {
                    Ok((snapshot, seq)) => {
                        respond(&out, id, Ok(snapshot));
                        if let Some(sub) = subs.get_mut(&chat) {
                            let mut last = seq;
                            for (s, e) in std::mem::take(&mut sub.held) {
                                if s > last {
                                    last = s;
                                    let _ = out.send(proto::chat_push(&chat, &e).to_string().into_bytes());
                                }
                            }
                            sub.seq = Some(last);
                        }
                    }
                    Err(e) => {
                        subs.remove(&chat);
                        respond(&out, id, Err(e));
                    }
                }
            });
        }
        proto::CHAT_CLOSE => {
            if let Ok(c) = chat_id(&params) {
                subs.lock().unwrap().remove(&c);
            }
            respond(out, id, Ok(Value::Null));
        }
        m if proto::HOST_METHODS.contains(&m) => {
            let (host, out, device) = (inner.host.clone(), out.clone(), device_id.to_string());
            tokio::spawn(async move {
                let r = tokio::task::spawn_blocking(move || host.call(&device, &method, params))
                    .await
                    .unwrap_or_else(|_| Err("the desktop failed".into()));
                respond(&out, id, r);
            });
        }
        m => respond(out, id, Err(format!("unknown method {m}"))),
    }
}
