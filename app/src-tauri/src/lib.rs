//! The Tauri shell: the window and the commands the UI calls. The work lives in the `oah-*`
//! crates; this file only adapts them to IPC.

mod chats;

use std::sync::Mutex;

use oah_agents::driver::Prompt;
use oah_agents::event::Decision;
use oah_agents::probe::{self, Probe};
use oah_core::chats::{ChatMeta, ChatStore};
use oah_core::settings::{AgentKind, Location, PermissionMode, Project, Settings};
use oah_core::{paths, ssh};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

/// The settings as last saved, shared by every command.
struct AppState {
    settings: Mutex<Settings>,
    chats: chats::Chats,
}

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: &'static str,
    platform: &'static str,
    data_dir: String,
    home: String,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        platform: if cfg!(windows) {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        },
        data_dir: paths::app_dir().to_string_lossy().into_owned(),
        home: paths::home_dir().to_string_lossy().into_owned(),
    }
}

#[tauri::command]
fn settings_get(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

/// Replaces the settings with `settings` and saves them.
#[tauri::command]
fn settings_set(state: State<AppState>, settings: Settings) -> CmdResult<Settings> {
    settings.save(&paths::settings_file()).map_err(err)?;
    *state.settings.lock().unwrap() = settings.clone();
    Ok(settings)
}

#[tauri::command]
fn project_add(state: State<AppState>, location: Location, path: String) -> CmdResult<(Settings, Project)> {
    let mut s = state.settings.lock().unwrap();
    let mut next = s.clone();
    let project = next.add_project(location, &path).map_err(err)?;
    next.save(&paths::settings_file()).map_err(err)?;
    *s = next.clone();
    Ok((next, project))
}

#[tauri::command]
fn project_remove(state: State<AppState>, id: String) -> CmdResult<Settings> {
    let mut s = state.settings.lock().unwrap();
    let mut next = s.clone();
    next.remove_project(&id);
    next.save(&paths::settings_file()).map_err(err)?;
    *s = next.clone();
    Ok(next)
}

/// Hosts named in `~/.ssh/config`.
#[tauri::command]
fn ssh_config_hosts() -> Vec<String> {
    ssh::user_config_hosts()
}

/// Whether an agent CLI is installed, where, and which version. Runs off the UI thread.
#[tauri::command]
async fn provider_probe(kind: AgentKind, binary: String) -> Probe {
    tauri::async_runtime::spawn_blocking(move || probe::probe(kind, &binary))
        .await
        .unwrap_or_else(|e| Probe { found: false, path: None, version: None, error: Some(e.to_string()) })
}

// ---- Chats -----------------------------------------------------------------------------------

#[tauri::command]
fn chat_list(state: State<AppState>) -> Vec<ChatMeta> {
    state.chats.store.list()
}

#[tauri::command]
fn chat_running(state: State<AppState>) -> Vec<String> {
    state.chats.running()
}

/// A new, empty chat in `project_id` (its agent starts with the first message).
#[tauri::command]
fn chat_create(state: State<AppState>, project_id: String, agent: Option<AgentKind>, model: Option<String>, cwd: Option<String>) -> CmdResult<ChatMeta> {
    let s = state.settings.lock().unwrap().clone();
    let project = s.projects.iter().find(|p| p.id == project_id).ok_or("no such project")?;
    let agent = agent.or(project.default_agent).unwrap_or(s.providers.default_agent);
    let cwd = cwd.unwrap_or_else(|| project.path.clone());
    state.chats.create(&project_id, agent, model.as_deref().unwrap_or(""), &cwd)
}

/// The chat's events so far, to replay into the transcript.
#[tauri::command]
fn chat_events(state: State<AppState>, chat_id: String) -> CmdResult<Vec<serde_json::Value>> {
    state.chats.store.events(&chat_id).map_err(err)
}

#[tauri::command]
async fn chat_send(app: AppHandle, chat_id: String, prompt: Prompt, permission: Option<PermissionMode>) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let settings = state.settings.lock().unwrap().clone();
        state.chats.send(&app, &settings, &chat_id, prompt, permission)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
fn chat_interrupt(state: State<AppState>, chat_id: String) -> CmdResult<()> {
    state.chats.interrupt(&chat_id)
}

#[tauri::command]
fn chat_answer(state: State<AppState>, chat_id: String, request_id: String, decision: Decision) -> CmdResult<()> {
    state.chats.answer(&chat_id, &request_id, decision)
}

#[tauri::command]
fn chat_set_permission(state: State<AppState>, chat_id: String, mode: PermissionMode) -> CmdResult<()> {
    state.chats.set_permission(&chat_id, mode)
}

#[tauri::command]
fn chat_close(state: State<AppState>, chat_id: String) {
    state.chats.close(&chat_id)
}

#[tauri::command]
fn chat_delete(state: State<AppState>, chat_id: String) -> CmdResult<()> {
    state.chats.delete(&chat_id)
}

#[tauri::command]
fn chat_rename(state: State<AppState>, chat_id: String, title: String) -> CmdResult<ChatMeta> {
    let mut m = state.chats.store.get(&chat_id).map_err(err)?;
    m.title = title.trim().to_string();
    state.chats.store.save(&m).map_err(err)?;
    Ok(m)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let settings = Settings::load(&paths::settings_file()).unwrap_or_else(|e| {
        eprintln!("oah: {e}; starting with default settings");
        Settings::default()
    });
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState { settings: Mutex::new(settings), chats: chats::Chats::new(ChatStore::new(paths::chats_dir())) })
        .invoke_handler(tauri::generate_handler![
            app_info,
            settings_get,
            settings_set,
            project_add,
            project_remove,
            ssh_config_hosts,
            provider_probe,
            chat_list,
            chat_running,
            chat_create,
            chat_events,
            chat_send,
            chat_interrupt,
            chat_answer,
            chat_set_permission,
            chat_close,
            chat_delete,
            chat_rename,
        ])
        .build(tauri::generate_context!())
        .expect("error while building oah")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                app.state::<AppState>().chats.stop_all();
            }
        });
}
