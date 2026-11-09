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

type Slot = Arc<Mutex<Box<dyn Driver>>>;

struct Activity {
    last: Instant,
    busy: bool,
    stopped: bool,
    /// Background tasks running, as the last `Tasks` said.
    tasks: usize,
    /// Questions asked as secrets (a password), by request: their answers are never shown or
    /// logged.
    secret: HashMap<String, Vec<String>>,
}

impl Activity {
    fn observe(&mut self, event: &ChatEvent) {
        self.last = Instant::now();
        match event {
            ChatEvent::UserMessage { .. } | ChatEvent::TurnStarted { .. } | ChatEvent::Approval { .. } | ChatEvent::Question { .. } => {
                self.busy = true
            }
            ChatEvent::TurnFinished { .. } | ChatEvent::Exited { .. } | ChatEvent::Error { .. } => self.busy = false,
            ChatEvent::Tasks { tasks } => self.tasks = tasks.iter().filter(|t| t.is_running() && t.tracked).count(),
            _ => {}
        }
        if let ChatEvent::Question { request } = event {
            let secret: Vec<String> = request.questions.iter().filter(|q| q.secret).map(|q| q.id.clone()).collect();
            if !secret.is_empty() {
                self.secret.insert(request.request_id.clone(), secret);
            }
        }
    }

    /// `event`, with the answers to secret questions masked. The agent already has them; the
    /// log, the window, phones and MCP clients get dots.
    fn redact(&mut self, event: ChatEvent) -> ChatEvent {
        match event {
            ChatEvent::QuestionResolved { request_id, mut answers } => {
                if let Some(ids) = self.secret.remove(&request_id) {
                    for id in ids {
                        if let Some(a) = answers.get_mut(&id) {
                            *a = a.iter().map(|_| "••••••".to_string()).collect();
                        }
                    }
                }
                ChatEvent::QuestionResolved { request_id, answers }
            }
            other => other,
        }
    }

    /// Asleep, the CLI would take its background work down with it.
    fn eligible(&self, minimum: Duration) -> bool {
        !self.stopped && !self.busy && self.tasks == 0 && self.last.elapsed() >= minimum
    }
}

pub struct Chats {
    pub store: ChatStore,
    /// What each running chat is doing, for MCP (`oah_mcp::Hub`); fed every event.
    pub hub: Arc<oah_mcp::Hub>,
    /// Every chat's text, for search.
    pub index: oah_mcp::search::Index,
    /// Each running chat's agent, behind its own lock: one chat waiting on its agent never
    /// holds up another. The map's lock is only ever held to look a chat up.
    live: Mutex<HashMap<String, Slot>>,
    /// Per chat, held through a send (which may start its agent, for seconds): two sends to one
    /// chat go one after the other, while other chats carry on.
    sending: Mutex<HashMap<String, Arc<Mutex<()>>>>,
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
            sending: Mutex::new(HashMap::new()),
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

    /// The mode a chat sends with: the one given (remembered from now on, in its meta too, so
    /// it outlives a restart), else the last one.
    fn permission(&self, chat_id: &str, given: Option<PermissionMode>) -> Option<PermissionMode> {
        let mut p = self.permissions.lock().unwrap();
        match given {
            Some(m) => {
                if p.insert(chat_id.to_string(), m) != Some(m) {
                    let _ = self.edit(chat_id, |meta| meta.permission = Some(m));
                }
                Some(m)
            }
            None => {
                let saved = p.get(chat_id).copied().or_else(|| self.store.get(chat_id).ok().and_then(|m| m.permission));
                if let Some(m) = saved {
                    p.insert(chat_id.to_string(), m);
                }
                saved
            }
        }
    }

    /// The mode the chat's next message runs with when none is given: its own, else its agent's
    /// default in Settings → Providers.
    pub fn permission_for(&self, settings: &Settings, chat_id: &str) -> PermissionMode {
        self.permission(chat_id, None)
            .or_else(|| self.store.get(chat_id).ok().map(|m| settings.providers.get(m.agent).permission_mode))
            .unwrap_or_default()
    }

    /// Chats left from a removed project whose folder is `project`'s (the same folder added
    /// again) move to it, with the rest of their old project's chats (its worktrees'); returns
    /// how many.
    pub fn adopt(&self, settings: &Settings, project: &oah_core::settings::Project) -> usize {
        let known: std::collections::HashSet<&str> = settings.projects.iter().map(|p| p.id.as_str()).collect();
        let same = |a: &str, b: &str| a.trim_end_matches(['/', '\\']) == b.trim_end_matches(['/', '\\']);
        let orphans: Vec<ChatMeta> = self.store.list().into_iter().filter(|c| !known.contains(c.project_id.as_str())).collect();
        let old: std::collections::HashSet<String> =
            orphans.iter().filter(|c| same(&c.cwd, &project.path)).map(|c| c.project_id.clone()).collect();
        let mut n = 0;
        for c in orphans.iter().filter(|c| old.contains(&c.project_id)) {
            if self.edit(&c.id, |m| m.project_id = project.id.clone()).is_ok() {
                n += 1;
            }
        }
        n
    }

    /// Every chat's events, numbered per chat, and list changes.
    pub fn feed(&self) -> broadcast::Receiver<Feed> {
        self.feed.subscribe()
    }

    /// The chat's running agent.
    fn driver(&self, chat_id: &str) -> Option<Slot> {
        self.live.lock().unwrap().get(chat_id).cloned()
    }

    fn add_driver(&self, chat_id: &str, driver: Box<dyn Driver>) -> Slot {
        let slot = Arc::new(Mutex::new(driver));
        self.live.lock().unwrap().insert(chat_id.to_string(), slot.clone());
        slot
    }

    /// Takes the chat's agent out of the running ones and stops it.
    fn stop_driver(&self, chat_id: &str) {
        let slot = self.live.lock().unwrap().remove(chat_id);
        if let Some(slot) = slot {
            slot.lock().unwrap().stop();
        }
    }

    fn send_gate(&self, chat_id: &str) -> Arc<Mutex<()>> {
        self.sending.lock().unwrap().entry(chat_id.to_string()).or_default().clone()
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
        // The agent before the log, never both: a driver reports (and takes the log) while
        // its own lock is held.
        let (live, busy) = match self.driver(chat_id) {
            Some(d) => (true, !d.lock().unwrap().idle()),
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
        // One send at a time for this chat; starting its agent holds up nothing else.
        let gate = self.send_gate(chat_id);
        let _gate = gate.lock().unwrap();
        // An effort the CLI only takes at start: restart it (resuming) now that it's idle.
        if self.restart.lock().unwrap().contains(chat_id) {
            let ready = self.driver(chat_id).is_some_and(|d| {
                let d = d.lock().unwrap();
                d.idle() && d.background() == 0
            });
            if ready {
                // The old agent's last words (its `exited`) mustn't land in the new one's turn.
                self.silence(chat_id);
                self.stop_driver(chat_id);
                self.restart.lock().unwrap().remove(chat_id);
            }
        }
        let slot = match self.driver(chat_id) {
            Some(slot) => slot,
            None => {
                let driver = self.start(app, settings, chat_id, permission)?;
                self.add_driver(chat_id, driver)
            }
        };
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
        let sent = slot.lock().unwrap().send(prompt.clone());
        match sent {
            // The CLI exited since the last message: start it again, resuming its session.
            Err(e) if e.contains("isn't running") => {
                self.live.lock().unwrap().remove(chat_id);
                let mut driver = self.start(app, settings, chat_id, permission)?;
                let r = driver.send(prompt);
                self.add_driver(chat_id, driver);
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
        let activity =
            Arc::new(Mutex::new(Activity { last: Instant::now(), busy: false, stopped: false, tasks: 0, secret: HashMap::new() }));
        self.activity.lock().unwrap().insert(chat_id.to_string(), activity.clone());
        let sink: driver::Sink = Arc::new(move |event: ChatEvent| {
            let mut activity = activity.lock().unwrap();
            if activity.stopped {
                return;
            }
            let tasks_before = activity.tasks;
            activity.observe(&event);
            let event = activity.redact(event);
            // Background work starting or ending changes the list too.
            let tasks_changed = activity.tasks != tasks_before;
            publisher.publish(&event);
            fold(&publisher.store, &metas, &publisher.id, &event);
            hub.push(&publisher.id, &event);
            if let ChatEvent::Exited { message: Some(msg), .. } = &event {
                if session_gone(msg) {
                    let note = ChatEvent::Notice {
                        level: NoticeLevel::Warn,
                        text: "The agent no longer has this chat's earlier session. Send your message again: it continues in a new session, which doesn't remember what's above.".into(),
                    };
                    publisher.publish(&note);
                    hub.push(&publisher.id, &note);
                }
            }
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
            ) || tasks_changed
            {
                let _ = publisher.app.emit("chats-changed", ());
                let _ = feed.send(Feed::ChatsChanged);
            }
        });
        let resumed = opts.resume.is_some();
        match driver::start(opts.clone(), sink.clone()) {
            // The CLI no longer has the saved session (cleaned up, another profile, a new
            // machine): carry on in a new one rather than failing every message from now on.
            Err(e) if resumed && session_gone(&e) => {
                self.forget_session(chat_id, &e, &sink);
                driver::start(StartOptions { resume: None, ..opts }, sink)
            }
            r => r,
        }
    }

    /// The chat's saved CLI session is gone: the next start is a new session, and the chat says
    /// so (the agent won't remember what's above).
    fn forget_session(&self, chat_id: &str, why: &str, sink: &driver::Sink) {
        let _ = self.edit(chat_id, |m| m.agent_session_id = None);
        let why = why.lines().find(|l| !l.trim().is_empty()).unwrap_or(why).trim();
        sink(ChatEvent::Notice {
            level: NoticeLevel::Warn,
            text: format!("The agent no longer has this chat's earlier session ({why}). It continues in a new one, which doesn't remember what's above."),
        });
    }

    /// Stops one of the chat's background tasks.
    pub fn stop_task(&self, chat_id: &str, task_id: &str) -> Result<(), String> {
        match self.driver(chat_id) {
            Some(d) => d.lock().unwrap().stop_task(task_id),
            None => Err("The agent isn't running; its background work stopped with it.".into()),
        }
    }

    /// Chats with background work running (between turns, too).
    pub fn background(&self) -> Vec<String> {
        let slots: Vec<(String, Slot)> = self.live.lock().unwrap().iter().map(|(k, d)| (k.clone(), d.clone())).collect();
        slots.into_iter().filter(|(_, d)| d.lock().unwrap().background() > 0).map(|(k, _)| k).collect()
    }

    pub fn interrupt(&self, chat_id: &str) -> Result<(), String> {
        match self.driver(chat_id) {
            Some(d) => d.lock().unwrap().interrupt(),
            None => Ok(()),
        }
    }

    pub fn answer(&self, chat_id: &str, request_id: &str, decision: Decision) -> Result<(), String> {
        match self.driver(chat_id) {
            Some(d) => d.lock().unwrap().answer(request_id, decision),
            None => Err("this chat's agent isn't running anymore".into()),
        }
    }

    pub fn answer_question(
        &self,
        chat_id: &str,
        request_id: &str,
        answers: std::collections::BTreeMap<String, Vec<String>>,
    ) -> Result<(), String> {
        match self.driver(chat_id) {
            Some(d) => d.lock().unwrap().answer_question(request_id, answers),
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
        if let Some(d) = self.driver(chat_id) {
            d.lock().unwrap().set_model(model)?;
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
        if let Some(d) = self.driver(chat_id) {
            if !d.lock().unwrap().set_effort(effort)? {
                self.restart.lock().unwrap().insert(chat_id.to_string());
            }
        }
        Ok(meta)
    }

    /// Switches a chat's permission mode: remembered for its next messages, applied now if its
    /// agent runs. `announce` tells the UI (and logs it) when the change came from elsewhere (MCP).
    pub fn set_permission(&self, app: &AppHandle, chat_id: &str, mode: PermissionMode, announce: bool) -> Result<(), String> {
        self.permission(chat_id, Some(mode));
        if let Some(d) = self.driver(chat_id) {
            d.lock().unwrap().set_permission(mode)?;
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
            let gate = self.send_gate(chat_id);
            let _gate = gate.lock().unwrap();
            let slot = match self.driver(chat_id) {
                Some(slot) => slot,
                None => {
                    let driver = self.start(app, settings, chat_id, permission)?;
                    self.add_driver(chat_id, driver)
                }
            };
            let activity = self.activity.lock().unwrap().get(chat_id).cloned();
            let was_busy = activity.as_ref().is_some_and(|a| {
                let mut a = a.lock().unwrap();
                let busy = a.busy;
                a.busy = true;
                a.last = Instant::now();
                busy
            });
            let result = slot.lock().unwrap().compact(instructions);
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

    /// Whatever the chat's current agent still reports from now on is dropped (it's being
    /// stopped for good, or replaced by a new one).
    fn silence(&self, chat_id: &str) {
        if let Some(a) = self.activity.lock().unwrap().get(chat_id) {
            a.lock().unwrap().stopped = true;
        }
    }

    /// Ends the chat's agent (it starts again, resumed, on the next message).
    pub fn close(&self, chat_id: &str) {
        self.stop_driver(chat_id);
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
        // No message can start a turn while this decides.
        let gate = self.send_gate(chat_id);
        let _gate = gate.lock().unwrap();
        let Some(slot) = self.driver(chat_id) else {
            self.edit(chat_id, |m| m.sleeping = true)?;
            return Ok(());
        };
        {
            let driver = slot.lock().unwrap();
            if !driver.idle() {
                return Err("Wait for the agent to finish before sleeping this chat.".into());
            }
            if driver.background() > 0 {
                return Err("This chat has background work running; sleeping it would stop that.".into());
            }
            if !driver.resumable() {
                return Err("This agent does not support resuming saved sessions.".into());
            }
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
        self.stop_driver(chat_id);
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
        // Its agent reports `exited` after it's stopped; that mustn't bring the chat's folder back.
        self.silence(chat_id);
        self.close(chat_id);
        self.restart.lock().unwrap().remove(chat_id);
        self.hub.forget(chat_id);
        self.permissions.lock().unwrap().remove(chat_id);
        self.logs.lock().unwrap().remove(chat_id);
        self.sending.lock().unwrap().remove(chat_id);
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
        let slots: Vec<(String, Slot)> = self.live.lock().unwrap().iter().map(|(k, d)| (k.clone(), d.clone())).collect();
        slots.into_iter().filter(|(_, d)| !d.lock().unwrap().idle()).map(|(k, _)| k).collect()
    }

    /// Stops every agent (the app is quitting or updating), after writing out what each chat
    /// was still streaming, so the half-finished answer is there when it's reopened.
    pub fn stop_all(&self) {
        let logs: Vec<(String, Arc<Mutex<Log>>)> = self.logs.lock().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for (id, log) in logs {
            for e in log.lock().unwrap().compactor.flush() {
                let _ = self.store.append(&id, &e);
            }
        }
        let ids: Vec<String> = self.running();
        for id in &ids {
            self.silence(id);
        }
        let all: Vec<Slot> = self.live.lock().unwrap().drain().map(|(_, d)| d).collect();
        for d in all {
            d.lock().unwrap().stop();
        }
    }
}

/// Whether a CLI's error says the session it was asked to resume doesn't exist (as opposed to
/// a timeout or a crash, after which it may still be there).
fn session_gone(e: &str) -> bool {
    let e = e.to_ascii_lowercase();
    ["no conversation found", "no rollout found", "thread not found", "session not found", "no such session", "unknown session"]
        .iter()
        .any(|p| e.contains(p))
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
        // Claude Code says so on its way out: start the next one fresh, not with the same id.
        ChatEvent::Exited { message: Some(msg), .. } if m.agent_session_id.is_some() && session_gone(msg) => {
            m.agent_session_id = None;
            true
        }
        ChatEvent::TurnFinished { usage, cost_usd, .. } => {
            m.tokens += usage.map(|u| u.fresh()).unwrap_or(0);
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

/// What goes in the chat's log: streamed text and reasoning are held back and written as a
/// whole `Text` / `Reasoning` event when anything else happens. The UI still gets every delta
/// live; the log stays about one line per message instead of one per fragment.
///
/// A message can be split by other events (a task update, the context meter, a steered
/// message) and go on streaming after them. Its text is kept for the whole turn, and each
/// write is the message so far, so the last one written (which replaces the earlier ones when
/// the chat is reopened) is the whole message, not just the part after the split.
#[derive(Default)]
pub(crate) struct Compactor {
    /// In the order the items started.
    held: Vec<Held>,
}

struct Held {
    item_id: String,
    reasoning: bool,
    text: String,
    /// Changed since it was last written.
    dirty: bool,
}

impl Held {
    fn event(&self) -> ChatEvent {
        let (item_id, text) = (self.item_id.clone(), self.text.clone());
        if self.reasoning {
            ChatEvent::Reasoning { item_id, text }
        } else {
            ChatEvent::Text { item_id, text }
        }
    }
}

impl Compactor {
    pub(crate) fn push(&mut self, e: &ChatEvent) -> Vec<ChatEvent> {
        let (item_id, reasoning, text, replace) = match e {
            ChatEvent::TextDelta { item_id, text } => (item_id, false, text, false),
            ChatEvent::ReasoningDelta { item_id, text } => (item_id, true, text, false),
            ChatEvent::Text { item_id, text } => (item_id, false, text, true),
            ChatEvent::Reasoning { item_id, text } => (item_id, true, text, true),
            other => {
                let mut out = self.flush();
                // A turn's messages don't go on after it.
                if matches!(other, ChatEvent::TurnFinished { .. } | ChatEvent::Exited { .. }) {
                    self.held.clear();
                }
                out.push(other.clone());
                return out;
            }
        };
        match self.held.iter_mut().find(|h| h.item_id == *item_id && h.reasoning == reasoning) {
            Some(h) if replace => h.text = text.clone(),
            Some(h) => h.text.push_str(text),
            None => self.held.push(Held { item_id: item_id.clone(), reasoning, text: text.clone(), dirty: true }),
        }
        if let Some(h) = self.held.iter_mut().find(|h| h.item_id == *item_id && h.reasoning == reasoning) {
            h.dirty = true;
        }
        Vec::new()
    }

    /// What's held back and not yet written, as whole events (without letting go of it).
    pub(crate) fn held(&self) -> Vec<ChatEvent> {
        self.held.iter().filter(|h| h.dirty).map(Held::event).collect()
    }

    /// Writes what changed: each such message whole, so far.
    pub(crate) fn flush(&mut self) -> Vec<ChatEvent> {
        self.held
            .iter_mut()
            .filter(|h| h.dirty)
            .map(|h| {
                h.dirty = false;
                h.event()
            })
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
        chats.add_driver(&meta.id, make_driver(false));
        let activity =
            Arc::new(Mutex::new(Activity { last: Instant::now(), busy: false, stopped: false, tasks: 0, secret: HashMap::new() }));
        chats.activity.lock().unwrap().insert(meta.id.clone(), activity.clone());
        assert!(chats.sleep_session(&meta.id, Duration::ZERO).is_err());
        chats.add_driver(&meta.id, make_driver(true));
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
        // A save that fails leaves the agent running.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = root.join(&meta.id);
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
            let refused = chats.sleep_session(&meta.id, Duration::ZERO);
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(refused.is_err());
            assert!(!stopped.load(std::sync::atomic::Ordering::SeqCst));
        }
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
    fn secret_answers_are_masked_everywhere_after_the_agent() {
        use oah_agents::event::{Question, QuestionRequest};
        let mut a = Activity { last: Instant::now(), busy: false, stopped: false, tasks: 0, secret: HashMap::new() };
        let q = |id: &str, secret| Question {
            id: id.into(),
            header: String::new(),
            question: "?".into(),
            options: vec![],
            multi: false,
            other: true,
            secret,
        };
        a.observe(&ChatEvent::Question {
            request: QuestionRequest { request_id: "r".into(), questions: vec![q("user", false), q("pw", true)] },
        });
        let answers = [("user".to_string(), vec!["me".to_string()]), ("pw".to_string(), vec!["hunter2".to_string()])].into_iter().collect();
        let out = a.redact(ChatEvent::QuestionResolved { request_id: "r".into(), answers });
        let text = serde_json::to_string(&out).unwrap();
        assert!(!text.contains("hunter2") && text.contains("me"), "{text}");
    }

    #[test]
    fn tells_a_missing_session_from_a_failure() {
        assert!(session_gone("No conversation found with session ID: 1234"));
        assert!(session_gone("thread/resume failed: no rollout found for thread id abc"));
        assert!(!session_gone("codex didn't answer thread/resume"));
        assert!(!session_gone("Claude Code exited"));
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

    /// A message split by another event and streaming on after it is written whole the second
    /// time, so reopening the chat (which keeps the last `text` for an item) shows all of it.
    #[test]
    fn a_split_message_is_written_whole() {
        let mut c = Compactor::default();
        let d = |t: &str| ChatEvent::TextDelta { item_id: "a".into(), text: t.into() };
        c.push(&d("The answer "));
        let split = c.push(&ChatEvent::Tasks { tasks: vec![] });
        assert_eq!(split[0], ChatEvent::Text { item_id: "a".into(), text: "The answer ".into() });
        c.push(&d("is 42."));
        assert_eq!(c.held(), vec![ChatEvent::Text { item_id: "a".into(), text: "The answer is 42.".into() }]);
        let end =
            c.push(&ChatEvent::TurnFinished { turn_id: "t".into(), usage: None, cost_usd: None, duration_ms: None, interrupted: false });
        assert_eq!(end[0], ChatEvent::Text { item_id: "a".into(), text: "The answer is 42.".into() });
        // Unchanged items aren't written again, and the next turn starts clean.
        assert_eq!(c.push(&ChatEvent::TurnStarted { turn_id: "u".into() }).len(), 1);
        c.push(&d("New"));
        assert_eq!(c.held(), vec![ChatEvent::Text { item_id: "a".into(), text: "New".into() }]);
    }
}
