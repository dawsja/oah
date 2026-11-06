// The desktop shell: owns the window, the `oah serve` engine process (here, or
// on WSL / an SSH host) and the session index, and relays engine events to the webview over a channel.
// Everything the agent does happens in the engine — the same code as the CLI.

use oah_desktop_core::{engine, paths, prefs, remote, remote::Target, sessions, sessions::SessionIndex};
use serde::Serialize;
use serde_json::{Map, Value};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::{Manager, RunEvent, State};

#[derive(Default)]
struct App {
    engine: Mutex<Option<engine::Engine>>,
    sessions: SessionIndex,
}

/// Environment for the engine: the login shell's PATH on Linux, so the
/// agent's tools find what they would in a terminal.
fn engine_env() -> &'static Vec<(OsString, OsString)> {
    static ENV: OnceLock<Vec<(OsString, OsString)>> = OnceLock::new();
    ENV.get_or_init(|| match engine::login_shell_path(Duration::from_secs(4)) {
        Some(p) => vec![("PATH".into(), p)],
        None => vec![],
    })
}

fn locate() -> Result<engine::EngineCommand, String> {
    let bundled = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    // Debug builds run the engine from this repo's source with Bun.
    let dev_source = cfg!(debug_assertions).then(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/index.tsx"));
    let path = engine_env().iter().find(|(k, _)| k == "PATH").map(|(_, v)| v);
    engine::locate(bundled.as_deref(), dev_source.as_deref(), path).ok_or_else(|| {
        "Couldn't find the oah engine. Install the oah CLI (https://oah.dev) or set OAH_ENGINE to an oah binary.".to_string()
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Started {
    cwd: String,
    cwd_short: String,
    engine: String,
    target: Target,
}

/// Start (or restart) the engine in `cwd`, optionally resuming a session. Its
/// events arrive on `on_event` as raw JSON lines; when it stops, a final
/// `{"t":"engine-exit",…}` line follows.
#[tauri::command]
fn engine_start(app: State<'_, App>, cwd: Option<String>, resume: Option<String>, target: Option<Target>, on_event: Channel<String>) -> Result<Started, String> {
    if let Some(t) = target.filter(|t| !t.is_local()) {
        return engine_start_remote(&app, t, cwd.unwrap_or_default(), resume, on_event);
    }
    let saved = prefs::load(&paths::desktop_prefs_file());
    let cwd = cwd
        .or_else(|| saved.get("lastCwd").and_then(Value::as_str).map(String::from))
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(paths::home_dir);
    let command = locate()?;

    // Stop the previous engine in the background: it saves its session first.
    if let Some(old) = app.engine.lock().unwrap_or_else(|e| e.into_inner()).take() {
        std::thread::spawn(move || old.shutdown(Duration::from_secs(5)));
    }

    let lines = on_event.clone();
    let started = engine::Engine::spawn(
        &command,
        &cwd,
        resume.as_deref(),
        engine_env(),
        move |line| {
            let _ = lines.send(line);
        },
        move |exit| {
            let ev = serde_json::json!({ "t": "engine-exit", "code": exit.code, "stderr": exit.stderr });
            let _ = on_event.send(ev.to_string());
        },
    )
    .map_err(|e| format!("Couldn't start the engine ({}): {e}", command.describe))?;

    let cwd_s = cwd.to_string_lossy().into_owned();
    let mut patch = Map::new();
    patch.insert("lastCwd".into(), Value::String(cwd_s.clone()));
    let _ = prefs::update(&paths::desktop_prefs_file(), patch);

    *app.engine.lock().unwrap_or_else(|e| e.into_inner()) = Some(started);
    Ok(Started { cwd_short: paths::short_path(&cwd_s), cwd: cwd_s, engine: command.describe, target: Target::Local })
}

/// The engine on a WSL distro or SSH host. `cwd` is a path there (empty: its
/// home folder); the UI learns the real one from the engine's hello.
fn engine_start_remote(app: &State<'_, App>, target: Target, cwd: String, resume: Option<String>, on_event: Channel<String>) -> Result<Started, String> {
    if let Some(old) = app.engine.lock().unwrap_or_else(|e| e.into_inner()).take() {
        std::thread::spawn(move || old.shutdown(Duration::from_secs(5)));
    }
    let lines = on_event.clone();
    let started = engine::Engine::spawn_remote(
        &target,
        &cwd,
        resume.as_deref(),
        move |line| {
            let _ = lines.send(line);
        },
        move |exit| {
            let ev = serde_json::json!({ "t": "engine-exit", "code": exit.code, "stderr": exit.stderr });
            let _ = on_event.send(ev.to_string());
        },
    )
    .map_err(|e| format!("Couldn't connect to {}: {e}", target.label()))?;
    let engine = started.command.describe.clone();
    *app.engine.lock().unwrap_or_else(|e| e.into_inner()) = Some(started);
    Ok(Started { cwd_short: if cwd.is_empty() { "~".into() } else { cwd.clone() }, cwd, engine, target })
}

/// Send one op to the engine (a JSON object; see `src/serve.ts`).
#[tauri::command]
fn engine_send(app: State<'_, App>, op: String) -> Result<(), String> {
    let guard = app.engine.lock().unwrap_or_else(|e| e.into_inner());
    let e = guard.as_ref().ok_or("The engine isn't running")?;
    e.send(&op).map_err(|e| format!("The engine isn't responding: {e}"))
}

#[tauri::command]
fn engine_stop(app: State<'_, App>) {
    if let Some(e) = app.engine.lock().unwrap_or_else(|e| e.into_inner()).take() {
        e.shutdown(Duration::from_secs(5));
    }
}

/// Saved sessions here, or on a WSL distro / SSH host.
#[tauri::command(async)]
fn list_sessions(app: State<'_, App>, target: Option<Target>) -> Result<Vec<sessions::SessionMeta>, String> {
    match target.filter(|t| !t.is_local()) {
        None => Ok(app.sessions.list(&paths::sessions_dir())),
        Some(t) => remote::run(&t, remote::SESSIONS, &[], Duration::from_secs(20)).map(|out| sessions::from_remote(&out)),
    }
}

#[tauri::command(async)]
fn delete_session(target: Option<Target>, id: String) -> Result<(), String> {
    if !remote::valid_session_id(&id) {
        return Err(format!("Not a session id: {id}"));
    }
    match target.filter(|t| !t.is_local()) {
        None => sessions::delete(&paths::sessions_dir(), &id).map_err(|e| e.to_string()),
        Some(t) => remote::run(&t, remote::DELETE_SESSION, &[&id], Duration::from_secs(20)).map(|_| ()),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Targets {
    /// WSL distributions (Windows).
    wsl: Vec<String>,
    /// Hosts from ~/.ssh/config.
    ssh_config: Vec<String>,
}

#[tauri::command(async)]
fn remote_targets() -> Targets {
    let config = std::fs::read_to_string(paths::home_dir().join(".ssh").join("config")).unwrap_or_default();
    Targets { wsl: remote::wsl_distros(), ssh_config: remote::ssh_config_hosts(&config) }
}

/// Is oah installed there, and new enough?
#[tauri::command(async)]
fn remote_check(target: Target) -> Result<remote::Check, String> {
    remote::run(&target, remote::CHECK, &[], Duration::from_secs(30)).map(|out| remote::parse_check(&out))
}

/// Install (or update) the oah CLI there with the install script.
#[tauri::command(async)]
fn remote_install(target: Target) -> Result<remote::Check, String> {
    remote::run(&target, remote::INSTALL, &[], Duration::from_secs(300))?;
    remote_check(target)
}

/// A folder there and its subfolders (for the folder browser).
#[tauri::command(async)]
fn remote_ls(target: Target, path: String) -> Result<remote::Listing, String> {
    remote::run(&target, remote::LIST_DIR, &[&path], Duration::from_secs(20)).map(|out| remote::parse_listing(&out))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImageData {
    name: String,
    media_type: String,
    data: String,
}

/// An image file as base64, to hand to an engine that can't read this
/// machine's files (one on WSL or over SSH).
#[tauri::command(async)]
fn read_image(path: String) -> Result<ImageData, String> {
    let p = PathBuf::from(&path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    let media_type = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => return Err(format!("Not an image: {path}")),
    };
    let bytes = std::fs::read(&p).map_err(|e| format!("Couldn't read {path}: {e}"))?;
    if bytes.len() > 5 * 1024 * 1024 {
        return Err(format!("{} is over 5 MB", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(path)));
    }
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(ImageData { name, media_type: media_type.into(), data: base64(&bytes) })
}

fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() { A[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Info {
    version: String,
    /// How this copy was installed (`appimage`, `deb`, `rpm`, `nsis`, `msi`, `app`), for
    /// the updater: some installs update silently, others need a restart or a password.
    bundle: Option<&'static str>,
    os: &'static str,
    home: String,
    home_short: String,
    home_dir: String,
    sessions_dir: String,
    oah_home_env: bool,
    xdg_env: bool,
}

fn bundle_name() -> Option<&'static str> {
    use tauri::utils::config::BundleType;
    Some(match tauri::utils::platform::bundle_type()? {
        BundleType::AppImage => "appimage",
        BundleType::Deb => "deb",
        BundleType::Rpm => "rpm",
        BundleType::Nsis => "nsis",
        BundleType::Msi => "msi",
        _ => return None,
    })
}

#[tauri::command]
fn app_info(app: tauri::AppHandle) -> Info {
    let home = paths::oah_home().to_string_lossy().into_owned();
    Info {
        version: app.package_info().version.to_string(),
        bundle: bundle_name(),
        os: std::env::consts::OS,
        home_short: paths::short_path(&home),
        home,
        home_dir: paths::home_dir().to_string_lossy().into_owned(),
        sessions_dir: paths::sessions_dir().to_string_lossy().into_owned(),
        oah_home_env: std::env::var_os("OAH_HOME").is_some_and(|v| !v.is_empty()),
        xdg_env: std::env::var_os("XDG_CONFIG_HOME").is_some_and(|v| !v.is_empty()),
    }
}

#[tauri::command]
fn prefs_get() -> Value {
    Value::Object(prefs::load(&paths::desktop_prefs_file()))
}

#[tauri::command]
fn prefs_set(patch: Map<String, Value>) -> Result<Value, String> {
    prefs::update(&paths::desktop_prefs_file(), patch).map(Value::Object).map_err(|e| e.to_string())
}

/// Open the oah data folder, the workspace, or `path` in the file manager.
/// On a WSL distro, paths open through \\wsl.localhost\<distro>.
#[tauri::command]
fn open_folder(app: tauri::AppHandle, which: String, path: Option<String>, target: Option<Target>) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    if let Some(p) = path {
        let p = match target {
            Some(Target::Wsl { distro }) => format!(r"\\wsl.localhost\{distro}{}", p.replace('/', r"\")),
            Some(Target::Ssh { .. }) => return Err("Folders on SSH hosts can't be opened in the file manager".into()),
            _ => p,
        };
        return app.opener().open_path(p, None::<&str>).map_err(|e| e.to_string());
    }
    let path = match which.as_str() {
        "home" => paths::oah_home(),
        "cwd" => app
            .state::<App>()
            .engine
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|e| e.cwd.clone())
            .ok_or("No workspace is open")?,
        _ => return Err(format!("unknown folder {which}")),
    };
    app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(App::default())
        .invoke_handler(tauri::generate_handler![
            engine_start,
            engine_send,
            engine_stop,
            list_sessions,
            delete_session,
            remote_targets,
            remote_check,
            remote_install,
            remote_ls,
            read_image,
            app_info,
            prefs_get,
            prefs_set,
            open_folder
        ])
        .build(tauri::generate_context!())
        .expect("error while building the oah desktop app");
    app.run(|handle, event| {
        // Quitting: let the engine save the session before the process exits.
        if let RunEvent::Exit = event {
            if let Some(e) = handle.state::<App>().engine.lock().unwrap_or_else(|e| e.into_inner()).take() {
                e.shutdown(Duration::from_secs(5));
            }
        }
    });
}
