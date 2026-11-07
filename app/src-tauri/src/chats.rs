//! Chats at runtime: one agent driver per open chat. Every event a driver reports is appended to
//! the chat's log, folded into its meta (title, CLI session id, usage) and emitted to the UI as
//! `chat-event` `{ chatId, event }`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use oah_agents::driver::{self, Driver, Prompt, StartOptions};
use oah_agents::event::{ChatEvent, Decision};
use oah_core::chats::{now_ms, title_from, ChatMeta, ChatStore};
use oah_core::settings::{AgentKind, PermissionMode, Settings};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Envelope<'a> {
    chat_id: &'a str,
    event: &'a ChatEvent,
}

pub struct Chats {
    pub store: ChatStore,
    live: Mutex<HashMap<String, Box<dyn Driver>>>,
    /// Metas of chats with a live driver, updated as events arrive.
    metas: Arc<Mutex<HashMap<String, ChatMeta>>>,
}

impl Chats {
    pub fn new(store: ChatStore) -> Chats {
        Chats { store, live: Mutex::new(HashMap::new()), metas: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn create(&self, project_id: &str, agent: AgentKind, model: &str, cwd: &str) -> Result<ChatMeta, String> {
        self.store.create(project_id, agent, model, cwd).map_err(|e| e.to_string())
    }

    /// Sends `prompt`, starting the chat's agent first if it isn't running.
    pub fn send(&self, app: &AppHandle, settings: &Settings, chat_id: &str, prompt: Prompt, permission: Option<PermissionMode>) -> Result<(), String> {
        let mut live = self.live.lock().unwrap();
        if !live.contains_key(chat_id) {
            let driver = self.start(app, settings, chat_id, permission)?;
            live.insert(chat_id.to_string(), driver);
        }
        {
            let mut metas = self.metas.lock().unwrap();
            if let Some(m) = metas.get_mut(chat_id) {
                if m.title.is_empty() {
                    m.title = title_from(&prompt.text);
                }
                m.updated_at = now_ms();
                let _ = self.store.save(m);
            }
        }
        match live.get_mut(chat_id).unwrap().send(prompt.clone()) {
            // The CLI exited since the last message: start it again, resuming its session.
            Err(e) if e.contains("isn't running") => {
                live.remove(chat_id);
                let mut driver = self.start(app, settings, chat_id, permission)?;
                let r = driver.send(prompt);
                live.insert(chat_id.to_string(), driver);
                r
            }
            r => r,
        }
    }

    fn start(&self, app: &AppHandle, settings: &Settings, chat_id: &str, permission: Option<PermissionMode>) -> Result<Box<dyn Driver>, String> {
        let meta = self.store.get(chat_id).map_err(|e| e.to_string())?;
        let project = settings.projects.iter().find(|p| p.id == meta.project_id).ok_or("this chat's project was removed")?;
        let provider = settings.providers.get(meta.agent);
        if !provider.enabled {
            return Err(format!("{:?} is turned off in Settings → Providers", meta.agent));
        }
        let model = Some(meta.model.clone()).filter(|m| !m.is_empty()).or_else(|| Some(project.default_model.clone()).filter(|m| !m.is_empty())).or_else(|| Some(provider.default_model.clone()).filter(|m| !m.is_empty()));
        let opts = StartOptions {
            kind: meta.agent,
            cwd: if meta.cwd.is_empty() { project.path.clone() } else { meta.cwd.clone() },
            location: project.location.clone(),
            binary: provider.binary.clone(),
            model,
            permission: permission.unwrap_or(provider.permission_mode),
            resume: meta.agent_session_id.clone(),
            env: provider.env.clone(),
        };
        self.metas.lock().unwrap().insert(chat_id.to_string(), meta);

        let (app, store, metas, id) = (app.clone(), self.store.clone(), self.metas.clone(), chat_id.to_string());
        let sink: driver::Sink = Arc::new(move |event: ChatEvent| {
            if let Err(e) = store.append(&id, &event) {
                eprintln!("oah: couldn't log chat {id}: {e}");
            }
            fold(&store, &metas, &id, &event);
            let _ = app.emit("chat-event", Envelope { chat_id: &id, event: &event });
        });
        driver::start(opts, sink)
    }

    pub fn interrupt(&self, chat_id: &str) -> Result<(), String> {
        match self.live.lock().unwrap().get_mut(chat_id) {
            Some(d) => d.interrupt(),
            None => Ok(()),
        }
    }

    pub fn answer(&self, chat_id: &str, request_id: &str, decision: Decision) -> Result<(), String> {
        match self.live.lock().unwrap().get_mut(chat_id) {
            Some(d) => d.answer(request_id, decision),
            None => Err("this chat's agent isn't running anymore".into()),
        }
    }

    pub fn set_permission(&self, chat_id: &str, mode: PermissionMode) -> Result<(), String> {
        match self.live.lock().unwrap().get_mut(chat_id) {
            Some(d) => d.set_permission(mode),
            None => Ok(()),
        }
    }

    /// Ends the chat's agent (it starts again, resumed, on the next message).
    pub fn close(&self, chat_id: &str) {
        if let Some(mut d) = self.live.lock().unwrap().remove(chat_id) {
            d.stop();
        }
        self.metas.lock().unwrap().remove(chat_id);
    }

    pub fn delete(&self, chat_id: &str) -> Result<(), String> {
        self.close(chat_id);
        self.store.delete(chat_id).map_err(|e| e.to_string())
    }

    pub fn running(&self) -> Vec<String> {
        self.live.lock().unwrap().keys().cloned().collect()
    }

    pub fn stop_all(&self) {
        for (_, mut d) in self.live.lock().unwrap().drain() {
            d.stop();
        }
    }
}

/// Keeps the meta in step with what happened: CLI session id, usage, and an `Exited` agent
/// is forgotten so the next message starts it again.
fn fold(store: &ChatStore, metas: &Mutex<HashMap<String, ChatMeta>>, id: &str, event: &ChatEvent) {
    let mut metas = metas.lock().unwrap();
    let Some(m) = metas.get_mut(id) else { return };
    let changed = match event {
        ChatEvent::Started { session_id: Some(s), .. } if m.agent_session_id.as_deref() != Some(s) => {
            m.agent_session_id = Some(s.clone());
            true
        }
        ChatEvent::TurnFinished { usage, cost_usd, .. } => {
            m.tokens += usage.map(|u| u.total()).unwrap_or(0);
            m.cost_usd += cost_usd.unwrap_or(0.0);
            m.updated_at = now_ms();
            true
        }
        _ => false,
    };
    if changed {
        let _ = store.save(m);
    }
}
