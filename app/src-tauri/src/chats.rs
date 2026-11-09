//! Chats at runtime: one agent driver per open chat. Every event a driver reports is appended to
//! the chat's log, folded into its meta (title, CLI session id, usage), emitted to the UI as
//! `chat-event` `{ chatId, event }` and fed, numbered, to paired phones ([`Chats::feed`]).
//! [`Chats::snapshot`] is a chat as it is right now, mid-turn included, lined up with that
//! numbering.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oah_agents::driver::{self, Driver, Prompt, StartOptions};
use oah_agents::event::{ChatEvent, Decision, NoticeLevel};
use oah_core::chats::{now_ms, title_from, ChatMeta, ChatStore};
use oah_core::settings::{AgentKind, Location, PermissionMode, Settings};
use oah_remote::server::Feed;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::broadcast;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Envelope<'a> {
    chat_id: &'a str,
    event: &'a ChatEvent,
    /// The event's number in the chat (as in [`Snapshot::seq`]).
    seq: u64,
}

struct Activity {
    last: Instant,
    busy: bool,
    stopped: bool,
}

impl Activity {
    fn observe(&mut self, event: &ChatEvent) {
        self.last = Instant::now();
        match event {
            ChatEvent::UserMessage { .. } | ChatEvent::TurnStarted { .. } | ChatEvent::Approval { .. } | ChatEvent::Question { .. } => {
                self.busy = true
            }
            ChatEvent::TurnFinished { .. } | ChatEvent::Exited { .. } | ChatEvent::Error { .. } => self.busy = false,
            _ => {}
        }
    }

    fn eligible(&self, minimum: Duration) -> bool {
        !self.stopped && !self.busy && self.last.elapsed() >= minimum
    }
}

pub struct Chats {
    pub store: ChatStore,
    /// What each running chat is doing, for MCP (`oah_mcp::Hub`); fed every event.
    pub hub: Arc<oah_mcp::Hub>,
    /// Every chat's text, for search.
    pub index: oah_mcp::search::Index,
    live: Mutex<HashMap<String, Box<dyn Driver>>>,
    activity: Mutex<HashMap<String, Arc<Mutex<Activity>>>>,
    /// Metas of chats with a live driver, updated as events arrive.
    metas: Arc<Mutex<HashMap<String, ChatMeta>>>,
    /// Chats whose CLI restarts before the next message (an effort it only takes at start).
    restart: Mutex<std::collections::HashSet<String>>,
    /// The permission mode each chat last ran with, from the UI or MCP, so both send with the
    /// same one.
    permissions: Mutex<HashMap<String, PermissionMode>>,
    /// Per chat, the log as it's being written (kept while the app runs).
    logs: Mutex<HashMap<String, Arc<Mutex<Log>>>>,
    feed: broadcast::Sender<Feed>,
}

/// A chat's log as it's written: what the Compactor holds back, and how many events it has
/// seen since the app started (each event fed to phones carries its number).
#[derive(Default)]
struct Log {
    compactor: Compactor,
    seq: u64,
}

/// A chat as it is now: its log plus the text still streaming, whether its agent runs and is
/// mid-turn, and the number of the last event in it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub events: Vec<serde_json::Value>,
    pub seq: u64,
    pub live: bool,
    pub busy: bool,
}

/// Writes a chat's events: to its log, to the page, and to phones, in one order.
#[derive(Clone)]
struct Publisher {
    app: AppHandle,
    store: ChatStore,
    feed: broadcast::Sender<Feed>,
    log: Arc<Mutex<Log>>,
    id: String,
}

impl Publisher {
    fn publish(&self, event: &ChatEvent) {
        let mut log = self.log.lock().unwrap();
        for e in log.compactor.push(event) {
            if let Err(err) = self.store.append(&self.id, &e) {
                eprintln!("oah: couldn't log chat {}: {err}", self.id);
            }
        }
        log.seq += 1;
        if self.feed.receiver_count() > 0 {
            if let Ok(v) = serde_json::to_value(event) {
                let _ = self.feed.send(Feed::Chat { chat_id: self.id.clone(), seq: log.seq, event: Arc::new(v) });
            }
        }
        let _ = self.app.emit("chat-event", Envelope { chat_id: &self.id, event, seq: log.seq });
    }
}

impl Chats {
    pub fn new(store: ChatStore) -> Chats {
        Chats {
            store,
            hub: Arc::new(oah_mcp::Hub::new()),
            index: oah_mcp::search::Index::new(),
            live: Mutex::new(HashMap::new()),
            activity: Mutex::new(HashMap::new()),
            metas: Arc::new(Mutex::new(HashMap::new())),
            restart: Mutex::new(std::collections::HashSet::new()),
            permissions: Mutex::new(HashMap::new()),
            logs: Mutex::new(HashMap::new()),
            feed: broadcast::channel(4096).0,
        }
    }

    /// A new chat with the project's and Providers' defaults for whatever `chat` leaves out.
    pub fn create_in(&self, settings: &Settings, chat: oah_mcp::host::NewChat) -> Result<ChatMeta, String> {
        let project = settings.projects.iter().find(|p| p.id == chat.project_id).ok_or("no such project")?;
        let agent = chat.agent.or(project.default_agent).unwrap_or(settings.providers.default_agent);
        let cwd = chat.cwd.unwrap_or_else(|| project.path.clone());
        self.create(&chat.project_id, agent, chat.model.as_deref().unwrap_or(""), chat.effort.as_deref().unwrap_or(""), &cwd)
    }

    /// The mode a chat sends with: the one given (remembered from now on), else the last one.
    fn permission(&self, chat_id: &str, given: Option<PermissionMode>) -> Option<PermissionMode> {
        let mut p = self.permissions.lock().unwrap();
        match given {
            Some(m) => {
                p.insert(chat_id.to_string(), m);
                Some(m)
            }
            None => p.get(chat_id).copied(),
        }
    }

    /// Every chat's events, numbered per chat, and list changes.
    pub fn feed(&self) -> broadcast::Receiver<Feed> {
        self.feed.subscribe()
    }

    fn log(&self, chat_id: &str) -> Arc<Mutex<Log>> {
        self.logs.lock().unwrap().entry(chat_id.to_string()).or_default().clone()
    }

    fn publisher(&self, app: &AppHandle, chat_id: &str) -> Publisher {
        Publisher { app: app.clone(), store: self.store.clone(), feed: self.feed.clone(), log: self.log(chat_id), id: chat_id.to_string() }
    }

    /// The chat list changed (a chat was made, renamed, deleted or finished a turn): the page
    /// and phones list it again.
    pub fn changed(&self, app: &AppHandle) {
        let _ = app.emit("chats-changed", ());
        let _ = self.feed.send(Feed::ChatsChanged);
    }

    /// The chat as it is now, mid-turn included.
    pub fn snapshot(&self, chat_id: &str) -> Result<Snapshot, String> {
        // `live` before the log, never both: a driver reports (and takes the log) while
        // `send` holds `live`.
        let (live, busy) = match self.live.lock().unwrap().get(chat_id) {
            Some(d) => (true, !d.idle()),
            None => (false, false),
        };
        let log = self.log(chat_id);
        let log = log.lock().unwrap();
        let mut events = self.store.events(chat_id).map_err(|e| e.to_string())?;
        events.extend(log.compactor.held().iter().filter_map(|e| serde_json::to_value(e).ok()));
        Ok(Snapshot { events, seq: log.seq, live, busy })
    }

    pub fn create(&self, project_id: &str, agent: AgentKind, model: &str, effort: &str, cwd: &str) -> Result<ChatMeta, String> {
        let mut meta = self.store.create(project_id, agent, model, cwd).map_err(|e| e.to_string())?;
        if !effort.is_empty() {
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
        let permission = self.permission(chat_id, permission);
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
                    self.changed(app);
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
        let mut meta = self.store.get(chat_id).map_err(|e| e.to_string())?;
        meta.sleeping = false;
        self.store.save(&meta).map_err(|e| e.to_string())?;
        let project = settings.projects.iter().find(|p| p.id == meta.project_id).ok_or("this chat's project was removed")?;
        let provider = settings.providers.get(meta.agent);
        if !provider.enabled {
            return Err("this agent is turned off in Settings → Providers".into());
        }
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
            env: crate::secrets::resolve(&crate::secrets::scope(meta.agent), &provider.env),
        };
        // A stopped distro starts here (a few seconds), so a missing or broken one says so
        // plainly instead of as an agent that exited.
        if let Location::Wsl { distro } = &project.location {
            oah_transport::wsl::start(distro)?;
        }
        self.metas.lock().unwrap().insert(chat_id.to_string(), meta);

        let (publisher, metas, hub, feed) = (self.publisher(app, chat_id), self.metas.clone(), self.hub.clone(), self.feed.clone());
        let activity = Arc::new(Mutex::new(Activity { last: Instant::now(), busy: false, stopped: false }));
        self.activity.lock().unwrap().insert(chat_id.to_string(), activity.clone());
        let sink: driver::Sink = Arc::new(move |event: ChatEvent| {
            let mut activity = activity.lock().unwrap();
            if activity.stopped {
                return;
            }
            activity.observe(&event);
            publisher.publish(&event);
            fold(&publisher.store, &metas, &publisher.id, &event);
            hub.push(&publisher.id, &event);
            // The list shows what's busy and what waits for an answer: a turn starting or ending,
            // or a question or approval coming or going, changes it.
            if matches!(
                event,
                ChatEvent::TurnFinished { .. }
                    | ChatEvent::UserMessage { .. }
                    | ChatEvent::Approval { .. }
                    | ChatEvent::ApprovalResolved { .. }
                    | ChatEvent::Question { .. }
                    | ChatEvent::QuestionResolved { .. }
            ) {
                let _ = publisher.app.emit("chats-changed", ());
                let _ = feed.send(Feed::ChatsChanged);
            }
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

    /// Changes a chat's meta on disk, and in memory while its agent runs, so the next usage
    /// update doesn't write the old one back.
    pub fn edit(&self, chat_id: &str, f: impl Fn(&mut ChatMeta)) -> Result<ChatMeta, String> {
        let mut metas = self.metas.lock().unwrap();
        let mut meta = self.store.get(chat_id).map_err(|e| e.to_string())?;
        f(&mut meta);
        self.store.save(&meta).map_err(|e| e.to_string())?;
        if let Some(m) = metas.get_mut(chat_id) {
            f(m);
        }
        Ok(meta)
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

    /// Switches a chat's permission mode: remembered for its next messages, applied now if its
    /// agent runs. `announce` tells the UI (and logs it) when the change came from elsewhere (MCP).
    pub fn set_permission(&self, app: &AppHandle, chat_id: &str, mode: PermissionMode, announce: bool) -> Result<(), String> {
        self.permission(chat_id, Some(mode));
        if let Some(d) = self.live.lock().unwrap().get_mut(chat_id) {
            d.set_permission(mode)?;
        }
        if announce {
            let event = ChatEvent::PermissionChanged { mode };
            self.publisher(app, chat_id).publish(&event);
            self.hub.push(chat_id, &event);
        }
        Ok(())
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
        let permission = self.permission(chat_id, permission);
        {
            let mut live = self.live.lock().unwrap();
            if !live.contains_key(chat_id) {
                let driver = self.start(app, settings, chat_id, permission)?;
                live.insert(chat_id.to_string(), driver);
            }
            let activity = self.activity.lock().unwrap().get(chat_id).cloned();
            let was_busy = activity.as_ref().is_some_and(|a| {
                let mut a = a.lock().unwrap();
                let busy = a.busy;
                a.busy = true;
                a.last = Instant::now();
                busy
            });
            let result = live.get_mut(chat_id).unwrap().compact(instructions);
            if matches!(result, Ok(true)) {
                return Ok(());
            }
            if let Some(a) = activity {
                a.lock().unwrap().busy = was_busy;
            }
            result?;
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
        self.publisher(app, chat_id).publish(&event);
        self.hub.push(chat_id, &event);
        Ok(())
    }

    /// Ends the chat's agent (it starts again, resumed, on the next message).
    pub fn close(&self, chat_id: &str) {
        if let Some(mut d) = self.live.lock().unwrap().remove(chat_id) {
            d.stop();
        }
        self.activity.lock().unwrap().remove(chat_id);
        self.metas.lock().unwrap().remove(chat_id);
        self.hub.stopped(chat_id);
    }

    /// Persist before stopping; refuse active turns and agents without resume support.
    pub fn sleep(&self, app: &AppHandle, chat_id: &str, minimum: Duration) -> Result<(), String> {
        self.sleep_session(chat_id, minimum)?;
        self.changed(app);
        Ok(())
    }

    fn sleep_session(&self, chat_id: &str, minimum: Duration) -> Result<(), String> {
        let mut live = self.live.lock().unwrap();
        let Some(driver) = live.get(chat_id) else {
            self.edit(chat_id, |m| m.sleeping = true)?;
            return Ok(());
        };
        if !driver.idle() {
            return Err("Wait for the agent to finish before sleeping this chat.".into());
        }
        if !driver.resumable() {
            return Err("This agent does not support resuming saved sessions.".into());
        }
        let activity = self.activity.lock().unwrap().get(chat_id).cloned().ok_or("No session activity available")?;
        let mut activity = activity.lock().unwrap();
        if !activity.eligible(minimum) {
            return Err("This chat is active or has not been idle long enough.".into());
        }
        let mut metas = self.metas.lock().unwrap();
        let mut meta = self.store.get(chat_id).map_err(|e| e.to_string())?;
        if meta.agent_session_id.as_deref().is_none_or(str::is_empty) {
            return Err("The agent has not saved a resumable session yet.".into());
        }
        meta.sleeping = true;
        self.store.save(&meta).map_err(|e| e.to_string())?;
        activity.stopped = true;
        metas.remove(chat_id);
        drop(metas);
        drop(activity);
        if let Some(mut driver) = live.remove(chat_id) {
            driver.stop();
        }
        self.activity.lock().unwrap().remove(chat_id);
        self.metas.lock().unwrap().remove(chat_id);
        self.hub.stopped(chat_id);
        Ok(())
    }

    pub fn sleep_idle(&self, app: &AppHandle, minutes: u32) {
        if minutes == 0 {
            return;
        }
        for id in self.running() {
            let _ = self.sleep(app, &id, Duration::from_secs(u64::from(minutes) * 60));
        }
    }

    pub fn delete(&self, chat_id: &str) -> Result<(), String> {
        self.close(chat_id);
        self.hub.forget(chat_id);
        self.permissions.lock().unwrap().remove(chat_id);
        self.logs.lock().unwrap().remove(chat_id);
        self.store.delete(chat_id).map_err(|e| e.to_string())
    }

    pub fn running(&self) -> Vec<String> {
        self.live.lock().unwrap().keys().cloned().collect()
    }

    /// What each running chat waits on the user for: `approval`, `plan` (an approval of a plan)
    /// or `question`. Chats waiting for nothing aren't in it.
    pub fn waiting(&self) -> std::collections::BTreeMap<String, &'static str> {
        self.running()
            .into_iter()
            .filter_map(|id| {
                let s = self.hub.status(&id);
                let what = match s.approvals.first() {
                    Some(a) if a.kind == oah_agents::event::ApprovalKind::Plan => "plan",
                    Some(_) => "approval",
                    None if !s.questions.is_empty() => "question",
                    None => return None,
                };
                Some((id, what))
            })
            .collect()
    }

    /// Chats mid-turn right now.
    pub fn busy(&self) -> Vec<String> {
        self.live.lock().unwrap().iter().filter(|(_, d)| !d.idle()).map(|(k, _)| k.clone()).collect()
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

    /// What's held back so far, as whole events (without letting go of it).
    pub(crate) fn held(&self) -> Vec<ChatEvent> {
        self.held
            .iter()
            .map(|((item_id, reasoning), text)| {
                let (item_id, text) = (item_id.clone(), text.clone());
                if *reasoning {
                    ChatEvent::Reasoning { item_id, text }
                } else {
                    ChatEvent::Text { item_id, text }
                }
            })
            .collect()
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

    struct SleepDriver {
        store: ChatStore,
        id: String,
        stopped: Arc<std::sync::atomic::AtomicBool>,
        resumable: bool,
    }

    impl Driver for SleepDriver {
        fn send(&mut self, _: Prompt) -> Result<(), String> {
            Ok(())
        }
        fn interrupt(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn answer(&mut self, _: &str, _: Decision) -> Result<(), String> {
            Ok(())
        }
        fn resumable(&self) -> bool {
            self.resumable
        }
        fn stop(&mut self) {
            let meta = self.store.get(&self.id).unwrap();
            assert!(meta.sleeping);
            assert_eq!(meta.agent_session_id.as_deref(), Some("provider-session"));
            self.stopped.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[test]
    fn sleep_preserves_session_and_refuses_active_or_unsaved_chats() {
        let root = std::env::temp_dir().join(format!("oah-sleep-{}-{}", std::process::id(), now_ms()));
        let chats = Chats::new(ChatStore::new(root.clone()));
        let mut meta = chats.store.create("project", AgentKind::Claude, "model", "/work").unwrap();
        meta.agent_session_id = Some("provider-session".into());
        meta.effort = "high".into();
        chats.store.save(&meta).unwrap();
        let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let make_driver = |resumable| {
            Box::new(SleepDriver { store: chats.store.clone(), id: meta.id.clone(), stopped: stopped.clone(), resumable })
                as Box<dyn Driver>
        };
        chats.live.lock().unwrap().insert(meta.id.clone(), make_driver(false));
        let activity = Arc::new(Mutex::new(Activity { last: Instant::now(), busy: false, stopped: false }));
        chats.activity.lock().unwrap().insert(meta.id.clone(), activity.clone());
        assert!(chats.sleep_session(&meta.id, Duration::ZERO).is_err());
        chats.live.lock().unwrap().insert(meta.id.clone(), make_driver(true));
        let mut unsaved = meta.clone();
        unsaved.agent_session_id = None;
        chats.store.save(&unsaved).unwrap();
        assert!(chats.sleep_session(&meta.id, Duration::ZERO).is_err());
        chats.store.save(&meta).unwrap();
        activity.lock().unwrap().observe(&ChatEvent::UserMessage { id: "m".into(), text: "work".into(), images: vec![], files: vec![] });
        assert!(chats.sleep_session(&meta.id, Duration::ZERO).is_err());
        activity.lock().unwrap().observe(&ChatEvent::TurnFinished {
            turn_id: "t".into(),
            usage: None,
            cost_usd: None,
            duration_ms: None,
            interrupted: false,
        });
        assert!(chats.sleep_session(&meta.id, Duration::from_secs(60)).is_err());
        let tmp = root.join(&meta.id).join("meta.json.tmp");
        std::fs::create_dir(&tmp).unwrap();
        assert!(chats.sleep_session(&meta.id, Duration::ZERO).is_err());
        assert!(!stopped.load(std::sync::atomic::Ordering::SeqCst));
        std::fs::remove_dir(tmp).unwrap();
        activity.lock().unwrap().last = Instant::now() - Duration::from_secs(61);
        chats.sleep_session(&meta.id, Duration::from_secs(60)).unwrap();
        assert!(stopped.load(std::sync::atomic::Ordering::SeqCst));
        assert!(chats.running().is_empty());
        meta.sleeping = true;
        assert_eq!(chats.store.get(&meta.id).unwrap(), meta);
        assert!(activity.lock().unwrap().stopped);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn compacts_streamed_text() {
        let mut c = Compactor::default();
        let d = |t: &str| ChatEvent::TextDelta { item_id: "a".into(), text: t.into() };
        assert!(c.push(&d("Hel")).is_empty());
        assert_eq!(c.held(), vec![ChatEvent::Text { item_id: "a".into(), text: "Hel".into() }]);
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
