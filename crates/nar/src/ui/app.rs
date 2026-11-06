//! Application state, input handling, commands and agent orchestration.
//! Rendering lives in `view.rs`.

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use opentui::{key, Event, KeyEvent, MouseEvent, MouseKind, Terminal, Waker};
use serde_json::Value;
use tokio::runtime::Handle;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::agent::{self, Agent, AgentEvent, PermissionReply, TurnConfig};
use crate::api::ModelInfo;
use crate::auth::{self, Auth, AuthMethod, AuthStatus};
use crate::commands;
use crate::config::{self, Effort, Mode, Permission, Settings};
use crate::mcp::{McpManager, ServerState};
use crate::session::{self, Entry, Goal, SessionData, ToolStatus};
use crate::skills::Skills;

use super::editor::Editor;
use super::text::Line;

pub enum AppMsg {
    Agent(AgentEvent),
    LoginStarted { url: String, display: String },
    Login(Result<AuthStatus, String>),
    Auth(AuthStatus),
    Models(Result<Vec<ModelInfo>, String>),
    McpChanged,
    Notice(String),
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerKind {
    Model,
    Effort,
    Permission,
    Session,
    Auth,
}

#[derive(Clone, Debug)]
pub struct PickItem {
    pub label: String,
    pub detail: String,
    pub value: String,
    pub current: bool,
}

pub struct Picker {
    pub kind: PickerKind,
    pub title: String,
    pub items: Vec<PickItem>,
    pub selected: usize,
    pub filter: String,
    pub loading: bool,
}

impl Picker {
    pub fn visible(&self) -> Vec<(usize, &PickItem)> {
        let f = self.filter.to_lowercase();
        self.items
            .iter()
            .enumerate()
            .filter(|(_, i)| f.is_empty() || i.label.to_lowercase().contains(&f) || i.detail.to_lowercase().contains(&f))
            .collect()
    }
}

pub struct PendingPermission {
    pub title: String,
    pub kind: String,
    pub reply: Option<oneshot::Sender<PermissionReply>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusyKind {
    Turn,
    Compact,
    Login,
}

pub struct Busy {
    pub kind: BusyKind,
    pub cancel: CancellationToken,
    pub started: Instant,
}

/// What to do after an automatic compaction finishes.
enum After {
    Nothing,
    Submit(String),
    ContinueGoal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputMode {
    Prompt,
    ApiKey,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w as i32 && y < self.y + self.h as i32
    }
}

/// Selection in flattened chat-line coordinates: (line index, display column).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: (usize, usize),
    pub head: (usize, usize),
}

impl Selection {
    pub fn ordered(&self) -> ((usize, usize), (usize, usize)) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }
    /// Selected column range [c0, c1) on `line`.
    pub fn cols(&self, line: usize, line_w: usize) -> Option<(usize, usize)> {
        let ((l0, c0), (l1, c1)) = self.ordered();
        if line < l0 || line > l1 || (l0 == l1 && c0 == c1) {
            return None;
        }
        let a = if line == l0 { c0 } else { 0 };
        let b = if line == l1 { c1 } else { line_w.max(1) };
        if a >= b {
            None
        } else {
            Some((a, b))
        }
    }
}

pub struct App {
    pub settings: Settings,
    pub session: SessionData,
    pub cwd: PathBuf,
    pub editor: Editor,
    pub input_mode: InputMode,
    history: Vec<String>,
    hist_idx: Option<usize>,
    draft: String,

    // transcript view
    pub revs: Vec<u64>,
    rev_counter: u64,
    pub cache: Vec<Option<(u64, usize, Vec<Line>)>>,
    pub flat: Vec<Line>,
    pub flat_key: (u64, usize),
    pub scroll: usize,
    pub selection: Option<Selection>,
    pub selecting: bool,
    pub chat_rect: Rect,
    pub chat_top: usize,
    stream_text: Option<usize>,
    stream_reasoning: Option<usize>,

    // overlays
    pub picker: Option<Picker>,
    pub palette_idx: usize,
    pub permission: Option<PendingPermission>,
    pub toast: Option<(String, Instant, bool)>,

    // activity
    pub busy: Option<Busy>,
    pub activity: String,
    queue: Vec<String>,
    after: After,
    pub tick: u64,

    // services
    pub auth_status: AuthStatus,
    pub models: Vec<ModelInfo>,
    pub skills: Arc<Skills>,
    pub mcp: McpManager,
    pub mcp_summary: Vec<(String, ServerState, Vec<String>, String)>,
    pub agent: Agent,
    pub auth: Auth,
    rt: Handle,
    tx: Sender<AppMsg>,
    waker: Waker,
    pub quit: bool,
    pub dirty: bool,
    pub screen_w: u16,
    login_cancel: Option<CancellationToken>,
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        settings: Settings,
        session: SessionData,
        cwd: PathBuf,
        agent: Agent,
        auth: Auth,
        mcp: McpManager,
        skills: Arc<Skills>,
        rt: Handle,
        tx: Sender<AppMsg>,
        waker: Waker,
    ) -> App {
        let n = session.transcript.len();
        let mut app = App {
            settings,
            session,
            cwd,
            editor: Editor::default(),
            input_mode: InputMode::Prompt,
            history: session::load_history(),
            hist_idx: None,
            draft: String::new(),
            revs: vec![1; n],
            rev_counter: 1,
            cache: vec![None; n],
            flat: vec![],
            flat_key: (0, 0),
            scroll: 0,
            selection: None,
            selecting: false,
            chat_rect: Rect::default(),
            chat_top: 0,
            stream_text: None,
            stream_reasoning: None,
            picker: None,
            palette_idx: 0,
            permission: None,
            toast: None,
            busy: None,
            activity: String::new(),
            queue: vec![],
            after: After::Nothing,
            tick: 0,
            auth_status: AuthStatus::default(),
            models: vec![],
            skills,
            mcp,
            mcp_summary: vec![],
            agent,
            auth,
            rt,
            tx,
            waker,
            quit: false,
            dirty: true,
            screen_w: 80,
            login_cancel: None,
        };
        app.refresh_auth();
        app
    }

    // ---- infrastructure ---------------------------------------------------

    fn send_later(&self, f: impl std::future::Future<Output = AppMsg> + Send + 'static) {
        let tx = self.tx.clone();
        let waker = self.waker.clone();
        self.rt.spawn(async move {
            let m = f.await;
            let _ = tx.send(m);
            waker.wake();
        });
    }

    fn sink(&self) -> agent::Sink {
        let tx = Mutex::new(self.tx.clone());
        let waker = self.waker.clone();
        Arc::new(move |ev| {
            let _ = tx.lock().unwrap().send(AppMsg::Agent(ev));
            waker.wake();
        })
    }

    pub fn refresh_auth(&self) {
        let auth = self.auth.clone();
        self.send_later(async move { AppMsg::Auth(auth.status().await) });
    }

    pub fn refresh_mcp(&self) {
        let mcp = self.mcp.clone();
        let tx = self.tx.clone();
        let waker = self.waker.clone();
        self.rt.spawn(async move {
            let _ = mcp.summary().await;
            let _ = tx.send(AppMsg::McpChanged);
            waker.wake();
        });
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now(), false));
    }

    fn toast_err(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now(), true));
    }

    pub fn context_window(&self) -> u64 {
        let from_list = self.models.iter().find(|m| m.slug == self.settings.model).and_then(|m| m.context_window);
        config::context_window_for(&self.settings.model, self.settings.context_window.or(from_list))
    }

    // ---- transcript -------------------------------------------------------

    fn bump(&mut self) -> u64 {
        self.rev_counter += 1;
        self.rev_counter
    }

    pub fn push_entry(&mut self, e: Entry) -> usize {
        self.session.transcript.push(e);
        let r = self.bump();
        self.revs.push(r);
        self.cache.push(None);
        self.dirty = true;
        if self.scroll == 0 {
            self.selection = None;
        }
        self.session.transcript.len() - 1
    }

    fn touch(&mut self, i: usize) {
        let r = self.bump();
        if let Some(v) = self.revs.get_mut(i) {
            *v = r;
        }
        self.dirty = true;
    }

    fn info(&mut self, text: impl Into<String>) {
        self.push_entry(Entry::Info { text: text.into() });
    }

    fn error(&mut self, text: impl Into<String>) {
        self.push_entry(Entry::Error { text: text.into() });
    }

    fn reset_transcript(&mut self) {
        let n = self.session.transcript.len();
        self.revs = vec![self.bump(); n];
        self.cache = vec![None; n];
        self.flat_key = (0, 0);
        self.scroll = 0;
        self.selection = None;
        self.stream_text = None;
        self.stream_reasoning = None;
        self.dirty = true;
    }

    // ---- events -----------------------------------------------------------

    pub fn handle_event(&mut self, ev: Event, term: &mut Terminal) {
        self.dirty = true;
        match ev {
            Event::Key(k) => self.on_key(k, term),
            Event::Paste(text) => {
                if let Some(p) = self.picker.as_mut() {
                    p.filter.push_str(text.trim());
                    p.selected = 0;
                } else if self.permission.is_none() {
                    self.editor.insert(&text);
                    self.hist_idx = None;
                }
            }
            Event::Mouse(m) => self.on_mouse(m, term),
            Event::Resize(..) => {
                self.flat_key = (0, 0);
            }
            Event::Focus(_) | Event::Wake => {}
        }
    }

    fn on_mouse(&mut self, m: MouseEvent, term: &mut Terminal) {
        let (x, y) = (m.x as i32, m.y as i32);
        match m.kind {
            MouseKind::ScrollUp => self.scroll_by(3),
            MouseKind::ScrollDown => self.scroll_by(-3),
            MouseKind::Down if m.button == 0 => {
                if self.chat_rect.contains(x, y) && self.picker.is_none() {
                    let pos = self.chat_pos(x, y);
                    self.selection = Some(Selection { anchor: pos, head: pos });
                    self.selecting = true;
                } else {
                    self.selection = None;
                }
            }
            MouseKind::Drag if self.selecting => {
                if y < self.chat_rect.y {
                    self.scroll_by(1);
                } else if y >= self.chat_rect.y + self.chat_rect.h as i32 {
                    self.scroll_by(-1);
                }
                let pos = self.chat_pos(x, y);
                if let Some(s) = self.selection.as_mut() {
                    s.head = pos;
                }
            }
            MouseKind::Up if self.selecting => {
                self.selecting = false;
                if let Some(sel) = self.selection {
                    if sel.anchor != sel.head {
                        let text = self.selected_text(sel);
                        if !text.trim().is_empty() {
                            self.copy(&text, term);
                        } else {
                            self.selection = None;
                        }
                    } else {
                        self.selection = None;
                    }
                }
            }
            _ => {}
        }
    }

    fn chat_pos(&self, x: i32, y: i32) -> (usize, usize) {
        let r = self.chat_rect;
        let yy = (y - r.y).clamp(0, r.h.saturating_sub(1) as i32) as usize;
        let xx = (x - r.x).max(0) as usize;
        let line = (self.chat_top + yy).min(self.flat.len().saturating_sub(1));
        let lw = self.flat.get(line).map(|l| l.width()).unwrap_or(0);
        (line, if x >= r.x + r.w as i32 { lw.max(xx) } else { xx })
    }

    fn selected_text(&self, sel: Selection) -> String {
        let ((l0, _), (l1, _)) = sel.ordered();
        let mut out: Vec<String> = Vec::new();
        for i in l0..=l1.min(self.flat.len().saturating_sub(1)) {
            let line = &self.flat[i];
            if let Some((a, b)) = sel.cols(i, line.width()) {
                out.push(line.copy_text(a, b).trim_end().to_string());
            } else {
                out.push(String::new());
            }
        }
        out.join("\n").trim_matches('\n').to_string()
    }

    fn copy(&mut self, text: &str, term: &mut Terminal) {
        let osc = term.copy_to_clipboard(text);
        let native = copy_native(text);
        let n = text.chars().count();
        if osc || native {
            self.toast(format!("Copied {n} characters"));
        } else {
            self.toast_err("Clipboard unavailable (terminal lacks OSC 52 and no wl-copy/xclip/pbcopy)");
        }
    }

    pub fn scroll_by(&mut self, delta: i32) {
        let max = self.flat.len().saturating_sub(self.chat_rect.h as usize);
        let s = self.scroll as i64 + delta as i64;
        self.scroll = s.clamp(0, max as i64) as usize;
        self.dirty = true;
    }

    fn palette(&self) -> Vec<(String, String, String)> {
        if self.picker.is_some() || self.permission.is_some() || self.input_mode != InputMode::Prompt {
            return vec![];
        }
        let t = &self.editor.text;
        if !t.starts_with('/') || t.contains(char::is_whitespace) {
            return vec![];
        }
        let skills: Vec<(String, String)> = self.skills.list.iter().map(|s| (s.name.clone(), s.description.clone())).collect();
        commands::matches(&t[1..], &skills)
    }

    pub fn palette_items(&self) -> Vec<(String, String, String)> {
        self.palette()
    }

    fn on_key(&mut self, k: KeyEvent, term: &mut Terminal) {
        if k.is_ctrl('c') {
            if self.input_mode == InputMode::ApiKey {
                self.input_mode = InputMode::Prompt;
                self.editor.masked = false;
                self.editor.clear();
                return;
            }
            self.quit = true;
            return;
        }
        if k.is_ctrl('l') {
            self.flat_key = (0, 0);
            return;
        }
        // Permission prompt captures input.
        if self.permission.is_some() {
            let reply = match (k.code, k.text.as_str()) {
                (_, "y") | (_, "Y") | (key::ENTER, _) | (key::KP_ENTER, _) => Some(PermissionReply::Once),
                (_, "a") | (_, "A") => Some(PermissionReply::Always),
                (_, "n") | (_, "N") | (key::ESCAPE, _) => Some(PermissionReply::Deny),
                _ => None,
            };
            if let Some(r) = reply {
                if let Some(mut p) = self.permission.take() {
                    if let Some(tx) = p.reply.take() {
                        let _ = tx.send(r);
                    }
                }
                self.activity.clear();
            }
            return;
        }
        if self.picker.is_some() {
            self.on_picker_key(k);
            return;
        }
        let width = self.input_width();
        let palette = self.palette();
        if !palette.is_empty() {
            match k.code {
                key::UP => {
                    self.palette_idx = (self.palette_idx + palette.len() - 1) % palette.len();
                    return;
                }
                key::DOWN => {
                    self.palette_idx = (self.palette_idx + 1) % palette.len();
                    return;
                }
                key::TAB => {
                    let (name, _, _) = &palette[self.palette_idx.min(palette.len() - 1)];
                    self.editor.set(&format!("/{name} "));
                    self.palette_idx = 0;
                    return;
                }
                key::ENTER | key::KP_ENTER if k.mods.none() => {
                    let (name, _, _) = palette[self.palette_idx.min(palette.len() - 1)].clone();
                    self.editor.clear();
                    self.palette_idx = 0;
                    self.run_command(&format!("/{name}"), term);
                    return;
                }
                key::ESCAPE => {
                    self.editor.clear();
                    return;
                }
                _ => {}
            }
        }

        match k.code {
            key::ESCAPE => {
                if let Some(b) = &self.busy {
                    b.cancel.cancel();
                    if b.kind == BusyKind::Login {
                        if let Some(c) = self.login_cancel.take() {
                            c.cancel();
                        }
                        self.busy = None;
                        self.info("Sign-in cancelled.");
                    } else {
                        self.activity = "Interrupting…".into();
                    }
                    self.queue.clear();
                    if let Some(g) = self.session.goal.as_mut() {
                        if !g.done {
                            self.info("Goal paused. Send a message to continue, or /goal clear to stop.");
                        }
                    }
                } else if self.input_mode == InputMode::ApiKey {
                    self.input_mode = InputMode::Prompt;
                    self.editor.masked = false;
                    self.editor.clear();
                } else if self.selection.is_some() {
                    self.selection = None;
                }
            }
            key::TAB if k.mods.shift() => self.toggle_mode(),
            key::ENTER | key::KP_ENTER => {
                if k.mods.shift() || k.mods.alt() || k.mods.ctrl() {
                    self.editor.insert("\n");
                } else {
                    self.submit(term);
                }
            }
            key::PAGE_UP => self.scroll_by((self.chat_rect.h as i32 - 2).max(1)),
            key::PAGE_DOWN => self.scroll_by(-(self.chat_rect.h as i32 - 2).max(1)),
            key::HOME if k.mods.ctrl() => self.scroll = self.flat.len(),
            key::END if k.mods.ctrl() => self.scroll = 0,
            key::HOME => self.editor.home(),
            key::END => self.editor.end(),
            key::LEFT if k.mods.ctrl() || k.mods.alt() => self.editor.word_left(),
            key::RIGHT if k.mods.ctrl() || k.mods.alt() => self.editor.word_right(),
            key::LEFT => self.editor.left(),
            key::RIGHT => self.editor.right(),
            key::UP => {
                if !self.editor.up(width) {
                    self.history_prev();
                }
            }
            key::DOWN => {
                if !self.editor.down(width) {
                    self.history_next();
                }
            }
            key::BACKSPACE | 8 => {
                if k.mods.alt() || k.mods.ctrl() || k.code == 8 && k.mods.none() && false {
                    self.editor.delete_word_back();
                } else {
                    self.editor.backspace();
                }
            }
            key::DELETE => {
                if k.mods.alt() || k.mods.ctrl() {
                    self.editor.delete_word_forward();
                } else {
                    self.editor.delete();
                }
            }
            _ => {
                if k.is_ctrl('a') {
                    self.editor.home();
                } else if k.is_ctrl('e') {
                    self.editor.end();
                } else if k.is_ctrl('u') {
                    self.editor.kill_to_start();
                } else if k.is_ctrl('k') {
                    self.editor.kill_to_end();
                } else if k.is_ctrl('w') {
                    self.editor.delete_word_back();
                } else if k.is_ctrl('j') || k.code == 10 {
                    self.editor.insert("\n");
                } else if k.is_ctrl('b') {
                    self.editor.left();
                } else if k.is_ctrl('f') {
                    self.editor.right();
                } else if k.is_ctrl('d') {
                    if self.editor.is_empty() {
                        self.quit = true;
                    } else {
                        self.editor.delete();
                    }
                } else if k.mods.alt() && (k.code == 'b' as u32) {
                    self.editor.word_left();
                } else if k.mods.alt() && (k.code == 'f' as u32) {
                    self.editor.word_right();
                } else if k.mods.alt() && k.code == 'd' as u32 {
                    self.editor.delete_word_forward();
                } else if !k.text.is_empty() && !k.mods.ctrl() {
                    self.editor.insert(&k.text);
                    self.hist_idx = None;
                    self.palette_idx = 0;
                    self.selection = None;
                }
            }
        }
    }

    fn history_prev(&mut self) {
        if self.history.is_empty() || self.input_mode != InputMode::Prompt {
            return;
        }
        let idx = match self.hist_idx {
            None => {
                self.draft = self.editor.text.clone();
                self.history.len() - 1
            }
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.hist_idx = Some(idx);
        let t = self.history[idx].clone();
        self.editor.set(&t);
    }

    fn history_next(&mut self) {
        let Some(i) = self.hist_idx else { return };
        if i + 1 >= self.history.len() {
            self.hist_idx = None;
            let d = std::mem::take(&mut self.draft);
            self.editor.set(&d);
        } else {
            self.hist_idx = Some(i + 1);
            let t = self.history[i + 1].clone();
            self.editor.set(&t);
        }
    }

    pub fn input_width(&self) -> usize {
        // box border (2) + padding + prompt glyph: matches view::render.
        (self.screen_w as usize).saturating_sub(6).max(4)
    }

    fn toggle_mode(&mut self) {
        self.set_mode(self.settings.mode.toggle());
    }

    fn set_mode(&mut self, m: Mode) {
        self.settings.mode = m;
        self.session.mode = m;
        self.settings.save();
        self.toast(match m {
            Mode::Plan => "Plan mode — read-only research and planning",
            Mode::Build => "Build mode — edits and commands enabled",
        });
    }

    fn on_picker_key(&mut self, k: KeyEvent) {
        let Some(p) = self.picker.as_mut() else { return };
        let n = p.visible().len();
        match k.code {
            key::ESCAPE => {
                self.picker = None;
            }
            key::UP => {
                if n > 0 {
                    p.selected = (p.selected + n - 1) % n;
                }
            }
            key::DOWN | key::TAB => {
                if n > 0 {
                    p.selected = (p.selected + 1) % n;
                }
            }
            key::PAGE_UP => p.selected = p.selected.saturating_sub(10),
            key::PAGE_DOWN => p.selected = (p.selected + 10).min(n.saturating_sub(1)),
            key::BACKSPACE => {
                p.filter.pop();
                p.selected = 0;
            }
            key::ENTER | key::KP_ENTER => {
                let vis = p.visible();
                if let Some((_, item)) = vis.get(p.selected) {
                    let value = item.value.clone();
                    let kind = p.kind;
                    self.picker = None;
                    self.picked(kind, &value);
                }
            }
            _ => {
                if !k.text.is_empty() && !k.mods.ctrl() {
                    p.filter.push_str(&k.text);
                    p.selected = 0;
                }
            }
        }
    }

    fn picked(&mut self, kind: PickerKind, value: &str) {
        match kind {
            PickerKind::Model => self.set_model(value),
            PickerKind::Effort => {
                if let Some(e) = Effort::parse(value) {
                    self.set_effort(e);
                }
            }
            PickerKind::Permission => {
                if let Some(p) = Permission::parse(value) {
                    self.set_permission(p);
                }
            }
            PickerKind::Session => self.resume(value),
            PickerKind::Auth => self.auth_action(value),
        }
    }

    // ---- settings -----------------------------------------------------------

    fn set_model(&mut self, slug: &str) {
        self.settings.model = slug.to_string();
        self.session.model = slug.to_string();
        self.settings.save();
        self.toast(format!("Model → {slug}"));
    }

    fn set_effort(&mut self, e: Effort) {
        self.settings.effort = e;
        self.session.effort = e;
        self.settings.save();
        self.toast(format!("Reasoning effort → {}", e.as_str()));
    }

    fn set_permission(&mut self, p: Permission) {
        self.settings.permission = p;
        self.settings.save();
        if p != Permission::AlwaysAllow {
            // Keep explicit grants only while the user stays permissive.
        }
        self.toast(format!("Permissions → {}", p.as_str()));
    }

    // ---- submit & commands ----------------------------------------------------

    fn submit(&mut self, term: &mut Terminal) {
        if self.input_mode == InputMode::ApiKey {
            let key = self.editor.take();
            self.editor.masked = false;
            self.input_mode = InputMode::Prompt;
            if key.trim().is_empty() {
                return;
            }
            let auth = self.auth.clone();
            self.send_later(async move {
                match auth.set_api_key(key).await {
                    Ok(s) => AppMsg::Login(Ok(s)),
                    Err(e) => AppMsg::Error(e.to_string()),
                }
            });
            return;
        }
        let text = self.editor.text.trim_end().to_string();
        if text.trim().is_empty() {
            return;
        }
        self.editor.clear();
        self.hist_idx = None;
        if self.history.last() != Some(&text) {
            self.history.push(text.clone());
            session::append_history(&text);
        }
        self.scroll = 0;
        if text.starts_with('/') && !text.starts_with("//") {
            self.run_command(&text, term);
            return;
        }
        if let Some(note) = text.strip_prefix('#') {
            if !note.starts_with('#') && !note.trim().is_empty() {
                match self.agent.ctx.memory.add(note, false) {
                    Ok(()) => self.toast(format!("Saved to project memory: {}", crate::tools::one_line(note, 60))),
                    Err(e) => self.toast_err(e.to_string()),
                }
                return;
            }
        }
        self.send_user(text);
    }

    /// Send a user message (or queue it while a turn is running).
    fn send_user(&mut self, text: String) {
        if self.busy.is_some() {
            self.queue.push(text);
            self.toast(format!("Queued · {} waiting", self.queue.len()));
            return;
        }
        if self.needs_compaction() {
            self.after = After::Submit(text);
            self.start_compact(String::new(), true);
            return;
        }
        if self.session.title.is_empty() {
            self.session.title = crate::tools::one_line(&text, 60);
        }
        self.push_entry(Entry::User { text: text.clone() });
        self.session.items.push(agent::user_message(&text));
        self.start_turn();
    }

    fn needs_compaction(&self) -> bool {
        self.settings.auto_compact
            && self.session.context_tokens > 0
            && (self.session.context_tokens as f64) > self.context_window() as f64 * self.settings.auto_compact_at
    }

    fn turn_config(&self) -> TurnConfig {
        let goal = self.session.goal.as_ref().filter(|g| !g.done).map(|g| g.text.clone());
        let mcp_names: Vec<String> =
            self.mcp_summary.iter().filter(|(_, s, _, _)| *s == ServerState::Ready).map(|(n, _, _, _)| n.clone()).collect();
        let instructions = agent::system_prompt(
            &self.cwd,
            self.settings.mode,
            goal.as_deref(),
            &self.skills.prompt_section(),
            &self.agent.ctx.memory.prompt_section(),
            &mcp_names,
        );
        TurnConfig {
            model: self.settings.model.clone(),
            effort: self.settings.effort.as_str().to_string(),
            mode: self.settings.mode,
            permission: self.settings.permission,
            instructions,
            goal_active: goal.is_some(),
            allowed_commands: self.settings.allowed_commands.clone(),
        }
    }

    fn start_turn(&mut self) {
        let cfg = self.turn_config();
        let cancel = CancellationToken::new();
        self.busy = Some(Busy { kind: BusyKind::Turn, cancel: cancel.clone(), started: Instant::now() });
        self.activity = "Thinking".into();
        self.stream_text = None;
        self.stream_reasoning = None;
        let agent = self.agent.clone();
        let items = self.session.items.clone();
        let sink = self.sink();
        self.rt.spawn(async move { agent.run_turn(cfg, items, sink, cancel).await });
    }

    fn start_compact(&mut self, focus: String, auto: bool) {
        if self.session.items.is_empty() {
            self.toast("Nothing to compact yet");
            return;
        }
        let cfg = self.turn_config();
        let cancel = CancellationToken::new();
        self.busy = Some(Busy { kind: BusyKind::Compact, cancel: cancel.clone(), started: Instant::now() });
        self.activity = if auto { "Auto-compacting conversation".into() } else { "Compacting conversation".into() };
        let agent = self.agent.clone();
        let items = self.session.items.clone();
        let sink = self.sink();
        self.rt.spawn(async move { agent.compact(cfg, items, focus, sink, cancel).await });
    }

    pub fn run_command(&mut self, line: &str, term: &mut Terminal) {
        let body = line.trim_start_matches('/');
        let (name, args) = match body.split_once(char::is_whitespace) {
            Some((n, a)) => (n.to_string(), a.trim().to_string()),
            None => (body.to_string(), String::new()),
        };
        let resolved = commands::resolve(&name.to_ascii_lowercase());
        let (name, args) = match resolved.split_once(' ') {
            Some((n, a)) => (n.to_string(), if args.is_empty() { a.to_string() } else { format!("{a} {args}") }),
            None => (resolved, args),
        };
        match name.as_str() {
            "help" => {
                self.push_entry(Entry::Info { text: commands::HELP.into() });
            }
            "exit" => self.quit = true,
            "model" => {
                if args.is_empty() {
                    self.open_model_picker();
                } else {
                    self.set_model(&args);
                }
            }
            "effort" => {
                if let Some(e) = Effort::parse(&args) {
                    self.set_effort(e);
                } else if args.is_empty() {
                    let supported: Vec<String> =
                        self.models.iter().find(|m| m.slug == self.settings.model).map(|m| m.efforts.clone()).unwrap_or_default();
                    let items = Effort::ALL
                        .iter()
                        .filter(|e| supported.is_empty() || supported.iter().any(|s| s == e.as_str()))
                        .map(|e| PickItem {
                            label: e.as_str().into(),
                            detail: e.describe().into(),
                            value: e.as_str().into(),
                            current: *e == self.settings.effort,
                        })
                        .collect();
                    self.open_picker(PickerKind::Effort, "Reasoning effort", items);
                } else {
                    self.toast_err(format!("Unknown effort '{args}' (minimal, low, medium, high, xhigh)"));
                }
            }
            "permissions" => {
                if let Some(p) = Permission::parse(&args) {
                    self.set_permission(p);
                } else if args.is_empty() {
                    let items = Permission::ALL
                        .iter()
                        .map(|p| PickItem {
                            label: p.as_str().into(),
                            detail: p.describe().into(),
                            value: p.as_str().into(),
                            current: *p == self.settings.permission,
                        })
                        .collect();
                    self.open_picker(PickerKind::Permission, "Permission level", items);
                } else {
                    self.toast_err(format!("Unknown permission level '{args}' (auto, ask, allow)"));
                }
            }
            "plan" => self.set_mode(Mode::Plan),
            "build" => self.set_mode(Mode::Build),
            "auth" => match args.as_str() {
                "" => self.open_auth_picker(),
                "login" | "chatgpt" => self.auth_action("login"),
                "apikey" | "api-key" | "key" => self.auth_action("apikey"),
                "logout" | "signout" => self.auth_action("logout"),
                "status" => self.auth_action("status"),
                "env" => self.auth_action("env"),
                other => self.toast_err(format!("Unknown /auth option '{other}'")),
            },
            "goal" => self.goal_command(&args),
            "compact" => {
                if self.busy.is_some() {
                    self.toast_err("Busy — wait for the current turn or press esc");
                } else {
                    self.after = After::Nothing;
                    self.start_compact(args, false);
                }
            }
            "clear" => self.new_conversation(),
            "session" => {
                if args.is_empty() {
                    self.open_session_picker();
                } else {
                    self.resume(&args);
                }
            }
            "memory" => self.memory_command(&args),
            "skills" => {
                if self.skills.list.is_empty() {
                    self.info("No skills found. Add folders with a SKILL.md to ~/.nar/skills or ./.nar/skills (also .claude/skills and .agents/skills).");
                } else {
                    let mut s = String::from("**Skills** — invoke with /<name>, or the model loads them when relevant:\n");
                    for sk in &self.skills.list {
                        s.push_str(&format!("- **{}** ({}) — {}\n", sk.name, sk.scope, sk.description));
                    }
                    self.info(s);
                }
            }
            "mcp" => {
                if self.mcp_summary.is_empty() {
                    self.info("No MCP servers configured. Add them to ~/.nar/mcp.json, ./.nar/mcp.json or ./.mcp.json:\n```json\n{ \"mcpServers\": { \"name\": { \"command\": \"npx\", \"args\": [\"-y\", \"some-mcp-server\"] } } }\n```");
                } else {
                    let mut s = String::from("**MCP servers**\n");
                    for (name, state, tools, src) in &self.mcp_summary {
                        let st = match state {
                            ServerState::Starting => "starting…".to_string(),
                            ServerState::Ready => format!("ready · {} tools", tools.len()),
                            ServerState::Failed(e) => format!("failed: {e}"),
                        };
                        s.push_str(&format!("- **{name}** — {st} _({src})_\n"));
                        if !tools.is_empty() {
                            s.push_str(&format!("  {}\n", tools.join(", ")));
                        }
                    }
                    self.info(s);
                }
            }
            "init" => {
                let prompt = "Analyze this repository and create (or update) an AGENTS.md file at the project root that helps AI coding agents work here. Include: a short project overview, layout of important directories, build/test/lint commands, code style conventions, and any gotchas you discover. Keep it concise and accurate — verify commands by reading config files.";
                self.send_user(prompt.into());
            }
            "copy" => {
                let last = self.session.transcript.iter().rev().find_map(|e| match e {
                    Entry::Assistant { text } => Some(text.clone()),
                    _ => None,
                });
                match last {
                    Some(t) => self.copy(&t, term),
                    None => self.toast_err("No response to copy yet"),
                }
            }
            other => {
                if let Some(skill) = self.skills.get(other) {
                    let name = skill.name.clone();
                    if let Some(body) = self.skills.load(&name) {
                        let msg = if args.is_empty() {
                            format!("Use the following skill.\n\n{body}")
                        } else {
                            format!("Use the following skill for this request: {args}\n\n{body}")
                        };
                        self.push_entry(Entry::Info { text: format!("Running skill **{name}**") });
                        if self.busy.is_some() {
                            self.queue.push(msg);
                        } else {
                            if self.session.title.is_empty() {
                                self.session.title = format!("/{name} {args}").trim().to_string();
                            }
                            self.push_entry(Entry::User { text: format!("/{name} {args}").trim().to_string() });
                            self.session.items.push(agent::user_message(&msg));
                            self.start_turn();
                        }
                    }
                } else {
                    self.toast_err(format!("Unknown command /{other} — type / to see commands"));
                }
            }
        }
    }

    fn goal_command(&mut self, args: &str) {
        match args {
            "" => match &self.session.goal {
                Some(g) if !g.done => self.info(format!("**Active goal** (turn {}):\n{}", g.turns, g.text)),
                Some(g) => self.info(format!("Last goal completed:\n{}", g.text)),
                None => self.info("No goal set. Usage: /goal <objective> — nar keeps working until it's complete."),
            },
            "clear" | "stop" | "off" | "cancel" => {
                self.session.goal = None;
                self.toast("Goal cleared");
            }
            text => {
                self.session.goal = Some(Goal { text: text.to_string(), turns: 0, done: false });
                self.push_entry(Entry::Info { text: format!("◎ **Goal set** — working until complete (esc pauses, /goal clear stops)\n{text}") });
                self.send_user(format!("Goal: {text}"));
            }
        }
    }

    fn memory_command(&mut self, args: &str) {
        let mem = self.agent.ctx.memory.clone();
        let (sub, rest) = args.split_once(char::is_whitespace).map(|(a, b)| (a, b.trim())).unwrap_or((args, ""));
        match sub {
            "add" => match mem.add(rest, false) {
                Ok(()) => self.toast("Saved to project memory"),
                Err(e) => self.toast_err(e.to_string()),
            },
            "global" => match mem.add(rest, true) {
                Ok(()) => self.toast("Saved to global memory"),
                Err(e) => self.toast_err(e.to_string()),
            },
            "remove" | "forget" => match mem.remove(rest, false) {
                Ok(n) => self.toast(format!("Removed {n} entries")),
                Err(e) => self.toast_err(e.to_string()),
            },
            "clear" => match mem.remove("", false) {
                Ok(n) => self.toast(format!("Cleared {n} entries")),
                Err(e) => self.toast_err(e.to_string()),
            },
            _ => {
                let mut s = String::from("**Memory**\n");
                let files = mem.instruction_files();
                if files.is_empty() {
                    s.push_str("- Instruction files: none (create AGENTS.md, or run /init)\n");
                } else {
                    for f in files {
                        s.push_str(&format!("- Instructions: `{}`\n", config::short_path(&f)));
                    }
                }
                let notes = mem.render();
                if notes.is_empty() {
                    s.push_str("- Notes: none yet. Add with `#note`, `/memory add <text>` or `/memory global <text>`.\n");
                } else {
                    s.push('\n');
                    s.push_str(&notes);
                }
                s.push_str(&format!(
                    "\nFiles: `{}` · `{}`",
                    config::short_path(&mem.project_file),
                    config::short_path(&mem.global_file)
                ));
                self.info(s);
            }
        }
    }

    fn new_conversation(&mut self) {
        if let Some(b) = &self.busy {
            b.cancel.cancel();
        }
        self.busy = None;
        self.session.save();
        let model = self.settings.model.clone();
        self.session = SessionData { model, effort: self.settings.effort, mode: self.settings.mode, ..Default::default() };
        self.agent.grants.lock().unwrap().clear();
        self.queue.clear();
        self.after = After::Nothing;
        self.reset_transcript();
        self.toast("Started a new conversation");
    }

    fn resume(&mut self, id: &str) {
        match SessionData::load(id) {
            Some(s) => {
                if let Some(b) = &self.busy {
                    b.cancel.cancel();
                }
                self.busy = None;
                self.session.save();
                self.session = s;
                if !self.session.model.is_empty() {
                    self.settings.model = self.session.model.clone();
                }
                self.settings.effort = self.session.effort;
                self.settings.mode = self.session.mode;
                self.reset_transcript();
                let id = self.session.id.clone();
                self.info(format!("Resumed session {id}"));
            }
            None => self.toast_err(format!("No unique session matches '{id}'")),
        }
    }

    // ---- pickers ------------------------------------------------------------

    fn open_picker(&mut self, kind: PickerKind, title: &str, items: Vec<PickItem>) {
        let selected = items.iter().position(|i| i.current).unwrap_or(0);
        self.picker = Some(Picker { kind, title: title.into(), items, selected, filter: String::new(), loading: false });
    }

    fn model_items(&self) -> Vec<PickItem> {
        let mut items: Vec<PickItem> = self
            .models
            .iter()
            .map(|m| PickItem {
                label: m.display_name.clone(),
                detail: if m.display_name != m.slug { m.slug.clone() } else { String::new() },
                value: m.slug.clone(),
                current: m.slug == self.settings.model,
            })
            .collect();
        if !items.iter().any(|i| i.current) {
            items.insert(
                0,
                PickItem { label: self.settings.model.clone(), detail: "current".into(), value: self.settings.model.clone(), current: true },
            );
        }
        items
    }

    fn open_model_picker(&mut self) {
        let items = self.model_items();
        self.open_picker(PickerKind::Model, "Model", items);
        if let Some(p) = self.picker.as_mut() {
            p.loading = true;
        }
        let client = self.agent.client.clone();
        self.send_later(async move {
            match client.list_models().await {
                Ok(m) => AppMsg::Models(Ok(m)),
                Err(e) => AppMsg::Models(Err(e.to_string())),
            }
        });
    }

    fn open_session_picker(&mut self) {
        let items: Vec<PickItem> = session::list()
            .into_iter()
            .filter(|s| s.id != self.session.id)
            .take(200)
            .map(|s| PickItem {
                label: if s.title.is_empty() { "(untitled)".into() } else { s.title },
                detail: format!("{} · {} · {}", s.id, session::relative_time(s.updated), config::short_path(std::path::Path::new(&s.cwd))),
                value: s.id,
                current: false,
            })
            .collect();
        if items.is_empty() {
            self.toast("No saved sessions yet");
            return;
        }
        self.open_picker(PickerKind::Session, "Resume a session", items);
    }

    fn open_auth_picker(&mut self) {
        let s = &self.auth_status;
        let mut items = vec![PickItem {
            label: "Continue with ChatGPT".into(),
            detail: "use your ChatGPT plan (Plus, Pro, Business…)".into(),
            value: "login".into(),
            current: s.method == AuthMethod::ChatGpt,
        }];
        items.push(PickItem {
            label: "Use an OpenAI API key".into(),
            detail: "billed to your API account".into(),
            value: "apikey".into(),
            current: s.method == AuthMethod::ApiKey,
        });
        if std::env::var_os("OPENAI_API_KEY").is_some() {
            items.push(PickItem { label: "Use OPENAI_API_KEY".into(), detail: "from the environment".into(), value: "env".into(), current: false });
        }
        if s.method != AuthMethod::None {
            items.push(PickItem { label: "Sign out".into(), detail: "revoke and remove saved credentials".into(), value: "logout".into(), current: false });
        }
        let who = match (&s.email, s.method) {
            (Some(e), AuthMethod::ChatGpt) => format!("Account · {} ({e})", s.label),
            _ => format!("Account · {}", s.label),
        };
        self.open_picker(PickerKind::Auth, &who, items);
        if let Some(p) = self.picker.as_mut() {
            p.selected = 0;
        }
    }

    fn auth_action(&mut self, action: &str) {
        match action {
            "login" => {
                if self.busy.is_some() {
                    self.toast_err("Busy — finish or interrupt the current turn first");
                    return;
                }
                let auth = self.auth.clone();
                let tx = self.tx.clone();
                let waker = self.waker.clone();
                let cancel = CancellationToken::new();
                self.login_cancel = Some(cancel.clone());
                self.busy = Some(Busy { kind: BusyKind::Login, cancel: cancel.clone(), started: Instant::now() });
                self.activity = "Waiting for ChatGPT sign-in in your browser".into();
                self.rt.spawn(async move {
                    let send = |m: AppMsg| {
                        let _ = tx.send(m);
                        waker.wake();
                    };
                    match auth.begin_chatgpt_login().await {
                        Ok(p) => {
                            send(AppMsg::LoginStarted { url: p.url.clone(), display: p.display_url.clone() });
                            let task = p.task;
                            tokio::select! {
                                r = task => {
                                    let r = r.map_err(|e| e.to_string()).and_then(|x| x.map_err(|e| e.to_string()));
                                    send(AppMsg::Login(r));
                                }
                                _ = cancel.cancelled() => {}
                            }
                        }
                        Err(e) => send(AppMsg::Login(Err(e.to_string()))),
                    }
                });
            }
            "apikey" => {
                self.input_mode = InputMode::ApiKey;
                self.editor.clear();
                self.editor.masked = true;
                self.toast("Paste your OpenAI API key and press enter (esc cancels)");
            }
            "env" => {
                let auth = self.auth.clone();
                self.send_later(async move {
                    match auth.use_method(AuthMethod::None).await {
                        Ok(s) => AppMsg::Login(Ok(s)),
                        Err(e) => AppMsg::Error(e.to_string()),
                    }
                });
            }
            "logout" => {
                let auth = self.auth.clone();
                self.send_later(async move {
                    match auth.logout().await {
                        Ok(note) => AppMsg::Notice(note),
                        Err(e) => AppMsg::Error(e.to_string()),
                    }
                });
            }
            _ => {
                let s = self.auth_status.clone();
                let mut text = format!("**Signed in:** {}", s.label);
                if let Some(e) = s.email {
                    text.push_str(&format!(" ({e})"));
                }
                if s.method == AuthMethod::ChatGpt {
                    text.push_str(&format!("\nUsing ChatGPT plan · Manage usage: {}", auth::USAGE_URL));
                }
                self.info(text);
            }
        }
    }

    // ---- background messages ----------------------------------------------

    pub fn handle_msg(&mut self, msg: AppMsg, term: &mut Terminal) {
        self.dirty = true;
        match msg {
            AppMsg::Agent(ev) => self.on_agent(ev, term),
            AppMsg::LoginStarted { url, display } => {
                let opened = auth::open_browser(&url);
                self.info(format!(
                    "**Sign in with ChatGPT** — {}\n{display}\n\nWaiting for the browser… press esc to cancel.",
                    if opened { "your browser should open; if not, open this link:" } else { "open this link in a browser:" }
                ));
            }
            AppMsg::Login(r) => {
                if matches!(self.busy.as_ref().map(|b| b.kind), Some(BusyKind::Login)) {
                    self.busy = None;
                }
                self.login_cancel = None;
                match r {
                    Ok(s) => {
                        let who = s.email.clone().map(|e| format!(" as {e}")).unwrap_or_default();
                        let note = match s.method {
                            AuthMethod::ChatGpt if s.plan_usage => format!("Signed in{who}. Using ChatGPT plan — manage usage at {}", auth::USAGE_URL),
                            AuthMethod::ChatGpt => format!("Signed in{who}, but ChatGPT plan usage wasn't granted. Run /auth again to enable it, or use an API key."),
                            _ => format!("Authentication set: {}", s.label),
                        };
                        self.auth_status = s;
                        self.info(note);
                        self.models.clear();
                    }
                    Err(e) => self.error(format!("Sign-in failed: {e}")),
                }
            }
            AppMsg::Auth(s) => self.auth_status = s,
            AppMsg::Models(r) => match r {
                Ok(m) => {
                    self.models = m;
                    if let Some(p) = self.picker.as_ref() {
                        if p.kind == PickerKind::Model {
                            let items = self.model_items();
                            if let Some(p) = self.picker.as_mut() {
                                p.items = items;
                                p.loading = false;
                                p.selected = p.items.iter().position(|i| i.current).unwrap_or(0);
                            }
                        }
                    }
                }
                Err(e) => {
                    if let Some(p) = self.picker.as_mut() {
                        p.loading = false;
                    }
                    self.toast_err(format!("Couldn't list models: {e}"));
                }
            },
            AppMsg::McpChanged => {
                let mcp = self.mcp.clone();
                let summary = self.rt.block_on(async move { mcp.summary().await });
                self.mcp_summary = summary;
            }
            AppMsg::Notice(n) => {
                self.info(n);
                self.refresh_auth();
            }
            AppMsg::Error(e) => self.error(e),
        }
    }

    fn on_agent(&mut self, ev: AgentEvent, term: &mut Terminal) {
        match ev {
            AgentEvent::TextDelta(d) => {
                self.activity = "Responding".into();
                self.stream_reasoning = None;
                match self.stream_text {
                    Some(i) if i + 1 == self.session.transcript.len() => {
                        if let Some(Entry::Assistant { text }) = self.session.transcript.get_mut(i) {
                            text.push_str(&d);
                        }
                        self.touch(i);
                    }
                    _ => {
                        let i = self.push_entry(Entry::Assistant { text: d.trim_start_matches('\n').to_string() });
                        self.stream_text = Some(i);
                    }
                }
            }
            AgentEvent::ReasoningDelta(d) => {
                self.activity = "Thinking".into();
                match self.stream_reasoning {
                    Some(i) if i + 1 == self.session.transcript.len() => {
                        if let Some(Entry::Reasoning { text }) = self.session.transcript.get_mut(i) {
                            text.push_str(&d);
                        }
                        self.touch(i);
                    }
                    _ => {
                        let i = self.push_entry(Entry::Reasoning { text: d });
                        self.stream_reasoning = Some(i);
                        self.stream_text = None;
                    }
                }
            }
            AgentEvent::ReasoningBreak => {
                if let Some(i) = self.stream_reasoning {
                    if let Some(Entry::Reasoning { text }) = self.session.transcript.get_mut(i) {
                        text.push_str("\n\n");
                    }
                }
            }
            AgentEvent::ToolStart { call_id, title } => {
                self.stream_text = None;
                self.stream_reasoning = None;
                self.activity = format!("Running {}", crate::tools::one_line(&title, 60));
                self.push_entry(Entry::Tool { call_id, title, summary: String::new(), detail: vec![], status: ToolStatus::Running });
            }
            AgentEvent::ToolEnd { call_id, outcome, denied } => {
                if let Some(i) = self.session.transcript.iter().rposition(|e| matches!(e, Entry::Tool { call_id: c, .. } if *c == call_id)) {
                    if let Some(Entry::Tool { summary, detail, status, .. }) = self.session.transcript.get_mut(i) {
                        *summary = outcome.summary;
                        *detail = outcome.detail;
                        *status = if denied {
                            ToolStatus::Denied
                        } else if outcome.ok {
                            ToolStatus::Ok
                        } else {
                            ToolStatus::Failed
                        };
                    }
                    self.touch(i);
                }
                self.activity = "Thinking".into();
            }
            AgentEvent::Permission { title, kind, reply } => {
                self.activity = "Waiting for permission".into();
                self.permission = Some(PendingPermission { title, kind, reply: Some(reply) });
                self.scroll = 0;
                term.notify("nar", "Permission needed");
            }
            AgentEvent::Items(items) => {
                self.session.items.extend(items);
            }
            AgentEvent::Usage(u) => {
                self.session.usage = u;
                self.session.context_tokens = u.input_tokens + u.output_tokens;
                self.session.total_tokens += u.total_tokens;
            }
            AgentEvent::Todos(t) => self.session.todos = t,
            AgentEvent::GoalComplete(summary) => {
                if let Some(g) = self.session.goal.as_mut() {
                    g.done = true;
                }
                self.push_entry(Entry::Info { text: format!("◎ **Goal complete** — {summary}") });
                term.notify("nar", "Goal complete");
            }
            AgentEvent::TurnDone { error, cancelled } => {
                self.busy = None;
                self.activity.clear();
                self.stream_text = None;
                self.stream_reasoning = None;
                if let Some(mut p) = self.permission.take() {
                    if let Some(tx) = p.reply.take() {
                        let _ = tx.send(PermissionReply::Deny);
                    }
                }
                if let Some(e) = &error {
                    self.error(e.clone());
                }
                if cancelled {
                    self.push_entry(Entry::Info { text: "Interrupted.".into() });
                }
                self.session.save();
                if error.is_none() && !cancelled {
                    self.after_turn();
                } else if error.is_some() {
                    if let Some(g) = &self.session.goal {
                        if !g.done {
                            self.info("Goal paused after an error. Send a message to continue.");
                        }
                    }
                }
            }
            AgentEvent::Compacted { summary, usage: _, error } => {
                self.busy = None;
                self.activity.clear();
                match error {
                    Some(e) => {
                        self.error(format!("Compaction failed: {e}"));
                        self.after = After::Nothing;
                    }
                    None => {
                        let note = format!(
                            "This conversation was compacted to save context. Summary of everything so far:\n\n{}",
                            summary.trim()
                        );
                        self.session.items = vec![agent::user_message(&note)];
                        self.session.context_tokens = (note.len() as u64) / 4 + 2000;
                        self.session.compactions += 1;
                        self.push_entry(Entry::Info { text: format!("⇣ **Conversation compacted** ({} so far)", self.session.compactions) });
                        self.session.save();
                        match std::mem::replace(&mut self.after, After::Nothing) {
                            After::Submit(t) => self.send_user(t),
                            After::ContinueGoal => self.continue_goal(),
                            After::Nothing => {}
                        }
                    }
                }
            }
        }
    }

    fn after_turn(&mut self) {
        let goal_pending = self.session.goal.as_ref().map(|g| !g.done).unwrap_or(false);
        if goal_pending {
            if self.needs_compaction() {
                self.after = After::ContinueGoal;
                self.start_compact("the active goal and remaining work".into(), true);
                return;
            }
            self.continue_goal();
            return;
        }
        if !self.queue.is_empty() {
            let next = self.queue.remove(0);
            self.send_user(next);
            return;
        }
        if self.needs_compaction() {
            self.after = After::Nothing;
            self.start_compact(String::new(), true);
        }
    }

    fn continue_goal(&mut self) {
        let Some(g) = self.session.goal.as_mut() else { return };
        if g.done {
            return;
        }
        g.turns += 1;
        let turns = g.turns;
        if turns > 1000 {
            self.info("Goal paused after 1000 continuation turns.");
            return;
        }
        // User input typed during the goal takes priority.
        let msg = if !self.queue.is_empty() {
            let q = self.queue.remove(0);
            self.push_entry(Entry::User { text: q.clone() });
            q
        } else {
            self.push_entry(Entry::Info { text: format!("↻ Continuing toward goal · turn {turns}") });
            "Continue working toward the goal. Check what remains, keep going, and verify your work. Call goal_complete only when the goal is fully achieved.".to_string()
        };
        self.session.items.push(agent::user_message(&msg));
        self.start_turn();
    }

    /// Called every frame tick for timeouts.
    pub fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if let Some((_, t, _)) = &self.toast {
            if t.elapsed().as_secs() >= 4 {
                self.toast = None;
                self.dirty = true;
            }
        }
    }

    pub fn animating(&self) -> bool {
        self.busy.is_some()
    }

    pub fn shutdown(&mut self) {
        if let Some(b) = &self.busy {
            b.cancel.cancel();
        }
        if let Some(mut p) = self.permission.take() {
            if let Some(tx) = p.reply.take() {
                let _ = tx.send(PermissionReply::Deny);
            }
        }
        self.session.model = self.settings.model.clone();
        self.session.effort = self.settings.effort;
        self.session.mode = self.settings.mode;
        self.session.save();
        self.settings.save();
    }

    pub fn last_items(&self) -> &[Value] {
        &self.session.items
    }
}

fn copy_native(text: &str) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else {
        &[("wl-copy", &[]), ("xclip", &["-selection", "clipboard"]), ("xsel", &["--clipboard", "--input"])]
    };
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let x11 = std::env::var_os("DISPLAY").is_some();
    for (cmd, args) in candidates {
        if !cfg!(target_os = "macos") && ((*cmd == "wl-copy" && !wayland) || (*cmd != "wl-copy" && !x11)) {
            continue;
        }
        if let Ok(mut child) = Command::new(cmd).args(*args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            // Don't block the UI on clipboard owners that stay alive (xclip).
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return true;
        }
    }
    false
}
