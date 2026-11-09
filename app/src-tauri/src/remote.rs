//! Phones (the oah Android app): the server they connect to while Settings → Phone has it on,
//! the pairing code, and the paired devices. The protocol lives in `oah-remote`; this file
//! answers its calls with the same chats the window uses.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};

use oah_agents::driver::Prompt;
use oah_agents::event::Decision;
use oah_core::chats::now_ms;
use oah_core::paths;
use oah_core::settings::{AgentKind, PairedDevice, PermissionMode};
use oah_remote::pairing;
use oah_remote::server::{Event, Feed, Host, Server};
use oah_remote::Identity;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::broadcast;

use crate::{err, AppState, CmdResult};

#[derive(Default)]
pub struct Remote {
    server: Mutex<Option<Server>>,
    discovery: Mutex<Option<oah_remote::discovery::Beacon>>,
    /// Why the server isn't running when it should be.
    error: Mutex<Option<String>>,
    identity: OnceLock<Identity>,
}

/// An address phones can reach this computer at.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Address {
    pub ip: String,
    /// The network it's on: the adapter's name, or "Tailscale".
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceState {
    #[serde(flatten)]
    pub device: PairedDevice,
    pub connected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteState {
    pub enabled: bool,
    pub running: bool,
    pub port: u16,
    pub addresses: Vec<Address>,
    pub devices: Vec<DeviceState>,
    pub error: Option<String>,
    pub desktop_name: String,
    pub discovery_available: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingInfo {
    /// `XXXX-XXXX`.
    pub code: String,
    pub expires_at: u64,
    /// `ip:port` the phone can type, best first.
    pub addresses: Vec<String>,
}

// ---- The desktop's key -----------------------------------------------------------------------

const KEY_SERVICE: &str = "dev.oah.app";
const KEY_NAME: &str = "remote/identity";

fn identity_file() -> std::path::PathBuf {
    paths::app_dir().join("remote").join("identity.key")
}

/// The desktop's long-term key: in the keychain, or (where there's none) a file only this user
/// can read. Made the first time it's needed.
fn load_identity() -> Identity {
    let entry = keyring::Entry::new(KEY_SERVICE, KEY_NAME).ok();
    if let Some(id) = entry.as_ref().and_then(|e| e.get_password().ok()).and_then(|s| Identity::from_private_b64(&s)) {
        return id;
    }
    if let Some(id) = std::fs::read_to_string(identity_file()).ok().and_then(|s| Identity::from_private_b64(&s)) {
        return id;
    }
    let id = Identity::generate();
    if entry.as_ref().is_some_and(|e| e.set_password(&id.private_b64()).is_ok()) {
        return id;
    }
    let path = identity_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = write_private(&path, &id.private_b64()) {
        eprintln!("oah: couldn't save the phone key: {e}");
    }
    id
}

fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
        f.write_all(text.as_bytes())
    }
    #[cfg(not(unix))]
    std::fs::write(path, text)
}

fn identity(app: &AppHandle) -> Identity {
    app.state::<AppState>().remote.identity.get_or_init(load_identity).clone()
}

fn desktop_name() -> String {
    let n = gethostname::gethostname().to_string_lossy().trim().to_string();
    if n.is_empty() {
        "oah desktop".into()
    } else {
        n
    }
}

// ---- Addresses -------------------------------------------------------------------------------

/// The IPv4 addresses a phone might reach: LAN first, then VPNs; no loopback, link-local or
/// virtual adapters (WSL's and Hyper-V's switches, Docker bridges) that a phone can't get to.
pub fn addresses() -> Vec<Address> {
    let mut out: Vec<(u8, Address)> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| {
            let IpAddr::V4(ip) = i.ip() else { return None };
            if ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() {
                return None;
            }
            let name = i.name.to_lowercase();
            if ["vethernet", "wsl", "docker", "br-", "veth", "virbr", "vmnet", "vboxnet", "hyper-v"].iter().any(|v| name.contains(v)) {
                return None;
            }
            let tailscale = ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1]);
            let rank = if tailscale {
                2
            } else if ip.is_private() {
                0
            } else {
                1
            };
            let label = if tailscale || name.contains("tailscale") { "Tailscale".to_string() } else { i.name.clone() };
            Some((rank, Address { ip: ip.to_string(), label }))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.ip.cmp(&b.1.ip)));
    out.dedup_by(|a, b| a.1.ip == b.1.ip);
    out.into_iter().map(|(_, a)| a).collect()
}

// ---- The host --------------------------------------------------------------------------------

struct AppHost {
    app: AppHandle,
    identity: Identity,
    name: String,
}

fn str_param(p: &Value, k: &str) -> CmdResult<String> {
    p.get(k).and_then(Value::as_str).map(String::from).ok_or_else(|| format!("{k} is missing"))
}

fn param<T: for<'de> Deserialize<'de>>(p: &Value, k: &str) -> CmdResult<T> {
    serde_json::from_value(p.get(k).cloned().unwrap_or(Value::Null)).map_err(|e| format!("{k}: {e}"))
}

fn opt_param<T: for<'de> Deserialize<'de>>(p: &Value, k: &str) -> CmdResult<Option<T>> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| format!("{k}: {e}")),
    }
}

fn agent_name(k: AgentKind) -> &'static str {
    match k {
        AgentKind::Claude => "Claude Code",
        AgentKind::Codex => "Codex",
        AgentKind::Opencode => "OpenCode",
        AgentKind::Opencode2 => "OpenCode 2",
    }
}

fn to_value(v: impl Serialize) -> CmdResult<Value> {
    serde_json::to_value(v).map_err(err)
}

impl Host for AppHost {
    fn identity(&self) -> Identity {
        self.identity.clone()
    }

    fn desktop_name(&self) -> String {
        self.name.clone()
    }

    fn device(&self, public_key: &[u8; 32]) -> Option<String> {
        let key = oah_remote::b64(public_key);
        let state = self.app.state::<AppState>();
        let s = state.settings.lock().unwrap();
        s.remote.devices.iter().find(|d| d.public_key == key).map(|d| d.id.clone())
    }

    fn pair(&self, name: &str, public_key: &[u8; 32]) -> Result<String, String> {
        let state = self.app.state::<AppState>();
        let mut s = state.settings.lock().unwrap();
        let mut next = s.clone();
        let key = oah_remote::b64(public_key);
        // The same phone pairing again replaces its old entry.
        next.remote.devices.retain(|d| d.public_key != key);
        let id = uuid::Uuid::new_v4().simple().to_string();
        next.remote.devices.push(PairedDevice {
            id: id.clone(),
            name: name.to_string(),
            public_key: key,
            paired_at: now_ms(),
            last_seen: 0,
        });
        next.save(&paths::settings_file()).map_err(err)?;
        *s = next;
        Ok(id)
    }

    fn call(&self, _device_id: &str, method: &str, p: Value) -> Result<Value, String> {
        let app = &self.app;
        let state = app.state::<AppState>();
        let chats = &state.chats;
        let settings = || state.settings.lock().unwrap().clone();
        match method {
            "projects.list" => to_value(&settings().projects),
            "chats.list" => {
                // What each running chat waits for, so the list can put those first.
                let running = chats.running();
                let waiting: serde_json::Map<String, Value> =
                    chats.waiting().into_iter().map(|(id, what)| (id, Value::from(what))).collect();
                Ok(json!({ "metas": chats.store.list(), "running": running, "busy": chats.busy(), "waiting": waiting }))
            }
            "agents.list" => {
                let s = settings();
                let p = &s.providers;
                let agents: Vec<Value> = AgentKind::ALL
                    .iter()
                    .filter(|k| p.get(**k).enabled)
                    .map(|k| {
                        let pr = p.get(*k);
                        json!({ "kind": k, "name": agent_name(*k), "defaultModel": pr.default_model, "permissionMode": pr.permission_mode })
                    })
                    .collect();
                Ok(json!({ "agents": agents, "defaultAgent": p.default_agent }))
            }
            "chat.create" => to_value(crate::create_chat(app, &state, serde_json::from_value(p).map_err(err)?)?),
            "chat.send" => {
                let mut prompt: Prompt = param(&p, "prompt")?;
                // Saved image names come from the desktop, never from the phone.
                prompt.files.clear();
                let permission: Option<PermissionMode> = opt_param(&p, "permission")?;
                chats.send(app, &settings(), &str_param(&p, "chatId")?, prompt, permission).map(|_| Value::Null)
            }
            "chat.interrupt" => chats.interrupt(&str_param(&p, "chatId")?).map(|_| Value::Null),
            "chat.answer" => {
                let decision: Decision = param(&p, "decision")?;
                chats.answer(&str_param(&p, "chatId")?, &str_param(&p, "requestId")?, decision).map(|_| Value::Null)
            }
            "chat.answerQuestion" => {
                let answers = param(&p, "answers")?;
                chats.answer_question(&str_param(&p, "chatId")?, &str_param(&p, "requestId")?, answers).map(|_| Value::Null)
            }
            "chat.setModel" => to_value(chats.set_model(&str_param(&p, "chatId")?, &str_param(&p, "model")?)?),
            "chat.setEffort" => to_value(chats.set_effort(&str_param(&p, "chatId")?, &str_param(&p, "effort")?)?),
            // Announced, so the window shows the mode the phone picked.
            "chat.setPermission" => chats.set_permission(app, &str_param(&p, "chatId")?, param(&p, "mode")?, true).map(|_| Value::Null),
            "chat.compact" => {
                let permission: Option<PermissionMode> = opt_param(&p, "permission")?;
                let instructions = p.get("instructions").and_then(Value::as_str).unwrap_or("");
                chats.compact(app, &settings(), &str_param(&p, "chatId")?, instructions, permission).map(|_| Value::Null)
            }
            "chat.clear" => chats.clear(app, &str_param(&p, "chatId")?).map(|_| Value::Null),
            "chat.readImage" => {
                let meta = chats.store.get(&str_param(&p, "chatId")?).map_err(err)?;
                let settings = settings();
                let project = settings.projects.iter().find(|project| project.id == meta.project_id).ok_or("no such project")?;
                let cwd = if meta.cwd.is_empty() { project.path.clone() } else { meta.cwd };
                let repo = oah_git::Repo::new(project.location.clone(), cwd);
                oah_git::read_image(&repo, &str_param(&p, "path")?, 20 * 1024 * 1024).map(Value::String)
            }
            "chat.image" => chats.image(&str_param(&p, "chatId")?, &str_param(&p, "file")?).map(Value::String),
            "chat.rename" => {
                let title = str_param(&p, "title")?;
                let m = chats.edit(&str_param(&p, "chatId")?, |m| m.title = title.trim().to_string())?;
                chats.changed(app);
                to_value(m)
            }
            "chat.pin" => {
                let pinned: bool = param(&p, "pinned")?;
                let m = chats.edit(&str_param(&p, "chatId")?, |m| m.pinned = pinned)?;
                chats.changed(app);
                to_value(m)
            }
            "chat.sleep" => chats.sleep(app, &str_param(&p, "chatId")?, std::time::Duration::ZERO).map(|_| Value::Null),
            "chat.delete" => {
                chats.delete(&str_param(&p, "chatId")?)?;
                chats.changed(app);
                Ok(Value::Null)
            }
            "provider.catalog" => {
                let kind: AgentKind = param(&p, "kind")?;
                let project: Option<String> = opt_param(&p, "projectId")?;
                to_value(crate::catalog(&settings(), kind, project.as_deref())?)
            }
            m => Err(format!("unknown method {m}")),
        }
    }

    fn open_chat(&self, chat_id: &str) -> Result<(Value, u64), String> {
        let snap = self.app.state::<AppState>().chats.snapshot(chat_id)?;
        let seq = snap.seq;
        Ok((to_value(snap)?, seq))
    }

    fn feed(&self) -> broadcast::Receiver<Feed> {
        self.app.state::<AppState>().chats.feed()
    }

    fn event(&self, e: Event) {
        match &e {
            Event::Connected { device_id } | Event::Disconnected { device_id } => {
                let state = self.app.state::<AppState>();
                let mut s = state.settings.lock().unwrap();
                let mut next = s.clone();
                if let Some(d) = next.remote.devices.iter_mut().find(|d| &d.id == device_id) {
                    d.last_seen = now_ms();
                    if next.save(&paths::settings_file()).is_ok() {
                        *s = next;
                    }
                }
            }
            Event::Paired { name, .. } => {
                let _ = self.app.emit("remote-paired", json!({ "name": name }));
            }
            Event::PairRequested { name } => {
                let _ = self.app.emit("remote-pair-request", json!({ "name": name }));
            }
            Event::PairFailed { left } => {
                let _ = self.app.emit("remote-pair-failed", json!({ "left": left }));
            }
        }
        let _ = self.app.emit("remote-state", ());
    }
}

// ---- Starting and stopping -------------------------------------------------------------------

/// Starts the server on `port` (or, when that's taken, on a free one, which is then saved).
async fn start(app: &AppHandle, port: u16) -> Result<u16, String> {
    let host = Arc::new(AppHost { app: app.clone(), identity: identity(app), name: desktop_name() });
    let any = |port| SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
    let server = match Server::bind(any(port), host.clone()).await {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            Server::bind(any(0), host).await.map_err(|e| format!("couldn't listen for phones: {e}"))?
        }
        Err(e) => return Err(format!("couldn't listen for phones on port {port}: {e}")),
    };
    let bound = server.local_addr().port();
    let discovery = oah_remote::discovery::advertise(desktop_name(), bound).await;
    if let Err(e) = &discovery {
        eprintln!("oah: nearby discovery unavailable: {e}");
    }
    let state = app.state::<AppState>();
    *state.remote.discovery.lock().unwrap() = discovery.ok();
    *state.remote.server.lock().unwrap() = Some(server);
    if bound != port {
        let mut s = state.settings.lock().unwrap();
        let mut next = s.clone();
        next.remote.port = bound;
        if next.save(&paths::settings_file()).is_ok() {
            *s = next;
        }
    }
    Ok(bound)
}

fn stop(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.remote.discovery.lock().unwrap().take();
    let server = state.remote.server.lock().unwrap().take();
    drop(server);
}

/// Called once at startup.
pub fn start_if_enabled(app: &AppHandle) {
    let (enabled, port) = {
        let state = app.state::<AppState>();
        let s = state.settings.lock().unwrap();
        (s.remote.enabled, s.remote.port)
    };
    if enabled {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let r = start(&app, port).await;
            *app.state::<AppState>().remote.error.lock().unwrap() = r.err();
            let _ = app.emit("remote-state", ());
        });
    }
}

fn state_of(app: &AppHandle) -> RemoteState {
    let state = app.state::<AppState>();
    let s = state.settings.lock().unwrap().remote.clone();
    let (running, port, connected) = match state.remote.server.lock().unwrap().as_ref() {
        Some(server) => (true, server.local_addr().port(), server.connected()),
        None => (false, s.port, vec![]),
    };
    let error = state.remote.error.lock().unwrap().clone();
    let discovery_available = state.remote.discovery.lock().unwrap().is_some();
    RemoteState {
        enabled: s.enabled,
        running,
        port,
        addresses: addresses(),
        devices: s.devices.into_iter().map(|d| DeviceState { connected: connected.contains(&d.id), device: d }).collect(),
        error,
        desktop_name: desktop_name(),
        discovery_available,
    }
}

fn save_remote(state: &AppState, f: impl FnOnce(&mut oah_core::settings::Remote)) -> CmdResult<()> {
    let mut s = state.settings.lock().unwrap();
    let mut next = s.clone();
    f(&mut next.remote);
    next.save(&paths::settings_file()).map_err(err)?;
    *s = next;
    Ok(())
}

// ---- Commands --------------------------------------------------------------------------------

#[tauri::command]
pub fn remote_state(app: AppHandle) -> RemoteState {
    state_of(&app)
}

/// Turns phone access on (the server starts) or off (it stops, and every phone disconnects).
#[tauri::command]
pub async fn remote_set_enabled(app: AppHandle, enabled: bool) -> CmdResult<RemoteState> {
    let port = {
        let state = app.state::<AppState>();
        save_remote(&state, |r| r.enabled = enabled)?;
        let port = state.settings.lock().unwrap().remote.port;
        port
    };
    stop(&app);
    let r = if enabled { start(&app, port).await.map(|_| ()) } else { Ok(()) };
    *app.state::<AppState>().remote.error.lock().unwrap() = r.clone().err();
    let _ = app.emit("remote-state", ());
    r.map(|_| state_of(&app))
}

#[tauri::command]
pub async fn remote_set_port(app: AppHandle, port: u16) -> CmdResult<RemoteState> {
    if port < 1024 {
        return Err("Pick a port from 1024 to 65535.".into());
    }
    let enabled = {
        let state = app.state::<AppState>();
        save_remote(&state, |r| r.port = port)?;
        let enabled = state.settings.lock().unwrap().remote.enabled;
        enabled
    };
    if enabled {
        stop(&app);
        let r = start(&app, port).await.map(|_| ());
        *app.state::<AppState>().remote.error.lock().unwrap() = r.clone().err();
        r?;
    }
    let _ = app.emit("remote-state", ());
    Ok(state_of(&app))
}

/// A new pairing code (any earlier one stops working), and the addresses to type with it.
#[tauri::command]
pub fn remote_pair_start(state: State<AppState>) -> CmdResult<PairingInfo> {
    let guard = state.remote.server.lock().unwrap();
    let server = guard.as_ref().ok_or("Turn on phone access first.")?;
    let port = server.local_addr().port();
    let code = server.open_pairing();
    drop(guard);
    let addresses: Vec<String> = addresses().into_iter().map(|a| format!("{}:{port}", a.ip)).collect();
    if addresses.is_empty() {
        return Err("This computer isn't on a network a phone can reach.".into());
    }
    Ok(PairingInfo { code: pairing::format_code(&code.code), expires_at: code.expires_at, addresses })
}

#[tauri::command]
pub fn remote_pair_cancel(state: State<AppState>) {
    if let Some(s) = state.remote.server.lock().unwrap().as_ref() {
        s.cancel_pairing();
    }
}

/// Forgets a phone: it can't connect again until it pairs again, and its session ends now.
#[tauri::command]
pub fn remote_revoke(app: AppHandle, state: State<AppState>, device_id: String) -> CmdResult<RemoteState> {
    save_remote(&state, |r| r.devices.retain(|d| d.id != device_id))?;
    if let Some(s) = state.remote.server.lock().unwrap().as_ref() {
        s.kick(&device_id);
    }
    let _ = app.emit("remote-state", ());
    Ok(state_of(&app))
}
