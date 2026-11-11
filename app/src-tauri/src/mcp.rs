//! MCP for other agents: the app's side of `oah_mcp`. [`AppHost`] gives the MCP tools the same
//! chats, settings and git the window uses, so a chat started over MCP shows (and streams) in the
//! sidebar, and either side can answer its approvals.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use oah_agents::driver::Prompt;
use oah_agents::event::{Decision, ModelOption, SlashCommand};
use oah_agents::{account, probe};
use oah_core::chats::ChatMeta;
use oah_core::paths;
use oah_core::settings::{AgentKind, Location, PermissionMode, Settings};
use oah_git::worktree;
use oah_git::Repo;
use oah_mcp::host::{AgentCheck, Host, HostResult, NewChat, WorktreeInfo};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;

pub struct AppHost {
    app: AppHandle,
}

impl AppHost {
    fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }

    fn repo(&self, project_id: &str, cwd: Option<&str>) -> HostResult<Repo> {
        let s = self.settings();
        let p = s.projects.iter().find(|p| p.id == project_id).ok_or("no such project: list_projects shows them")?;
        Ok(Repo::new(p.location.clone(), cwd.filter(|c| !c.is_empty()).map(str::to_string).unwrap_or_else(|| p.path.clone())))
    }

    /// The sidebar lists chats again (one was added, renamed or removed from here).
    fn changed(&self) {
        let _ = self.app.emit("chats-changed", ());
    }
}

fn info(w: worktree::Worktree) -> WorktreeInfo {
    WorktreeInfo { path: w.path, branch: w.branch, main: w.main }
}

impl Host for AppHost {
    fn settings(&self) -> Settings {
        self.state().settings.lock().unwrap().clone()
    }

    fn worktrees(&self, project_id: &str) -> HostResult<Vec<WorktreeInfo>> {
        Ok(worktree::worktrees(&self.repo(project_id, None)?)?.into_iter().map(info).collect())
    }

    fn create_worktree(&self, project_id: &str, branch: &str) -> HostResult<WorktreeInfo> {
        let s = self.settings();
        let git = s.projects.iter().find(|p| p.id == project_id).map(|p| p.git.clone()).ok_or("no such project")?;
        let r = self.repo(project_id, None)?;
        let path = worktree::default_dir(&r, &git.worktree_dir, branch);
        let base = worktree::default_base(&r, &git.base_branch);
        worktree::add(&r, &path, branch.trim(), &base).map(info)
    }

    fn check_agent(&self, kind: AgentKind, location: &Location) -> AgentCheck {
        let s = self.settings();
        let binary = s.providers.get(kind).binary_on(location);
        let env = crate::secrets::resolve(&crate::secrets::scope(kind), &s.providers.get(kind).env);
        let acct = account::account(kind, &binary, location, &env);
        // `probe` looks on this computer only; elsewhere, a sign-in answer means it's there.
        let (installed, version, perr) = match location {
            Location::Local => {
                let p = probe::probe(kind, &binary);
                (p.found, p.version, p.error.or(p.outdated))
            }
            _ => (acct.error.is_none(), None, None),
        };
        AgentCheck {
            installed,
            version,
            signed_in: (acct.error.is_none()).then_some(acct.signed_in),
            account: acct.label.map(|l| match acct.plan {
                Some(p) => format!("{l} ({p})"),
                None => l,
            }),
            error: perr.or(acct.error),
        }
    }

    fn catalog(&self, kind: AgentKind, project_id: Option<&str>) -> HostResult<(Vec<ModelOption>, Vec<SlashCommand>)> {
        crate::catalog(&self.settings(), kind, project_id).map(|c| (c.models, c.commands))
    }

    fn chats(&self) -> Vec<ChatMeta> {
        self.state().chats.store.list()
    }

    fn running(&self) -> Vec<String> {
        self.state().chats.running()
    }

    fn busy(&self, chat_id: &str) -> bool {
        self.state().chats.busy().iter().any(|c| c == chat_id)
    }

    fn create(&self, chat: NewChat) -> HostResult<ChatMeta> {
        let m = self.state().chats.create_in(&self.settings(), chat)?;
        self.changed();
        Ok(m)
    }

    fn send(&self, chat_id: &str, prompt: Prompt, permission: Option<PermissionMode>) -> HostResult<()> {
        let settings = self.settings();
        self.state().chats.send(&self.app, &settings, chat_id, prompt, permission)
    }

    fn interrupt(&self, chat_id: &str) -> HostResult<()> {
        self.state().chats.interrupt(chat_id)
    }

    fn answer(&self, chat_id: &str, request_id: &str, decision: Decision) -> HostResult<()> {
        self.state().chats.answer(chat_id, request_id, decision)
    }

    fn answer_question(&self, chat_id: &str, request_id: &str, answers: BTreeMap<String, Vec<String>>) -> HostResult<()> {
        self.state().chats.answer_question(chat_id, request_id, answers)
    }

    fn set_model(&self, chat_id: &str, model: &str) -> HostResult<()> {
        self.state().chats.set_model(chat_id, model)?;
        self.changed();
        Ok(())
    }

    fn set_effort(&self, chat_id: &str, effort: &str) -> HostResult<()> {
        self.state().chats.set_effort(chat_id, effort)?;
        self.changed();
        Ok(())
    }

    fn set_permission(&self, chat_id: &str, mode: PermissionMode) -> HostResult<()> {
        self.state().chats.set_permission(&self.app, chat_id, mode, true)
    }

    fn permission(&self, chat_id: &str) -> PermissionMode {
        self.state().chats.permission_for(&self.settings(), chat_id)
    }

    fn compact(&self, chat_id: &str, instructions: &str) -> HostResult<()> {
        let settings = self.settings();
        self.state().chats.compact(&self.app, &settings, chat_id, instructions, None)
    }

    fn clear(&self, chat_id: &str) -> HostResult<()> {
        self.state().chats.clear(&self.app, chat_id)
    }

    fn rename(&self, chat_id: &str, title: &str) -> HostResult<ChatMeta> {
        let m = self.state().chats.edit(chat_id, |m| m.title = title.trim().to_string())?;
        self.changed();
        Ok(m)
    }

    fn close(&self, chat_id: &str) {
        // Open in its agent's own CLI in the window: that's what has the session, and ends.
        self.state().chats.cli_take(&self.app, chat_id);
        self.state().chats.close(chat_id);
        self.changed();
    }

    fn delete(&self, chat_id: &str) -> HostResult<()> {
        self.state().chats.delete(chat_id)?;
        self.changed();
        Ok(())
    }

    fn events(&self, chat_id: &str) -> HostResult<Vec<serde_json::Value>> {
        self.state().chats.store.events(chat_id).map_err(|e| e.to_string())
    }

    fn git_status(&self, project_id: &str, cwd: Option<&str>) -> HostResult<serde_json::Value> {
        let status = oah_git::status(&self.repo(project_id, cwd)?)?;
        serde_json::to_value(status).map_err(|e| e.to_string())
    }

    fn git_diff(&self, project_id: &str, cwd: Option<&str>, path: &str) -> HostResult<String> {
        let r = self.repo(project_id, cwd)?;
        let status = oah_git::status(&r)?;
        if status.untracked.iter().any(|c| c.path == path) {
            return oah_git::diff(&r, path, false, true);
        }
        // Staged and unstaged changes, in that order.
        let staged = oah_git::diff(&r, path, true, false).unwrap_or_default();
        let unstaged = oah_git::diff(&r, path, false, false)?;
        Ok([staged, unstaged].into_iter().filter(|d| !d.trim().is_empty()).collect::<Vec<_>>().join("\n"))
    }
}

/// Opens MCP's local socket for the app's lifetime, and HTTP when it's on.
pub fn listen(app: &AppHandle) {
    let hub = app.state::<AppState>().chats.hub.clone();
    let ctx = Arc::new(oah_mcp::Ctx::new(Arc::new(AppHost { app: app.clone() }), hub));
    if let Err(e) = oah_mcp::server::listen(&paths::mcp_socket(), ctx.clone()) {
        eprintln!("oah: couldn't open the MCP socket {}: {e}", paths::mcp_socket());
    }
    HTTP.lock().unwrap().ctx = Some(ctx);
    apply_http(&app.state::<AppState>().settings.lock().unwrap().mcp);
}

// ---- HTTP ------------------------------------------------------------------------------------

#[derive(Default)]
struct Http {
    ctx: Option<Arc<oah_mcp::Ctx>>,
    /// The token and sessions: made when HTTP is first turned on, kept while oah runs.
    state: Option<Arc<oah_mcp::http::HttpState>>,
    /// The keychain keeps the token (else it's new each time oah starts).
    kept: bool,
    server: Option<oah_mcp::http::Server>,
    /// The server listens for other devices too.
    lan: bool,
    /// Why it isn't listening (the port is taken…).
    error: Option<String>,
}

static HTTP: Mutex<Http> = Mutex::new(Http { ctx: None, state: None, kept: false, server: None, lan: false, error: None });

fn http_state(h: &mut Http) -> Arc<oah_mcp::http::HttpState> {
    h.state
        .get_or_insert_with(|| {
            let (token, kept) = crate::secrets::mcp_token();
            h.kept = kept;
            Arc::new(oah_mcp::http::HttpState::new(token))
        })
        .clone()
}

/// Starts, moves or stops the HTTP listener to match the settings: it runs while MCP and HTTP
/// are both on.
pub fn apply_http(mcp: &oah_core::settings::Mcp) {
    let mut h = HTTP.lock().unwrap();
    let want = (mcp.enabled && mcp.http).then_some((mcp.http_port, mcp.lan()));
    let running = h.server.as_ref().map(|s| (s.addr().port(), h.lan));
    if want.is_some() && want == running {
        return;
    }
    h.server = None;
    h.error = None;
    let (Some((port, lan)), Some(ctx)) = (want, h.ctx.clone()) else { return };
    let state = http_state(&mut h);
    match oah_mcp::http::start(port, lan, ctx, state) {
        Ok(server) => {
            h.server = Some(server);
            h.lan = lan;
        }
        Err(e) => h.error = Some(format!("couldn't listen on port {port}: {e}")),
    }
}

/// The HTTP token, for the client snippets.
#[tauri::command]
pub fn mcp_token() -> String {
    let mut h = HTTP.lock().unwrap();
    let state = http_state(&mut h);
    state.token()
}

/// A new HTTP token: clients using the old one are disconnected.
#[tauri::command]
pub fn mcp_token_regenerate() -> String {
    let mut h = HTTP.lock().unwrap();
    let state = http_state(&mut h);
    let token = oah_mcp::http::new_token();
    h.kept = crate::secrets::save_mcp_token(&token);
    state.set_token(token.clone());
    token
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpStatus {
    /// `http://127.0.0.1:<port>/mcp` while listening.
    url: Option<String>,
    /// `http://<ip>:<port>/mcp` for 127.0.0.1 and each private/VPN address on this computer.
    urls: Vec<String>,
    error: Option<String>,
    /// Sessions used in the last hour.
    sessions: usize,
    /// The token is in the keychain (else it changes when oah restarts).
    token_kept: bool,
    activity: Vec<oah_mcp::http::Activity>,
}

fn http_status() -> HttpStatus {
    let h = HTTP.lock().unwrap();
    let port = h.server.as_ref().map(|s| s.addr().port());
    let url = port.map(|p| format!("http://127.0.0.1:{p}{}", oah_mcp::http::PATH));
    let lan = h.lan;
    let urls = match port {
        Some(p) => std::iter::once("127.0.0.1".to_string())
            .chain(if lan { crate::remote::addresses().into_iter().map(|a| a.ip).collect() } else { Vec::new() })
            .map(|ip| format!("http://{ip}:{p}{}", oah_mcp::http::PATH))
            .collect(),
        None => Vec::new(),
    };
    HttpStatus {
        url,
        urls,
        error: h.error.clone(),
        sessions: h.state.as_ref().map(|s| s.sessions()).unwrap_or(0),
        token_kept: h.kept,
        activity: h.state.as_ref().map(|s| s.activity()).unwrap_or_default(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    /// Where `oah mcp` connects.
    socket: String,
    /// MCP clients connected now.
    clients: usize,
    /// This executable, for the client config snippets.
    exe: String,
    http: HttpStatus,
}

#[tauri::command]
pub fn mcp_status() -> McpStatus {
    McpStatus {
        socket: paths::mcp_socket(),
        clients: oah_mcp::server::CLIENTS.load(std::sync::atomic::Ordering::SeqCst),
        // An AppImage runs from a temporary mount; the file itself is what a client should run.
        exe: std::env::var("APPIMAGE")
            .ok()
            .filter(|p| !p.is_empty())
            .or_else(|| std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "oah".into()),
        http: http_status(),
    }
}
