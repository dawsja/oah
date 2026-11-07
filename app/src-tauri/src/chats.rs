//! Chats at runtime: one agent driver per open chat. Every event a driver reports is appended to
//! the chat's log, folded into its meta (title, CLI session id, usage) and emitted to the UI as
//! `chat-event` `{ chatId, event }`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use oah_agents::driver::{self, Driver, Prompt, StartOptions};
use oah_agents::event::{ChatEvent, Decision, NoticeLevel};
use oah_core::chats::{now_ms, title_from, ChatMeta, ChatStore};
use oah_core::settings::{AgentKind, Location, PermissionMode, Settings};
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
    /// Chats whose CLI restarts before the next message (an effort it only takes at start).
    restart: Mutex<std::collections::HashSet<String>>,
}

impl Chats {
    pub fn new(store: ChatStore) -> Chats {
        Chats {
            store,
            live: Mutex::new(HashMap::new()),
            metas: Arc::new(Mutex::new(HashMap::new())),
            restart: Mutex::new(std::collections::HashSet::new()),
        }
    }

    pub fn create(
        &self,
        project_id: &str,
        agent: AgentKind,
        acp_agent: Option<String>,
        model: &str,
        effort: &str,
        cwd: &str,
    ) -> Result<ChatMeta, String> {
        let mut meta = self.store.create(project_id, agent, model, cwd).map_err(|e| e.to_string())?;
        if agent == AgentKind::Acp || !effort.is_empty() {
            meta.acp_agent = acp_agent.filter(|_| agent == AgentKind::Acp);
            meta.effort = effort.to_string();
            self.store.save(&meta).map_err(|e| e.to_string())?;
        }
        Ok(meta)
    }

    /// Saves a prompt's images with the chat (so a reopened chat still shows them); returns the
    /// file names, in order.
    fn save_images(&self, chat_id: &str, prompt: &Prompt) -> Vec<String> {
        use base64::Engine;
        let Ok(dir) = self.store.images_dir(chat_id) else { return vec![] };
        prompt
            .images
            .iter()
            .enumerate()
            .filter_map(|(i, img)| {
                let ext = match img.mime.as_str() {
                    "image/png" => "png",
                    "image/jpeg" => "jpg",
                    "image/gif" => "gif",
                    "image/webp" => "webp",
                    _ => "img",
                };
                let bytes = base64::engine::general_purpose::STANDARD.decode(img.data.as_bytes()).ok()?;
                let name = format!("{}-{i}.{ext}", now_ms());
                std::fs::write(dir.join(&name), bytes).ok()?;
                Some(name)
            })
            .collect()
    }

    /// A saved image as a `data:` URL.
    pub fn image(&self, chat_id: &str, file: &str) -> Result<String, String> {
        use base64::Engine;
        let path = self.store.image_path(chat_id, file).map_err(|e| e.to_string())?;
        let bytes = std::fs::read(&path).map_err(|_| "that image is gone".to_string())?;
        let mime = match file.rsplit('.').next() {
            Some("png") => "image/png",
            Some("jpg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            _ => "application/octet-stream",
        };
        Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
    }

    /// Sends `prompt`, starting the chat's agent first if it isn't running.
    pub fn send(
        &self,
        app: &AppHandle,
        settings: &Settings,
        chat_id: &str,
        mut prompt: Prompt,
        permission: Option<PermissionMode>,
    ) -> Result<(), String> {
        if !prompt.images.is_empty() && prompt.files.is_empty() {
            prompt.files = self.save_images(chat_id, &prompt);
        }
        let mut live = self.live.lock().unwrap();
        // An effort the CLI only takes at start: restart it (resuming) now that it's idle.
        if self.restart.lock().unwrap().contains(chat_id) && live.get(chat_id).is_some_and(|d| d.idle()) {
            if let Some(mut d) = live.remove(chat_id) {
                d.stop();
            }
            self.restart.lock().unwrap().remove(chat_id);
        }
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

    fn start(
        &self,
        app: &AppHandle,
        settings: &Settings,
        chat_id: &str,
        permission: Option<PermissionMode>,
    ) -> Result<Box<dyn Driver>, String> {
        let meta = self.store.get(chat_id).map_err(|e| e.to_string())?;
        let project = settings.projects.iter().find(|p| p.id == meta.project_id).ok_or("this chat's project was removed")?;
        let provider = settings
            .providers
            .for_chat(meta.agent, meta.acp_agent.as_deref())
            .ok_or("this chat's agent was removed in Settings → Providers")?;
        if !provider.enabled {
            return Err("this agent is turned off in Settings → Providers".into());
        }
        let args = meta.acp_agent.as_deref().and_then(|id| settings.providers.acp_agent(id)).map(|a| a.args.clone()).unwrap_or_default();
        let model = Some(meta.model.clone())
            .filter(|m| !m.is_empty())
            .or_else(|| Some(project.default_model.clone()).filter(|m| !m.is_empty()))
            .or_else(|| Some(provider.default_model.clone()).filter(|m| !m.is_empty()));
        let opts = StartOptions {
            kind: meta.agent,
            cwd: if meta.cwd.is_empty() { project.path.clone() } else { meta.cwd.clone() },
            location: project.location.clone(),
            binary: provider.binary_on(&project.location),
            model,
            effort: Some(meta.effort.clone()).filter(|e| !e.is_empty()),
            permission: permission.unwrap_or(provider.permission_mode),
            resume: meta.agent_session_id.clone(),
            env: crate::secrets::resolve(&crate::secrets::scope(meta.agent, meta.acp_agent.as_deref()), &provider.env),
            args,
        };
        // A stopped distro starts here (a few seconds), so a missing or broken one says so
        // plainly instead of as an agent that exited.
        if let Location::Wsl { distro } = &project.location {
            oah_transport::wsl::start(distro)?;
        }
        self.metas.lock().unwrap().insert(chat_id.to_string(), meta);

        let (app, store, metas, id) = (app.clone(), self.store.clone(), self.metas.clone(), chat_id.to_string());
        let log = Mutex::new(Compactor::default());
        let sink: driver::Sink = Arc::new(move |event: ChatEvent| {
            for e in log.lock().unwrap().push(&event) {
                if let Err(err) = store.append(&id, &e) {
                    eprintln!("oah: couldn't log chat {id}: {err}");
                }
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

    pub fn answer_question(
        &self,
        chat_id: &str,
        request_id: &str,
        answers: std::collections::BTreeMap<String, Vec<String>>,
    ) -> Result<(), String> {
        match self.live.lock().unwrap().get_mut(chat_id) {
            Some(d) => d.answer_question(request_id, answers),
            None => Err("this chat's agent isn't running anymore".into()),
        }
    }

    /// Switches a chat's model: saved for next time, and applied now if its agent runs.
    pub fn set_model(&self, chat_id: &str, model: &str) -> Result<ChatMeta, String> {
        let mut meta = self.store.get(chat_id).map_err(|e| e.to_string())?;
        meta.model = model.to_string();
        self.store.save(&meta).map_err(|e| e.to_string())?;
        if let Some(m) = self.metas.lock().unwrap().get_mut(chat_id) {
            m.model = model.to_string();
        }
        if let Some(d) = self.live.lock().unwrap().get_mut(chat_id) {
            d.set_model(model)?;
        }
        Ok(meta)
    }

    /// Switches a chat's reasoning effort: saved, and applied now when the CLI can; otherwise the
    /// CLI restarts (resuming its session) before the next message.
    pub fn set_effort(&self, chat_id: &str, effort: &str) -> Result<ChatMeta, String> {
        let mut meta = self.store.get(chat_id).map_err(|e| e.to_string())?;
        meta.effort = effort.to_string();
        self.store.save(&meta).map_err(|e| e.to_string())?;
        if let Some(m) = self.metas.lock().unwrap().get_mut(chat_id) {
            m.effort = effort.to_string();
        }
        if let Some(d) = self.live.lock().unwrap().get_mut(chat_id) {
            if !d.set_effort(effort)? {
                self.restart.lock().unwrap().insert(chat_id.to_string());
            }
        }
        Ok(meta)
    }

    pub fn set_permission(&self, chat_id: &str, mode: PermissionMode) -> Result<(), String> {
        match self.live.lock().unwrap().get_mut(chat_id) {
            Some(d) => d.set_permission(mode),
            None => Ok(()),
        }
    }

    /// `/compact`: the agent's own way when it has one, else `/compact` as a message.
    pub fn compact(
        &self,
        app: &AppHandle,
        settings: &Settings,
        chat_id: &str,
        instructions: &str,
        permission: Option<PermissionMode>,
    ) -> Result<(), String> {
        {
            let mut live = self.live.lock().unwrap();
            if !live.contains_key(chat_id) {
                let driver = self.start(app, settings, chat_id, permission)?;
                live.insert(chat_id.to_string(), driver);
            }
            if live.get_mut(chat_id).unwrap().compact(instructions)? {
                return Ok(());
            }
        }
        let text = format!("/compact {}", instructions.trim()).trim_end().to_string();
        self.send(app, settings, chat_id, Prompt { id: format!("compact-{}", now_ms()), text, images: vec![], files: vec![] }, permission)
    }

    /// `/clear`: the agent starts a new session with the next message. The chat keeps what was
    /// said, with a line marking where the agent's memory starts again.
    pub fn clear(&self, app: &AppHandle, chat_id: &str) -> Result<(), String> {
        self.close(chat_id);
        let mut meta = self.store.get(chat_id).map_err(|e| e.to_string())?;
        meta.agent_session_id = None;
        self.store.save(&meta).map_err(|e| e.to_string())?;
        let event = ChatEvent::Notice {
            level: NoticeLevel::Info,
            text: "Context cleared. The agent starts fresh from the next message; it no longer sees what's above.".into(),
        };
        self.store.append(chat_id, &event).map_err(|e| e.to_string())?;
        let _ = app.emit("chat-event", Envelope { chat_id, event: &event });
        Ok(())
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

/// What goes in the chat's log: streamed text and reasoning are held back and written once per
/// item, as a whole `Text` / `Reasoning` event, when anything else happens. The UI still gets
/// every delta live; the log stays one line per message instead of one per fragment.
#[derive(Default)]
pub(crate) struct Compactor {
    /// (item id, is reasoning) → text so far, in the order the items started.
    held: Vec<((String, bool), String)>,
}

impl Compactor {
    pub(crate) fn push(&mut self, e: &ChatEvent) -> Vec<ChatEvent> {
        let mut hold = |id: &str, reasoning: bool, text: &str, replace: bool| match self
            .held
            .iter_mut()
            .find(|(k, _)| k.0 == id && k.1 == reasoning)
        {
            Some((_, t)) if replace => *t = text.to_string(),
            Some((_, t)) => t.push_str(text),
            None => self.held.push(((id.to_string(), reasoning), text.to_string())),
        };
        match e {
            ChatEvent::TextDelta { item_id, text } => hold(item_id, false, text, false),
            ChatEvent::ReasoningDelta { item_id, text } => hold(item_id, true, text, false),
            ChatEvent::Text { item_id, text } => hold(item_id, false, text, true),
            ChatEvent::Reasoning { item_id, text } => hold(item_id, true, text, true),
            other => {
                let mut out = self.flush();
                out.push(other.clone());
                return out;
            }
        }
        Vec::new()
    }

    fn flush(&mut self) -> Vec<ChatEvent> {
        self.held
            .drain(..)
            .map(
                |((item_id, reasoning), text)| {
                    if reasoning {
                        ChatEvent::Reasoning { item_id, text }
                    } else {
                        ChatEvent::Text { item_id, text }
                    }
                },
            )
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compacts_streamed_text() {
        let mut c = Compactor::default();
        let d = |t: &str| ChatEvent::TextDelta { item_id: "a".into(), text: t.into() };
        assert!(c.push(&d("Hel")).is_empty());
        assert!(c.push(&ChatEvent::ReasoningDelta { item_id: "r".into(), text: "hm".into() }).is_empty());
        assert!(c.push(&d("lo")).is_empty());
        let out = c.push(&ChatEvent::TurnStarted { turn_id: "t".into() });
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], ChatEvent::Text { item_id: "a".into(), text: "Hello".into() });
        assert_eq!(out[1], ChatEvent::Reasoning { item_id: "r".into(), text: "hm".into() });
        assert!(matches!(out[2], ChatEvent::TurnStarted { .. }));
        assert!(c.push(&ChatEvent::TurnStarted { turn_id: "u".into() }).len() == 1);
    }
}
