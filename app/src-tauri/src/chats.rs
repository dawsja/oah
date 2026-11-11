//! Chats at runtime: one agent driver per open chat. Every event a driver reports is appended to
//! the chat's log, folded into its meta (title, CLI session id, usage), emitted to the UI as
//! `chat-event` `{ chatId, event }` and fed, numbered, to paired phones ([`Chats::feed`]).
//! [`Chats::snapshot`] is a chat as it is right now, mid-turn included, lined up with that
//! numbering.
//!
//! A chat's session has one owner at a time: its driver, or the agent's own CLI in a terminal
//! in the window ([`Chats::cli_open`]). While the CLI has it nothing is sent to the chat (from
//! the window, a phone or MCP); when the CLI ends, the next message resumes the session with
//! what was said there.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oah_agents::cli;
use oah_agents::driver::{self, Driver, Prompt, StartOptions};
use oah_agents::event::{ChatEvent, Decision, NoticeLevel};
use oah_core::chats::{now_ms, title_from, ChatMeta, ChatStore};
use oah_core::settings::{AgentKind, Location, PermissionMode, Settings};
use oah_remote::server::Feed;
use oah_transport::pty::{self, Pty, PtyEvent};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
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

/// A chat open in its agent's own CLI, in a terminal in the window. The CLI has the session, so
/// the chat has no driver until it ends.
struct CliRun {
    pty: Pty,
    /// Which start this is: a late report from an earlier one isn't about this one.
    run: u64,
    started: Instant,
    /// The conversation it was opened on: the chat's, or a new one under an id oah picked.
    session: String,
    /// That id was oah's pick (the chat had no session): the chat's from now on, if the CLI
    /// saved a conversation under it.
    preset: bool,
    /// Where the conversation stood as it opened, to read back what's said in it.
    mark: cli::Mark,
    /// The note it keeps of the conversations it's on, where it can.
    watch: Option<cli::Watch>,
    /// Enter was pressed in it: something may have been said there.
    used: bool,
    /// It has ended, and what it left is being read back: the chat is its driver's again
    /// once that's in the chat's log.
    ending: bool,
}

/// What of a [`CliRun`] is needed once it's over.
struct CliLeft {
    started: Instant,
    session: String,
    preset: bool,
    mark: cli::Mark,
    watch: Option<cli::Watch>,
    used: bool,
}

/// How long a message waits for a CLI that just ended to be read back (over SSH, a while).
const CLI_READ_BACK: Duration = Duration::from_secs(90);

/// Why nothing can be sent to a chat whose CLI is open.
pub const IN_CLI: &str =
    "This chat is open in its agent's own CLI on the computer, which has the conversation until it's closed there or taken back.";

/// Why a chat's CLI doesn't open over a working agent unless asked to.
pub const CLI_BUSY: &str = "The agent is working in this chat. Opening its CLI stops that.";

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
    /// Chats open in their agent's CLI. A chat is here or in `live`, never both: both are
    /// changed under the chat's send gate.
    cli: Mutex<HashMap<String, CliRun>>,
    cli_runs: AtomicU64,
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
struct Publisher<R: Runtime> {
    app: AppHandle<R>,
    store: ChatStore,
    feed: broadcast::Sender<Feed>,
    log: Arc<Mutex<Log>>,
    id: String,
}

impl<R: Runtime> Clone for Publisher<R> {
    fn clone(&self) -> Self {
        Publisher { app: self.app.clone(), store: self.store.clone(), feed: self.feed.clone(), log: self.log.clone(), id: self.id.clone() }
    }
}

impl<R: Runtime> Publisher<R> {
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
            cli: Mutex::new(HashMap::new()),
            cli_runs: AtomicU64::new(0),
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

    fn publisher<R: Runtime>(&self, app: &AppHandle<R>, chat_id: &str) -> Publisher<R> {
        Publisher { app: app.clone(), store: self.store.clone(), feed: self.feed.clone(), log: self.log(chat_id), id: chat_id.to_string() }
    }

    /// The chat list changed (a chat was made, renamed, deleted or finished a turn): the page
    /// and phones list it again.
    pub fn changed<R: Runtime>(&self, app: &AppHandle<R>) {
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
    pub fn send<R: Runtime>(
        &self,
        app: &AppHandle<R>,
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
        self.free(chat_id)?;
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

    fn start<R: Runtime>(
        &self,
        app: &AppHandle<R>,
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
    pub fn set_permission<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        chat_id: &str,
        mode: PermissionMode,
        announce: bool,
    ) -> Result<(), String> {
        self.permission(chat_id, Some(mode));
        if let Some(d) = self.driver(chat_id) {
            let mut d = d.lock().unwrap();
            if d.takes_permission(mode) {
                d.set_permission(mode)?;
            } else {
                // Restarted (resuming) with it before the next message.
                self.restart.lock().unwrap().insert(chat_id.to_string());
            }
        }
        if announce {
            let event = ChatEvent::PermissionChanged { mode };
            self.publisher(app, chat_id).publish(&event);
            self.hub.push(chat_id, &event);
        }
        Ok(())
    }

    /// `/compact`: the agent's own way when it has one, else `/compact` as a message.
    pub fn compact<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        settings: &Settings,
        chat_id: &str,
        instructions: &str,
        permission: Option<PermissionMode>,
    ) -> Result<(), String> {
        let permission = self.permission(chat_id, permission);
        {
            let gate = self.send_gate(chat_id);
            let _gate = gate.lock().unwrap();
            self.free(chat_id)?;
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
    pub fn clear<R: Runtime>(&self, app: &AppHandle<R>, chat_id: &str) -> Result<(), String> {
        // The CLI open on it would carry on in the session this lets go of.
        self.free(chat_id)?;
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

    /// Ends the chat's agent (it starts again, resumed, on the next message), and its CLI if
    /// that's what has the session.
    pub fn close(&self, chat_id: &str) {
        // Dropped, the terminal ends its program; its exit then finds nothing to report on.
        let cli = self.cli.lock().unwrap().remove(chat_id);
        drop(cli);
        self.stop_driver(chat_id);
        self.activity.lock().unwrap().remove(chat_id);
        self.metas.lock().unwrap().remove(chat_id);
        self.hub.stopped(chat_id);
    }

    /// Persist before stopping; refuse active turns and agents without resume support.
    pub fn sleep<R: Runtime>(&self, app: &AppHandle<R>, chat_id: &str, minimum: Duration) -> Result<(), String> {
        self.sleep_session(chat_id, minimum)?;
        self.changed(app);
        Ok(())
    }

    fn sleep_session(&self, chat_id: &str, minimum: Duration) -> Result<(), String> {
        // No message can start a turn while this decides.
        let gate = self.send_gate(chat_id);
        let _gate = gate.lock().unwrap();
        self.free(chat_id)?;
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

    pub fn sleep_idle<R: Runtime>(&self, app: &AppHandle<R>, minutes: u32) {
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
        // The CLIs end with their terminals. A session one started keeps its chat when
        // something was said in it (there's no time left to look whether it was saved).
        let clis: Vec<(String, CliRun)> = self.cli.lock().unwrap().drain().collect();
        for (id, run) in clis {
            if run.preset && run.used {
                let _ = self.edit(&id, |m| m.agent_session_id = m.agent_session_id.clone().or(Some(run.session.clone())));
            }
        }
    }

    // ---- The agent's own CLI ---------------------------------------------------------------

    /// `Err` while the chat's CLI has its session: nothing is sent to it meanwhile. A CLI that
    /// has just ended is waited for: what was said in it is in the chat's log before the next
    /// thing said is.
    fn free(&self, chat_id: &str) -> Result<(), String> {
        let until = Instant::now() + CLI_READ_BACK;
        loop {
            match self.cli.lock().unwrap().get(chat_id) {
                None => return Ok(()),
                Some(c) if !c.ending || Instant::now() >= until => return Err(IN_CLI.into()),
                Some(_) => {}
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Chats open in their agent's CLI.
    pub fn cli_ids(&self) -> Vec<String> {
        self.cli.lock().unwrap().keys().cloned().collect()
    }

    /// Opens the chat's session in its agent's own CLI, in a `cols`×`rows` terminal whose
    /// output goes to `output` (which answers false once nobody is there to see it: the CLI
    /// ends). The chat's driver stops first, handing the session over; one that's working
    /// (mid-turn, or with background work) is only stopped with `force`, else this is
    /// [`CLI_BUSY`]. A chat with no session yet starts one under an id oah picks, where the
    /// CLI can ([`cli::presets_session`]).
    #[allow(clippy::too_many_arguments)]
    pub fn cli_open<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        settings: &Settings,
        chat_id: &str,
        cols: u16,
        rows: u16,
        force: bool,
        output: impl Fn(PtyEvent) -> bool + Send + Sync + 'static,
    ) -> Result<(), String> {
        // No message starts the chat's agent while the session changes hands.
        let gate = self.send_gate(chat_id);
        let _gate = gate.lock().unwrap();
        // Opened again (the page was reloaded under it): the earlier terminal has no screen.
        self.cli_close(app, chat_id);
        let meta = self.store.get(chat_id).map_err(|e| e.to_string())?;
        let project = settings.projects.iter().find(|p| p.id == meta.project_id).ok_or("this chat's project was removed")?;
        let provider = settings.providers.get(meta.agent);
        if !provider.enabled {
            return Err("this agent is turned off in Settings → Providers".into());
        }
        let session = match meta.agent_session_id.clone().filter(|s| !s.is_empty()) {
            Some(id) => cli::Session::Resume(id),
            None if cli::presets_session(meta.agent) => cli::Session::New(uuid::Uuid::new_v4().to_string()),
            None => {
                return Err(format!(
                    "Send a first message here, then open the CLI: {} names its sessions itself, so oah can only open one that exists.",
                    meta.agent.label()
                ))
            }
        };
        if let Location::Wsl { distro } = &project.location {
            oah_transport::wsl::start(distro)?;
        }
        let permission = self.permission(chat_id, None).unwrap_or(provider.permission_mode);
        let model = Some(meta.model.clone())
            .filter(|m| !m.is_empty())
            .or_else(|| Some(project.default_model.clone()).filter(|m| !m.is_empty()))
            .or_else(|| Some(provider.default_model.clone()).filter(|m| !m.is_empty()));
        let mut env = crate::secrets::resolve(&crate::secrets::scope(meta.agent), &provider.env);
        cli::env(meta.agent, permission, &mut env);
        let binary = provider.binary_on(&project.location);
        let program = if binary.trim().is_empty() { meta.agent.default_binary() } else { binary.trim() };
        let cwd = if meta.cwd.is_empty() { project.path.clone() } else { meta.cwd.clone() };
        let target = cli::Target { kind: meta.agent, location: &project.location, cwd: &cwd, binary: &binary, env: &env };
        let opts = cli::CliOptions {
            kind: meta.agent,
            session: session.clone(),
            model,
            effort: Some(meta.effort.clone()).filter(|e| !e.is_empty()),
            permission,
            alone: cli::runs_alone(&target),
        };
        // The CLI notes the conversations it's on, where it can: a `/clear` there is followed.
        let watch = cli::watch(meta.agent, &project.location, &uuid::Uuid::new_v4().simple().to_string());
        let mut args = cli::args(&opts);
        args.extend(watch.iter().flat_map(|w| w.args.iter().cloned()));
        let spec = pty::command_spec(&oah_transport::Spec { location: &project.location, cwd: &cwd, program, args, env: &env })?;
        // Everything that can be refused is, before the chat's agent is touched.
        if let Some(slot) = self.driver(chat_id) {
            let working = || {
                let d = slot.lock().unwrap();
                !d.idle() || d.background() > 0
            };
            if working() {
                if !force {
                    return Err(CLI_BUSY.into());
                }
                // Its turn ends as a stopped one, when it stops in time.
                let _ = slot.lock().unwrap().interrupt();
                let until = Instant::now() + Duration::from_secs(3);
                while Instant::now() < until && !slot.lock().unwrap().idle() {
                    std::thread::sleep(Duration::from_millis(40));
                }
            }
            // What it says from here on isn't the chat's; the page stops waiting on it (a turn
            // cut short, a card unanswered), and what it had streamed is written out.
            self.silence(chat_id);
            self.close(chat_id);
            self.publisher(app, chat_id).publish(&ChatEvent::Exited { code: None, message: None });
            self.changed(app);
        }
        // Where the conversation stands, now that its agent has written all it had to.
        let (opened_on, preset) = match &session {
            cli::Session::Resume(id) => (id.clone(), false),
            cli::Session::New(id) => (id.clone(), true),
        };
        let mark = cli::mark(&target, (!preset).then_some(opened_on.as_str()), now_ms());
        let run = self.cli_runs.fetch_add(1, Ordering::Relaxed) + 1;
        // Held until the CLI is in the map, so an instant exit isn't reported before it is.
        let mut open = self.cli.lock().unwrap();
        let (handle, id) = (app.clone(), chat_id.to_string());
        let pty = Pty::spawn(&spec, cols, rows, move |e| {
            let state = handle.state::<crate::AppState>();
            match e {
                // The chat is free again before the page hears of the exit.
                PtyEvent::Exit(_) => {
                    state.chats.cli_ended(&handle, &id, run);
                    output(e);
                }
                e => {
                    if !output(e) {
                        state.chats.cli_kill(&id, run);
                    }
                }
            }
        })
        .map_err(|e| format!("couldn't open {}'s CLI: {e}", meta.agent.label()))?;
        let cli = CliRun { pty, run, started: Instant::now(), session: opened_on, preset, mark, watch, used: false, ending: false };
        open.insert(chat_id.to_string(), cli);
        drop(open);
        let _ = self.edit(chat_id, |m| m.sleeping = false);
        self.changed(app);
        Ok(())
    }

    /// Keys typed (or text pasted) into the chat's CLI.
    pub fn cli_write(&self, chat_id: &str, data: &str) -> Result<(), String> {
        let mut open = self.cli.lock().unwrap();
        let c = open.get_mut(chat_id).filter(|c| !c.ending).ok_or("this chat's CLI isn't open")?;
        // The terminal's own answers (where the cursor is, focus) never have one.
        c.used |= data.contains(['\r', '\n']);
        c.pty.write(data.as_bytes())
    }

    pub fn cli_resize(&self, chat_id: &str, cols: u16, rows: u16) -> Result<(), String> {
        match self.cli.lock().unwrap().get(chat_id) {
            Some(c) => c.pty.resize(cols, rows),
            None => Ok(()),
        }
    }

    /// Ends the chat's CLI and waits until the chat is its driver's again: the CLI gone (a
    /// few seconds at most), and what was said in it read back into the chat.
    pub fn cli_close<R: Runtime>(&self, app: &AppHandle<R>, chat_id: &str) {
        let run = match self.cli.lock().unwrap().get(chat_id) {
            Some(c) => {
                c.pty.kill();
                c.run
            }
            None => return,
        };
        let (gone, read) = (Instant::now() + Duration::from_secs(3), Instant::now() + CLI_READ_BACK);
        loop {
            match self.cli.lock().unwrap().get(chat_id).filter(|c| c.run == run).map(|c| c.ending) {
                None => return,
                // It hasn't gone in time: the chat is let go all the same.
                Some(false) if Instant::now() >= gone => break,
                Some(true) if Instant::now() >= read => return,
                Some(_) => {}
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        self.cli_ended(app, chat_id, run);
    }

    /// Takes the chat back from its CLI from outside the window (a phone, MCP): the CLI ends,
    /// and the window is told so its terminal goes with it. False when no CLI had the chat.
    pub fn cli_take<R: Runtime>(&self, app: &AppHandle<R>, chat_id: &str) -> bool {
        if !self.cli.lock().unwrap().contains_key(chat_id) {
            return false;
        }
        let _ = app.emit("chat-cli-taken", chat_id);
        self.cli_close(app, chat_id);
        true
    }

    /// Ends start `run` of the chat's CLI (nobody sees its output any more).
    fn cli_kill(&self, chat_id: &str, run: u64) {
        if let Some(c) = self.cli.lock().unwrap().get(chat_id).filter(|c| c.run == run) {
            c.pty.kill();
        }
    }

    /// Start `run` of the chat's CLI ended. What was said in it is read back into the chat,
    /// a session it started (or moved to) becomes the chat's, and then the chat is its
    /// driver's again: until then it still counts as the CLI's, so nothing said next lands
    /// before what was said there.
    fn cli_ended<R: Runtime>(&self, app: &AppHandle<R>, chat_id: &str, run: u64) {
        let ended = match self.cli.lock().unwrap().get_mut(chat_id) {
            Some(c) if c.run == run && !c.ending => {
                c.ending = true;
                CliLeft {
                    started: c.started,
                    session: c.session.clone(),
                    preset: c.preset,
                    mark: c.mark.clone(),
                    watch: c.watch.clone(),
                    used: c.used,
                }
            }
            _ => return,
        };
        self.cli_read_back(app, chat_id, &ended);
        {
            let mut open = self.cli.lock().unwrap();
            if open.get(chat_id).is_some_and(|c| c.run == run) {
                open.remove(chat_id);
            }
        }
        self.changed(app);
    }

    fn cli_read_back<R: Runtime>(&self, app: &AppHandle<R>, chat_id: &str, ended: &CliLeft) {
        let Ok(meta) = self.store.get(chat_id) else { return };
        let settings = app.state::<crate::AppState>().settings.lock().unwrap().clone();
        let Some(project) = settings.projects.iter().find(|p| p.id == meta.project_id) else { return };
        let provider = settings.providers.get(meta.agent);
        let mut env = crate::secrets::resolve(&crate::secrets::scope(meta.agent), &provider.env);
        cli::env(meta.agent, PermissionMode::FullAccess, &mut env);
        let binary = provider.binary_on(&project.location);
        let cwd = if meta.cwd.is_empty() { project.path.clone() } else { meta.cwd.clone() };
        let target = cli::Target { kind: meta.agent, location: &project.location, cwd: &cwd, binary: &binary, env: &env };
        // What it saved of the conversation, and where it left off.
        let left = cli::outcome(&target, &ended.session, &ended.mark, ended.watch.as_ref());
        let events = left.events.clone().unwrap_or_default();
        // Something was said there: it shows, or (where it can't be read back) Enter was
        // pressed. A session the CLI started is the chat's then, unless the CLI is known
        // not to have saved it; one opened and left unused isn't, even where the CLI made an
        // empty one.
        let saved = || cli::session_saved(meta.agent, &project.location, &env, &ended.session);
        let said = !events.is_empty() || (left.events.is_none() && ended.used && (!ended.preset || saved() != Some(false)));
        let session = left.moved_to.clone().or_else(|| (ended.preset && said).then(|| ended.session.clone()));
        if let Some(session) = session {
            let moved = left.moved_to.is_some();
            let _ = self.edit(chat_id, |m| {
                if moved || m.agent_session_id.is_none() {
                    m.agent_session_id = Some(session.clone());
                }
            });
        }
        let publish = |event: &ChatEvent| {
            self.publisher(app, chat_id).publish(event);
            self.hub.push(chat_id, event);
        };
        if said {
            publish(&ChatEvent::Notice {
                level: NoticeLevel::Info,
                text: cli_note(meta.agent, ended.started.elapsed(), !events.is_empty()),
            });
            events.iter().for_each(&publish);
            // A chat begun in the CLI is named after the first thing said there, when that's known.
            let first = events.iter().find_map(|e| match e {
                ChatEvent::UserMessage { text, .. } => Some(title_from(text)),
                _ => None,
            });
            let name = first.unwrap_or_else(|| format!("{} CLI session", meta.agent.label()));
            let _ = self.edit(chat_id, |m| {
                m.updated_at = now_ms();
                if m.title.is_empty() {
                    m.title = name.clone();
                }
            });
        }
        if left.moved_to.is_some() {
            publish(&ChatEvent::Notice {
                level: NoticeLevel::Info,
                text: "In the CLI the conversation changed (a /clear or a /resume there). This chat continues the one the CLI ended on."
                    .into(),
            });
        }
    }
}

/// The line a chat gets after time in its agent's CLI.
fn cli_note(agent: AgentKind, spent: Duration, shown: bool) -> String {
    let s = spent.as_secs();
    let minutes = (s + 30) / 60;
    let time = match s {
        0..=89 => format!("{} s", s.max(1)),
        _ if minutes < 60 => format!("{minutes} min"),
        _ => format!("{} h {} min", minutes / 60, minutes % 60),
    };
    let rest = if shown { "What was said there follows." } else { "What was said there isn't shown here; the agent remembers it." };
    format!("Continued in {}'s own CLI for {time}. {rest}", agent.label())
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

    /// A chat whose CLI is open takes nothing (a message would start a second owner of its
    /// session) until the CLI is gone.
    #[test]
    #[cfg(unix)]
    fn a_chat_in_its_cli_takes_nothing_until_the_cli_ends() {
        let root = std::env::temp_dir().join(format!("oah-cli-{}-{}", std::process::id(), now_ms()));
        let chats = Chats::new(ChatStore::new(root.clone()));
        let meta = chats.store.create("project", AgentKind::Claude, "", "/work").unwrap();
        let other = chats.store.create("project", AgentKind::Claude, "", "/work").unwrap();
        let open = |id: &str, preset: &str| {
            let spec = pty::ShellSpec {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), "sleep 30".into()],
                cwd: None,
                env: vec![],
                shell: None,
            };
            let pty = Pty::spawn(&spec, 80, 24, |_| {}).unwrap();
            chats.cli.lock().unwrap().insert(
                id.to_string(),
                CliRun {
                    pty,
                    run: 1,
                    started: Instant::now(),
                    session: preset.into(),
                    preset: true,
                    mark: cli::Mark::Unread,
                    watch: None,
                    used: false,
                    ending: false,
                },
            );
        };
        open(&meta.id, "s-1");
        open(&other.id, "s-2");
        assert_eq!(chats.free(&meta.id).unwrap_err(), IN_CLI);
        assert_eq!(chats.sleep_session(&meta.id, Duration::ZERO).unwrap_err(), IN_CLI);
        assert!(!chats.store.get(&meta.id).unwrap().sleeping);
        // The terminal's own answers aren't the user's; Enter is.
        chats.cli_write(&meta.id, "\x1b[1;1R").unwrap();
        assert!(!chats.cli.lock().unwrap()[&meta.id].used);
        chats.cli_write(&meta.id, "hi\r").unwrap();
        assert!(chats.cli.lock().unwrap()[&meta.id].used);
        // A removed project's chat lets go of its CLI.
        chats.close(&other.id);
        assert_eq!(chats.cli_ids(), std::slice::from_ref(&meta.id));
        assert!(chats.free(&other.id).is_ok());
        // Quitting keeps the session something was said in, and only that one.
        chats.stop_all();
        assert!(chats.cli_ids().is_empty());
        assert!(chats.free(&meta.id).is_ok());
        assert_eq!(chats.store.get(&meta.id).unwrap().agent_session_id.as_deref(), Some("s-1"));
        assert_eq!(chats.store.get(&other.id).unwrap().agent_session_id, None);
        assert!(chats.cli_write(&meta.id, "x").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A driver that's mid-turn until it's interrupted.
    #[cfg(target_os = "linux")]
    struct Working {
        idle: Arc<std::sync::atomic::AtomicBool>,
        stopped: Arc<std::sync::atomic::AtomicBool>,
    }

    #[cfg(target_os = "linux")]
    impl Driver for Working {
        fn send(&mut self, _: Prompt) -> Result<(), String> {
            Ok(())
        }
        fn interrupt(&mut self) -> Result<(), String> {
            self.idle.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        fn answer(&mut self, _: &str, _: Decision) -> Result<(), String> {
            Ok(())
        }
        fn idle(&self) -> bool {
            self.idle.load(std::sync::atomic::Ordering::SeqCst)
        }
        fn stop(&mut self) {
            self.stopped.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// The whole handover, in an app with no window: a chat's working agent gives way to the
    /// CLI only when told to, the CLI runs on the chat's session and has the chat to itself,
    /// and when it ends the chat has what was said there and its agent resumes the same
    /// session. The "CLI" is a script that does what Claude Code does as far as oah sees it:
    /// it prints its arguments and answers what's typed, saves the conversation in a file
    /// named after its session once something is said, starts a new session on `/clear`, and
    /// notes each session it's on where its settings' hook says to.
    #[test]
    #[cfg(target_os = "linux")]
    fn the_cli_takes_a_chat_over_and_hands_it_back() {
        use oah_core::settings::EnvVar;
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicBool, Ordering as O};

        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let fake = dir.path().join("fake-cli");
        std::fs::write(
            &fake,
            r#"#!/bin/sh
printf 'args:%s\n' "$*"
sid=$2
dir="$CLAUDE_CONFIG_DIR/projects/p"
log=$(printf '%s' "$*" | sed -n 's/.*>> \(\/tmp\/oah-cli-[A-Za-z0-9]*\.log\).*/\1/p')
visit() { [ -n "$log" ] && printf '%s {"session_id":"%s","transcript_path":"%s"}\n' "$(date +%s)" "$sid" "$dir/$sid.jsonl" >> "$log"; }
visit
n=0
while IFS= read -r line; do
  [ "$line" = "/exit" ] && exit 0
  [ "$line" = "/fail" ] && exit 3
  if [ "$line" = "/clear" ]; then sid="cleared-$$"; visit; printf 'cleared\n'; continue; fi
  mkdir -p "$dir"
  n=$((n+1)); ts="$(printf '%.23s' "$(date -u +%Y-%m-%dT%H:%M:%S.%N)")Z"
  printf '{"type":"user","uuid":"u%s-%s","timestamp":"%s","message":{"role":"user","content":"%s"}}\n' "$$" "$n" "$ts" "$line" >> "$dir/$sid.jsonl"
  printf '{"type":"assistant","uuid":"a%s-%s","timestamp":"%s","message":{"id":"m","content":[{"type":"text","text":"heard: %s"}]}}\n' "$$" "$n" "$ts" "$line" >> "$dir/$sid.jsonl"
  printf 'said:%s\n' "$line"
done
"#,
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut settings = Settings::default();
        let project = settings.add_project(Location::Local, work.to_str().unwrap()).unwrap();
        settings.providers.claude.binary = fake.to_string_lossy().into_owned();
        settings.providers.claude.env = vec![EnvVar {
            key: "CLAUDE_CONFIG_DIR".into(),
            value: dir.path().join("claude").to_string_lossy().into_owned(),
            in_keychain: false,
        }];
        let app = tauri::test::mock_app();
        app.manage(crate::AppState::for_tests(settings.clone(), ChatStore::new(dir.path().join("chats"))));
        let handle = app.handle().clone();
        let state = app.state::<crate::AppState>();
        let chats = &state.chats;

        let mut meta = chats.store.create(&project.id, AgentKind::Claude, "", work.to_str().unwrap()).unwrap();
        meta.agent_session_id = Some("sess-1".into());
        meta.sleeping = true;
        chats.store.save(&meta).unwrap();
        let id = meta.id.clone();
        let (idle, stopped) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        chats.add_driver(&id, Box::new(Working { idle: idle.clone(), stopped: stopped.clone() }));

        // What the terminal shows, and whether its program ended (with which code).
        let screen = Arc::new(Mutex::new(String::new()));
        let ended = Arc::new(Mutex::new(None::<Option<u32>>));
        let open = |chat: &str, force: bool| {
            let (screen, ended) = (screen.clone(), ended.clone());
            screen.lock().unwrap().clear();
            *ended.lock().unwrap() = None;
            chats.cli_open(&handle, &settings, chat, 80, 24, force, move |e| {
                match e {
                    PtyEvent::Data(d) => screen.lock().unwrap().push_str(&d),
                    PtyEvent::Exit(code) => *ended.lock().unwrap() = Some(code),
                }
                true
            })
        };
        let until = |what: &str, done: &dyn Fn() -> bool| {
            let end = Instant::now() + Duration::from_secs(10);
            while !done() {
                assert!(Instant::now() < end, "never: {what}; the terminal shows {:?}", screen.lock().unwrap());
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        let shows = |text: &str| until(text, &|| screen.lock().unwrap().contains(text));
        let log = |chat: &str| chats.store.events(chat).unwrap();
        let texts = |chat: &str, kind: &str| {
            log(chat).iter().filter(|e| e["type"] == kind).map(|e| e["text"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>()
        };
        let notes = |chat: &str| texts(chat, "notice");
        let prompt = |text: &str| Prompt { id: "p".into(), text: text.into(), images: vec![], files: vec![] };

        // An agent that's working isn't stopped unasked.
        assert_eq!(open(&id, false).unwrap_err(), CLI_BUSY);
        assert!(!stopped.load(O::SeqCst) && chats.cli_ids().is_empty() && chats.running() == [id.clone()]);
        // Asked to, its turn is interrupted, it stops, and the CLI opens on the chat's session.
        open(&id, true).unwrap();
        assert!(idle.load(O::SeqCst) && stopped.load(O::SeqCst), "interrupted, then stopped");
        assert!(chats.running().is_empty());
        assert_eq!(chats.cli_ids(), std::slice::from_ref(&id));
        shows("args:--resume sess-1 --permission-mode default");
        assert!(log(&id).iter().any(|e| e["type"] == "exited"), "the page hears the agent went: {:?}", log(&id));
        assert!(!chats.store.get(&id).unwrap().sleeping);

        // The CLI has the chat to itself.
        assert_eq!(chats.send(&handle, &settings, &id, prompt("hi"), None).unwrap_err(), IN_CLI);
        assert_eq!(chats.compact(&handle, &settings, &id, "", None).unwrap_err(), IN_CLI);
        assert_eq!(chats.clear(&handle, &id).unwrap_err(), IN_CLI);
        assert_eq!(chats.sleep(&handle, &id, Duration::ZERO).unwrap_err(), IN_CLI);
        assert!(chats.running().is_empty(), "no agent was started beside the CLI");

        // What's typed reaches it; leaving it frees the chat, which says where the time went.
        chats.cli_write(&id, "remember this\r").unwrap();
        shows("said:remember this");
        chats.cli_write(&id, "/exit\r").unwrap();
        until("the exit", &|| ended.lock().unwrap().is_some());
        assert_eq!(*ended.lock().unwrap(), Some(Some(0)));
        assert!(chats.cli_ids().is_empty(), "free before the page hears of the exit");
        let said = notes(&id);
        assert!(
            said.len() == 1 && said[0].starts_with("Continued in Claude Code's own CLI for ") && said[0].ends_with("follows."),
            "{said:?}"
        );
        // What was said there is the chat's now: the message, the answer, as a turn.
        assert_eq!(texts(&id, "userMessage"), ["remember this"]);
        assert_eq!(texts(&id, "text"), ["heard: remember this"]);
        let kinds: Vec<String> = log(&id).iter().map(|e| e["type"].as_str().unwrap_or("").to_string()).collect();
        assert_eq!(kinds, ["exited", "notice", "userMessage", "turnStarted", "text", "turnFinished"]);
        assert_eq!(chats.store.get(&id).unwrap().agent_session_id.as_deref(), Some("sess-1"));

        // The next message starts the chat's agent again, on the same session.
        std::env::set_var("OAH_MOCK_AGENT", "1");
        chats.send(&handle, &settings, &id, prompt("and now?"), None).unwrap();
        assert_eq!(chats.running(), std::slice::from_ref(&id));
        until("the agent's start", &|| log(&id).iter().any(|e| e["type"] == "started"));
        let started = log(&id).into_iter().find(|e| e["type"] == "started").unwrap();
        assert_eq!(started["sessionId"], "sess-1", "{started}");
        chats.close(&id);

        // Opened and left with nothing said: no line in the chat.
        open(&id, false).unwrap();
        shows("args:--resume sess-1");
        chats.cli_write(&id, "\x1b[1;1R").unwrap();
        chats.cli_close(&handle, &id);
        assert!(chats.cli_ids().is_empty());
        assert_eq!(notes(&id).len(), 1, "{:?}", log(&id).iter().rev().take(8).collect::<Vec<_>>());
        assert!(chats.send(&handle, &settings, &id, prompt("still here"), None).is_ok());
        chats.close(&id);

        // A `/clear` in the CLI starts another conversation: the chat follows it there, with
        // what was said on both sides of it.
        open(&id, false).unwrap();
        shows("args:--resume sess-1");
        chats.cli_write(&id, "one more\r").unwrap();
        shows("said:one more");
        chats.cli_write(&id, "/clear\r").unwrap();
        shows("cleared");
        chats.cli_write(&id, "a new topic\r").unwrap();
        shows("said:a new topic");
        chats.cli_write(&id, "/exit\r").unwrap();
        until("the exit", &|| ended.lock().unwrap().is_some());
        let now = chats.store.get(&id).unwrap().agent_session_id.unwrap();
        assert!(now.starts_with("cleared-"), "{now}");
        assert_eq!(texts(&id, "userMessage"), ["remember this", "and now?", "still here", "one more", "a new topic"]);
        assert!(texts(&id, "text").ends_with(&["heard: one more".to_string(), "heard: a new topic".to_string()]));
        assert!(notes(&id).last().unwrap().starts_with("In the CLI the conversation changed"), "{:?}", notes(&id));
        // From here the CLI opens on the one it ended on.
        open(&id, false).unwrap();
        shows(&format!("args:--resume {now}"));
        chats.cli_close(&handle, &id);

        // A CLI that fails says so with its code, and the chat is free all the same.
        open(&id, false).unwrap();
        shows("args:");
        chats.cli_write(&id, "/fail\r").unwrap();
        until("the failure", &|| ended.lock().unwrap().is_some());
        assert_eq!(*ended.lock().unwrap(), Some(Some(3)));
        assert!(chats.cli_ids().is_empty());

        // A chat with no session starts one in the CLI under an id oah picks. Left unused,
        // the chat keeps none; used, the saved session is the chat's, and names it.
        let fresh = chats.store.create(&project.id, AgentKind::Claude, "", work.to_str().unwrap()).unwrap();
        open(&fresh.id, false).unwrap();
        shows("args:--session-id ");
        chats.cli_write(&fresh.id, "/exit\r").unwrap();
        until("the exit", &|| ended.lock().unwrap().is_some());
        let left = chats.store.get(&fresh.id).unwrap();
        assert!(left.agent_session_id.is_none() && left.title.is_empty() && notes(&fresh.id).is_empty(), "{left:?}");
        open(&fresh.id, false).unwrap();
        shows("args:--session-id ");
        let given = screen.lock().unwrap().split("--session-id ").nth(1).unwrap().split_whitespace().next().unwrap().to_string();
        chats.cli_write(&fresh.id, "hello\r").unwrap();
        shows("said:hello");
        chats.cli_write(&fresh.id, "/exit\r").unwrap();
        until("the exit", &|| ended.lock().unwrap().is_some());
        let used = chats.store.get(&fresh.id).unwrap();
        assert_eq!(used.agent_session_id.as_deref(), Some(given.as_str()));
        assert_eq!(used.title, "hello", "named after the first thing said there");
        assert_eq!(texts(&fresh.id, "text"), ["heard: hello"]);
        assert_eq!(notes(&fresh.id).len(), 1);
        // From here it resumes that one.
        open(&fresh.id, false).unwrap();
        shows(&format!("args:--resume {given}"));
        chats.cli_close(&handle, &fresh.id);

        // A phone or MCP takes the chat back: the CLI ends, the window is told, and what was
        // said there is the chat's as when it's left from the window.
        let told = Arc::new(Mutex::new(Vec::new()));
        {
            use tauri::Listener;
            let told = told.clone();
            handle.listen("chat-cli-taken", move |e| told.lock().unwrap().push(e.payload().to_string()));
        }
        open(&fresh.id, false).unwrap();
        shows("args:--resume ");
        chats.cli_write(&fresh.id, "before the phone\r").unwrap();
        shows("said:before the phone");
        assert!(chats.cli_take(&handle, &fresh.id));
        assert!(chats.cli_ids().is_empty());
        assert!(!chats.cli_take(&handle, &fresh.id), "nothing to take twice");
        until("the window hearing of it", &|| !told.lock().unwrap().is_empty());
        assert_eq!(*told.lock().unwrap(), [format!("\"{}\"", fresh.id)]);
        assert_eq!(texts(&fresh.id, "userMessage"), ["hello", "before the phone"]);
        assert!(chats.send(&handle, &settings, &fresh.id, prompt("from the phone"), None).is_ok());
        chats.close(&fresh.id);

        // An agent that names its sessions itself has none to open before a first message.
        let codex = chats.store.create(&project.id, AgentKind::Codex, "", work.to_str().unwrap()).unwrap();
        assert!(open(&codex.id, false).unwrap_err().contains("Send a first message"));
        // Deleting a chat ends its CLI.
        open(&id, false).unwrap();
        chats.delete(&id).unwrap();
        assert!(chats.cli_ids().is_empty());
        until("the deleted chat's CLI ending", &|| ended.lock().unwrap().is_some());
        chats.stop_all();
        std::env::remove_var("OAH_MOCK_AGENT");
    }

    #[test]
    fn says_how_long_the_cli_had_the_chat() {
        let note = |s| cli_note(AgentKind::Claude, Duration::from_secs(s), false);
        assert!(note(5).ends_with("the agent remembers it."));
        assert!(cli_note(AgentKind::Codex, Duration::from_secs(5), true).ends_with("Codex's own CLI for 5 s. What was said there follows."));
        assert!(note(0).contains("Claude Code's own CLI for 1 s."), "{}", note(0));
        assert!(note(45).contains("for 45 s."));
        assert!(note(252).contains("for 4 min."));
        assert!(note(3900).contains("for 1 h 5 min."));
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
