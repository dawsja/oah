//! Chats waiting on the user, outside the window: the taskbar/dock badge with their count, and
//! a click on a notification bringing the window back on the chat it was about.

use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_notification::{Action, ActionType, NotificationExt};

use crate::{err, CmdResult};

/// The window on screen and in front.
pub(crate) fn focus_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Notifications about a chat carry its id (`extra.chatId`); a click on one, or on its
/// Answer button, focuses the window and tells the page (`notification-action`
/// `{ chatId, action }`, `action` being `tap` or `answer`).
pub(crate) fn listen(app: &AppHandle) {
    let n = app.notification();
    let answer = ActionType::builder("waiting").actions(vec![Action::builder("answer", "Answer").foreground(true).build()]).build();
    if let Err(e) = n.register_action_types(vec![answer]) {
        eprintln!("oah: notification actions: {e}");
    }
    let handle = app.clone();
    let _ = n.on_action(move |a| {
        focus_main(&handle);
        let chat = a.notification().and_then(|n| n.extra().get("chatId")).and_then(|v| v.as_str());
        if let Some(chat_id) = chat {
            let _ = handle.emit("notification-action", serde_json::json!({ "chatId": chat_id, "action": a.action_id() }));
        }
    });
}

/// How many chats wait on the user, on the taskbar or dock and in the window title. Windows has
/// no badge count, only an overlay icon: `icon` is that, square RGBA drawn by the page.
#[tauri::command]
pub(crate) fn app_set_badge(window: WebviewWindow, count: u32, icon: Option<Vec<u8>>) -> CmdResult<()> {
    #[cfg(windows)]
    {
        let image = icon.filter(|_| count > 0).and_then(|rgba| {
            let side = ((rgba.len() / 4) as f64).sqrt() as u32;
            (side > 0 && (side * side * 4) as usize == rgba.len()).then(|| tauri::image::Image::new_owned(rgba, side, side))
        });
        window.set_overlay_icon(image).map_err(err)?;
    }
    #[cfg(not(windows))]
    {
        let _ = icon;
        // Only some Linux docks show one (through libunity); elsewhere this does nothing.
        let _ = window.set_badge_count((count > 0).then_some(i64::from(count)));
    }
    let title = if count > 0 { format!("oah ({count} waiting)") } else { "oah".to_string() };
    window.set_title(&title).map_err(err)
}
