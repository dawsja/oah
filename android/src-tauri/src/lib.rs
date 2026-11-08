//! The oah phone app's Rust side: the phone's key, the desktops it paired with, and one live
//! session at a time (reconnecting on its own). The UI calls the desktop through `rpc` and gets
//! its pushes, and the connection's ups and downs, on the channel it passed to `connect`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use oah_remote::client::{self, Client};
use oah_remote::pairing;
use oah_remote::Identity;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::ipc::Channel;
use tauri::{Manager, State};
use tokio::sync::Notify;

/// The port the desktop listens on unless it was changed there.
const DEFAULT_PORT: u16 = 7787;
/// Waits between reconnect tries; the last repeats.
const BACKOFF: &[u64] = &[500, 1000, 2000, 4000, 8000, 15000, 30000];

/// A desktop this phone paired with.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Desktop {
    /// The desktop's public key (base64): who it is.
    pub id: String,
    pub name: String,
    /// `host:port` to try.
    pub addresses: Vec<String>,
    /// What the desktop calls this phone.
    pub device_id: String,
    pub paired_at: u64,
    /// The address that answered last (tried first).
    #[serde(default)]
    pub last_address: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    /// This phone's private key (base64). The app's data folder is private to the app.
    device_key: String,
    desktops: Vec<Desktop>,
}

struct Live {
    desktop_id: String,
    client: Arc<Mutex<Option<Arc<Client>>>>,
    wake: Arc<Notify>,
    task: tauri::async_runtime::JoinHandle<()>,
}

pub struct AppState {
    path: PathBuf,
    saved: Mutex<Saved>,
    identity: Identity,
    live: Mutex<Option<Live>>,
}

impl AppState {
    fn load(path: PathBuf) -> AppState {
        let mut saved: Saved = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let identity = match Identity::from_private_b64(&saved.device_key) {
            Some(id) => id,
            None => {
                let id = Identity::generate();
                saved.device_key = id.private_b64();
                id
            }
        };
        let state = AppState { path, saved: Mutex::new(saved), identity, live: Mutex::new(None) };
        state.save();
        state
    }

    fn save(&self) {
        let s = self.saved.lock().unwrap();
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec_pretty(&*s).unwrap()).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }

    fn desktop(&self, id: &str) -> Option<Desktop> {
        self.saved.lock().unwrap().desktops.iter().find(|d| d.id == id).cloned()
    }

    fn stop(&self) {
        if let Some(l) = self.live.lock().unwrap().take() {
            l.task.abort();
        }
    }
}

/// `192.168.1.20` → `192.168.1.20:7787`; `host:port` and `[v6]:port` stay as they are.
fn with_port(a: &str) -> String {
    let a = a.trim().trim_start_matches("ws://").trim_start_matches("http://").trim_end_matches('/');
    let has_port = if a.starts_with('[') { a.contains("]:") } else { a.matches(':').count() == 1 };
    if has_port {
        a.to_string()
    } else if a.contains(':') {
        format!("[{a}]:{DEFAULT_PORT}")
    } else {
        format!("{a}:{DEFAULT_PORT}")
    }
}

/// The desktops paired with this phone.
#[tauri::command]
fn desktops(state: State<AppState>) -> Vec<Desktop> {
    state.saved.lock().unwrap().desktops.clone()
}

/// Pairs with a desktop from the `addresses` and `code` it shows.
#[tauri::command]
async fn pair(state: State<'_, AppState>, addresses: Vec<String>, code: String, name: String) -> Result<Desktop, String> {
    let addresses: Vec<String> = addresses.iter().map(|a| a.trim()).filter(|a| !a.is_empty()).map(with_port).collect();
    if addresses.is_empty() {
        return Err("Type the desktop's address (Settings → Phone shows it).".into());
    }
    if pairing::normalize_code(&code).len() != pairing::CODE_LEN {
        return Err("The code has 8 characters.".into());
    }
    let name = if name.trim().is_empty() { "Android phone".to_string() } else { name.trim().to_string() };
    let p = client::pair(&addresses, &code, &name, &state.identity).await?;
    let id = oah_remote::b64(&p.desktop_key);
    // The address that answered goes first; the others stay for other networks.
    let mut all = vec![p.address.clone()];
    all.extend(addresses.into_iter().filter(|a| *a != p.address));
    let desktop = Desktop {
        id: id.clone(),
        name: p.desktop_name,
        addresses: all,
        device_id: p.device_id,
        paired_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
        last_address: Some(p.address),
    };
    {
        let mut s = state.saved.lock().unwrap();
        s.desktops.retain(|d| d.id != id);
        s.desktops.insert(0, desktop.clone());
    }
    state.save();
    Ok(desktop)
}

/// Forgets a desktop (it still lists this phone until it's removed there too).
#[tauri::command]
fn forget(state: State<AppState>, id: String) {
    if state.live.lock().unwrap().as_ref().is_some_and(|l| l.desktop_id == id) {
        state.stop();
    }
    state.saved.lock().unwrap().desktops.retain(|d| d.id != id);
    state.save();
}

/// Keeps a session with desktop `id` open, reconnecting when it drops, until `disconnect` or
/// another `connect`. `on_event` gets `{kind: "state", state, error?, address?}` and
/// `{kind: "push", push}`.
#[tauri::command]
fn connect(app: tauri::AppHandle, state: State<AppState>, id: String, on_event: Channel<Value>) -> Result<(), String> {
    let desktop = state.desktop(&id).ok_or("That desktop isn't paired with this phone.")?;
    state.stop();
    let client: Arc<Mutex<Option<Arc<Client>>>> = Arc::new(Mutex::new(None));
    let wake = Arc::new(Notify::new());
    let (c, w, identity) = (client.clone(), wake.clone(), state.identity.clone());
    let task = tauri::async_runtime::spawn(async move {
        let key = oah_remote::key_from_b64(&desktop.id).unwrap_or_default();
        let mut attempt = 0usize;
        loop {
            let _ = on_event.send(json!({ "kind": "state", "state": "connecting" }));
            let mut addresses =
                app.state::<AppState>().desktop(&desktop.id).map(|d| d.addresses).unwrap_or_else(|| desktop.addresses.clone());
            if let Some(last) = app.state::<AppState>().desktop(&desktop.id).and_then(|d| d.last_address) {
                addresses.retain(|a| *a != last);
                addresses.insert(0, last);
            }
            let events = on_event.clone();
            let why = match Client::connect(&addresses, &identity, &key, move |push| {
                let _ = events.send(json!({ "kind": "push", "push": push }));
            })
            .await
            {
                Ok(session) => {
                    attempt = 0;
                    let session = Arc::new(session);
                    remember_address(&app, &desktop.id, &session.address);
                    *c.lock().unwrap() = Some(session.clone());
                    let _ = on_event.send(
                        json!({ "kind": "state", "state": "connected", "address": session.address, "desktopName": session.desktop_name }),
                    );
                    let why = session.closed().await;
                    *c.lock().unwrap() = None;
                    why
                }
                Err(e) => e,
            };
            // The desktop forgot this phone: trying again won't help.
            if why.contains("isn't paired") || why.contains("different desktop key") {
                let _ = on_event.send(json!({ "kind": "state", "state": "revoked", "error": why }));
                return;
            }
            let _ = on_event.send(json!({ "kind": "state", "state": "offline", "error": why }));
            let delay = BACKOFF[attempt.min(BACKOFF.len() - 1)];
            attempt += 1;
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(delay)) => {}
                _ = w.notified() => attempt = 0,
            }
        }
    });
    *state.live.lock().unwrap() = Some(Live { desktop_id: id, client, wake, task });
    Ok(())
}

fn remember_address(app: &tauri::AppHandle, id: &str, address: &str) {
    let state = app.state::<AppState>();
    let changed = {
        let mut s = state.saved.lock().unwrap();
        match s.desktops.iter_mut().find(|d| d.id == id) {
            Some(d) if d.last_address.as_deref() != Some(address) => {
                d.last_address = Some(address.to_string());
                true
            }
            _ => false,
        }
    };
    if changed {
        state.save();
    }
}

#[tauri::command]
fn disconnect(state: State<AppState>) {
    state.stop();
}

/// The app came back to the foreground: a session that doesn't answer within a few seconds
/// is dropped and opened again, and a wait between tries ends now.
#[tauri::command]
async fn wake(state: State<'_, AppState>) -> Result<(), String> {
    let (client, wake) = match state.live.lock().unwrap().as_ref() {
        Some(l) => (l.client.lock().unwrap().clone(), l.wake.clone()),
        None => return Ok(()),
    };
    match client {
        Some(c) => {
            if !matches!(tokio::time::timeout(Duration::from_secs(3), c.call("hello", Value::Null)).await, Ok(Ok(_))) {
                c.close("the link went quiet while the phone slept");
            }
        }
        None => wake.notify_one(),
    }
    Ok(())
}

/// Calls a method on the connected desktop.
#[tauri::command]
async fn rpc(state: State<'_, AppState>, method: String, params: Value) -> Result<Value, String> {
    let client = state.live.lock().unwrap().as_ref().and_then(|l| l.client.lock().unwrap().clone());
    let client = client.ok_or("Not connected to the desktop.")?;
    client.call(&method, params).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from("."));
            app.manage(AppState::load(dir.join("desktops.json")));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![desktops, pair, forget, connect, disconnect, wake, rpc])
        .run(tauri::generate_context!())
        .expect("error while running oah");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_the_default_port() {
        assert_eq!(with_port("192.168.1.20"), "192.168.1.20:7787");
        assert_eq!(with_port(" 192.168.1.20:9000 "), "192.168.1.20:9000");
        assert_eq!(with_port("ws://pc.local:7787/"), "pc.local:7787");
        assert_eq!(with_port("fe80::1"), "[fe80::1]:7787");
        assert_eq!(with_port("[fe80::1]:80"), "[fe80::1]:80");
    }

    #[test]
    fn keeps_its_key() {
        let dir = std::env::temp_dir().join(format!("oah-android-test-{}", std::process::id()));
        let path = dir.join("desktops.json");
        let a = AppState::load(path.clone());
        let b = AppState::load(path);
        assert_eq!(a.identity, b.identity);
        let _ = std::fs::remove_dir_all(dir);
    }
}
