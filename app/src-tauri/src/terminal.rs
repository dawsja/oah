//! Terminals in the panel under the chat: a shell where the project is (oah-transport's pty),
//! its output streamed to the page over a channel. A terminal lives until its shell exits, the
//! page closes it, or the page that opened it is gone.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use oah_core::settings::Location;
use oah_transport::pty::{self, Pty, PtyEvent};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use crate::{AppState, CmdResult};

#[derive(Default)]
pub(crate) struct Terminals {
    open: Arc<Mutex<HashMap<String, Pty>>>,
    next: AtomicU64,
}

impl Terminals {
    fn get<T>(&self, id: &str, f: impl FnOnce(&Pty) -> Result<T, String>) -> CmdResult<T> {
        match self.open.lock().unwrap().get(id) {
            Some(p) => f(p),
            None => Err("that terminal is closed".into()),
        }
    }

    /// Ends every shell (the app is quitting).
    pub(crate) fn close_all(&self) {
        self.open.lock().unwrap().clear();
    }
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum TerminalEvent {
    Data { data: String },
    Exit { code: Option<u32> },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Opened {
    id: String,
    /// The shell's name when it's known up front (this computer); remote shells announce theirs.
    shell: Option<String>,
}

fn project_location(state: &AppState, project_id: &str) -> CmdResult<(Location, String)> {
    let s = state.settings.lock().unwrap();
    let p = s.projects.iter().find(|p| p.id == project_id).ok_or("no such project")?;
    Ok((p.location.clone(), p.path.clone()))
}

/// The shell a new terminal in the project runs, when that's known without starting one.
#[tauri::command]
pub(crate) fn terminal_shell(state: State<AppState>, project_id: String) -> CmdResult<Option<String>> {
    Ok(pty::known_shell(&project_location(&state, &project_id)?.0))
}

/// Starts a shell in `cwd` (a worktree; none: the project's folder), `cols`×`rows`.
#[tauri::command]
pub(crate) fn terminal_open(
    app: AppHandle,
    state: State<AppState>,
    project_id: String,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
    on_event: Channel<TerminalEvent>,
) -> CmdResult<Opened> {
    let (location, path) = project_location(&state, &project_id)?;
    let spec = pty::shell_spec(&location, cwd.as_deref().filter(|c| !c.is_empty()).unwrap_or(&path))?;
    let id = format!("t{}", state.terminals.next.fetch_add(1, Ordering::Relaxed) + 1);
    let open = state.terminals.open.clone();
    // Held until the terminal is in the map, so an instant exit can't be reported before it is.
    let mut map = open.lock().unwrap();
    let (key, app) = (id.clone(), app.clone());
    let pty = Pty::spawn(&spec, cols, rows, move |e| {
        let (event, last) = match e {
            PtyEvent::Data(data) => (TerminalEvent::Data { data }, false),
            PtyEvent::Exit(code) => (TerminalEvent::Exit { code }, true),
        };
        // A page that's gone (reloaded, closed) can't show it anymore: end the shell.
        if on_event.send(event).is_err() || last {
            let terminals = &app.state::<AppState>().terminals;
            let gone = terminals.open.lock().unwrap().remove(&key);
            drop(gone);
        }
    })?;
    map.insert(id.clone(), pty);
    Ok(Opened { id, shell: spec.shell })
}

/// Keys typed (or text pasted) into the terminal.
#[tauri::command]
pub(crate) fn terminal_write(state: State<AppState>, id: String, data: String) -> CmdResult<()> {
    state.terminals.get(&id, |p| p.write(data.as_bytes()))
}

#[tauri::command]
pub(crate) fn terminal_resize(state: State<AppState>, id: String, cols: u16, rows: u16) -> CmdResult<()> {
    state.terminals.get(&id, |p| p.resize(cols, rows))
}

/// Ends the shell (its exit still arrives on the channel).
#[tauri::command]
pub(crate) fn terminal_close(state: State<AppState>, id: String) {
    let _ = state.terminals.get(&id, |p| {
        p.kill();
        Ok(())
    });
}
