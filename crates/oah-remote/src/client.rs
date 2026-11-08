//! The phone's half: pair with a desktop, then open sessions and call it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::select_ok;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::pairing::{self, DesktopHello, Dir, PhoneHello};
use crate::proto::{Incoming, Request, Response};
use crate::wire::{self, Transport};
use crate::Identity;

/// Time to reach an address and finish the handshake.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
/// A call the desktop doesn't answer in this long fails (starting an agent can take a while).
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// The desktop pings every 20 s: nothing for this long means the link is gone.
const IDLE_TIMEOUT: Duration = Duration::from_secs(45);

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A desktop this phone paired with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paired {
    pub device_id: String,
    pub desktop_name: String,
    pub desktop_key: [u8; 32],
    /// The address that answered.
    pub address: String,
}

async fn open_ws(address: &str) -> Result<Ws, String> {
    let url = format!("ws://{address}{}", crate::PATH);
    let config = WebSocketConfig::default().max_message_size(Some(wire::MAX_MESSAGE)).max_frame_size(Some(1 << 20));
    let (ws, _) = tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async_with_config(url, Some(config), true))
        .await
        .map_err(|_| format!("{address} didn't answer"))?
        .map_err(|e| format!("couldn't reach {address}: {e}"))?;
    Ok(ws)
}

/// Tries every address at once; the first that works wins.
async fn first<T, F, Fut>(addresses: &[String], f: F) -> Result<(T, String), String>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<T, String>> + Send + 'static,
    T: Send + 'static,
{
    if addresses.is_empty() {
        return Err("no address to connect to".into());
    }
    let tries = addresses.iter().map(|a| {
        let a = a.clone();
        let fut = f(a.clone());
        Box::pin(async move { fut.await.map(|t| (t, a)) })
    });
    match select_ok(tries).await {
        Ok((r, _rest)) => Ok(r),
        Err(e) => Err(if addresses.len() == 1 { e } else { format!("couldn't reach the desktop at {} ({e})", addresses.join(", ")) }),
    }
}

/// Pairs with the desktop showing `code`.
pub async fn pair(addresses: &[String], code: &str, device_name: &str, me: &Identity) -> Result<Paired, String> {
    // Find the address that answers first, then pair over that one only (each try uses one of
    // the code's five).
    let (mut ws, address) = first(addresses, |a| async move { open_ws(&a).await }).await?;
    let (state, spake_phone) = pairing::start(code);
    wire::send(&mut ws, wire::PAIR, &spake_phone).await?;
    let (kind, spake_desktop) = wire::recv(&mut ws).await?;
    if kind != wire::PAIR {
        return Err("the desktop answered something else".into());
    }
    let key = pairing::finish(state, &spake_desktop)?;
    let hello = PhoneHello { name: device_name.to_string(), public_key: crate::b64(&me.public) };
    wire::send(&mut ws, wire::PAIR, &pairing::seal(&key, Dir::Phone, &serde_json::to_vec(&hello).unwrap())).await?;
    let (kind, sealed) = wire::recv(&mut ws).await?;
    let _ = ws.close(None).await;
    if kind != wire::PAIR {
        return Err("the desktop answered something else".into());
    }
    let reply: DesktopHello = pairing::open(&key, Dir::Desktop, &sealed)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or("The desktop's answer didn't check out. Try again with a new code.")?;
    let desktop_key = crate::key_from_b64(&reply.public_key).ok_or("the desktop's key didn't read")?;
    Ok(Paired { device_id: reply.device_id, desktop_name: reply.desktop_name, desktop_key, address })
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Response>>>>;

/// An open session with a desktop.
pub struct Client {
    out: mpsc::UnboundedSender<Vec<u8>>,
    pending: Pending,
    next: AtomicU64,
    closed: watch::Receiver<Option<String>>,
    close_tx: Arc<watch::Sender<Option<String>>>,
    /// The address that answered.
    pub address: String,
    pub desktop_name: String,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

async fn handshake(address: String, me: Identity, desktop: [u8; 32]) -> Result<(Ws, Transport, String), String> {
    let mut ws = open_ws(&address).await?;
    let mut hs = wire::initiator(&me, &desktop)?;
    let msg1 = wire::hs_write(&mut hs, serde_json::json!({ "protocol": crate::PROTOCOL }).to_string().as_bytes())?;
    wire::send(&mut ws, wire::HANDSHAKE, &msg1).await?;
    let (kind, msg2) =
        tokio::time::timeout(CONNECT_TIMEOUT, wire::recv(&mut ws)).await.map_err(|_| format!("{address} didn't answer"))??;
    if kind != wire::HANDSHAKE {
        return Err("the desktop answered something else".into());
    }
    let payload = wire::hs_read(&mut hs, &msg2)?;
    let name =
        serde_json::from_slice::<Value>(&payload).ok().and_then(|v| v.get("desktopName")?.as_str().map(String::from)).unwrap_or_default();
    Ok((ws, Transport::new(hs)?, name))
}

impl Client {
    /// Opens a session, trying every address at once. `on_push` gets every push (chat events,
    /// list changes) in order.
    pub async fn connect(
        addresses: &[String],
        me: &Identity,
        desktop_key: &[u8; 32],
        on_push: impl Fn(Value) + Send + Sync + 'static,
    ) -> Result<Client, String> {
        let (me, key) = (me.clone(), *desktop_key);
        let ((ws, transport, desktop_name), address) = first(addresses, move |a| handshake(a, me.clone(), key)).await?;
        let transport = Arc::new(transport);
        let (mut sink, mut stream) = ws.split();
        let (out, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (closed_tx, closed) = watch::channel(None::<String>);
        let closed_tx = Arc::new(closed_tx);
        let close_tx = closed_tx.clone();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));

        let t = transport.clone();
        let c = closed_tx.clone();
        let writer = tokio::spawn(async move {
            while let Some(msg) = out_rx.recv().await {
                for f in t.seal(&msg) {
                    if sink.send(Message::Binary(f.into())).await.is_err() {
                        let _ = c.send(Some("the connection broke".into()));
                        return;
                    }
                }
            }
            let _ = sink.close().await;
        });

        let p = pending.clone();
        let reader = tokio::spawn(async move {
            let why = loop {
                let msg = match tokio::time::timeout(IDLE_TIMEOUT, stream.next()).await {
                    Err(_) => break "the desktop stopped answering".to_string(),
                    Ok(None) | Ok(Some(Ok(Message::Close(_)))) => break "the desktop closed the connection".to_string(),
                    Ok(Some(Err(e))) => break format!("the connection broke: {e}"),
                    Ok(Some(Ok(Message::Binary(b)))) => b,
                    Ok(Some(Ok(_))) => continue,
                };
                match msg.split_first() {
                    Some((&wire::DATA, body)) => match transport.open(body) {
                        Ok(Some(whole)) => match serde_json::from_slice::<Incoming>(&whole) {
                            Ok(Incoming::Response(r)) => {
                                if let Some(tx) = p.lock().unwrap().remove(&r.id) {
                                    let _ = tx.send(r);
                                }
                            }
                            Ok(Incoming::Push(v)) => on_push(v),
                            Err(_) => {}
                        },
                        Ok(None) => {}
                        Err(e) => break e,
                    },
                    Some((&wire::ERROR, why)) => break String::from_utf8_lossy(why).into_owned(),
                    _ => {}
                }
            };
            p.lock().unwrap().clear();
            let _ = closed_tx.send(Some(why));
        });

        Ok(Client { out, pending, next: AtomicU64::new(1), closed, close_tx, address, desktop_name, tasks: vec![writer, reader] })
    }

    /// Calls a method on the desktop.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        if let Some(why) = self.closed.borrow().clone() {
            return Err(why);
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let req = Request { id, method: method.to_string(), params };
        if self.out.send(serde_json::to_vec(&req).unwrap()).is_err() {
            self.pending.lock().unwrap().remove(&id);
            return Err("not connected".into());
        }
        let closed = self.closed.clone();
        let mut wait = self.closed.clone();
        tokio::select! {
            r = tokio::time::timeout(CALL_TIMEOUT, rx) => match r {
                Ok(Ok(r)) if r.ok => Ok(r.result),
                Ok(Ok(r)) => Err(r.error.unwrap_or_else(|| "the desktop refused".into())),
                Ok(Err(_)) => Err(closed.borrow().clone().unwrap_or_else(|| "the connection closed".into())),
                Err(_) => {
                    self.pending.lock().unwrap().remove(&id);
                    Err("the desktop didn't answer in time".into())
                }
            },
            _ = wait.wait_for(|c| c.is_some()) => {
                self.pending.lock().unwrap().remove(&id);
                Err(closed.borrow().clone().unwrap_or_default())
            }
        }
    }

    /// Waits until the session ends; returns why.
    pub async fn closed(&self) -> String {
        let mut c = self.closed.clone();
        let _ = c.wait_for(|c| c.is_some()).await;
        let why = c.borrow().clone().unwrap_or_default();
        why
    }

    pub fn is_closed(&self) -> bool {
        self.closed.borrow().is_some()
    }

    /// Ends the session now (a link that went quiet while the phone slept).
    pub fn close(&self, why: &str) {
        self.close_tx.send_if_modified(|c| {
            if c.is_none() {
                *c = Some(why.to_string());
                true
            } else {
                false
            }
        });
        for t in &self.tasks {
            t.abort();
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}
