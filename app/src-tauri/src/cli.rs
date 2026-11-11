//! A chat in its agent's own CLI: the CLI runs in a terminal where the project is, on the chat's
//! session, and its output is streamed to the page over a channel like a shell's. Who has the
//! session (the chat's driver or the CLI) is `Chats`' to say; these commands only adapt it.

use oah_transport::pty::PtyEvent;
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use crate::terminal::TerminalEvent;
use crate::{off_main, AppState, CmdResult};

/// Chats open in their agent's CLI.
#[tauri::command]
pub(crate) async fn chat_cli(app: AppHandle) -> Vec<String> {
    off_main(app, |_, state| state.chats.cli_ids()).await.unwrap_or_default()
}

/// Opens the chat's session in its agent's CLI, `cols`×`rows`. `force` stops an agent that's
/// working in the chat first; without it that's refused.
#[tauri::command]
pub(crate) async fn cli_open(
    app: AppHandle,
    chat_id: String,
    cols: u16,
    rows: u16,
    force: bool,
    on_event: Channel<TerminalEvent>,
) -> CmdResult<()> {
    off_main(app, move |app, state| {
        let settings = state.settings.lock().unwrap().clone();
        state.chats.cli_open(app, &settings, &chat_id, cols, rows, force, move |e| {
            let event = match e {
                PtyEvent::Data(data) => TerminalEvent::Data { data },
                PtyEvent::Exit(code) => TerminalEvent::Exit { code },
            };
            // A page that's gone (reloaded, closed) can't show it anymore: the CLI ends.
            on_event.send(event).is_ok()
        })
    })
    .await?
}

/// Keys typed (or text pasted) into the chat's CLI.
#[tauri::command]
pub(crate) fn cli_write(state: State<AppState>, chat_id: String, data: String) -> CmdResult<()> {
    state.chats.cli_write(&chat_id, &data)
}

#[tauri::command]
pub(crate) fn cli_resize(state: State<AppState>, chat_id: String, cols: u16, rows: u16) -> CmdResult<()> {
    state.chats.cli_resize(&chat_id, cols, rows)
}

/// Ends the chat's CLI; the chat takes messages again once this returns.
#[tauri::command]
pub(crate) async fn cli_close(app: AppHandle, chat_id: String) -> CmdResult<()> {
    off_main(app, move |app, state| state.chats.cli_close(app, &chat_id)).await
}
