//! The Tauri shell: the window and the commands the UI calls. The work lives in the `oah-*`
//! crates; this file only adapts them to IPC.

use std::sync::Mutex;

use oah_agents::probe::{self, Probe};
use oah_core::settings::{AgentKind, Location, Project, Settings};
use oah_core::{paths, ssh};
use serde::Serialize;
use tauri::State;

/// The settings as last saved, shared by every command.
struct AppState {
    settings: Mutex<Settings>,
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
        .manage(AppState { settings: Mutex::new(settings) })
        .invoke_handler(tauri::generate_handler![
            app_info,
            settings_get,
            settings_set,
            project_add,
            project_remove,
            ssh_config_hosts,
            provider_probe,
        ])
        .run(tauri::generate_context!())
        .expect("error while running oah");
}
