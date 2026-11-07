//! Voice: turn speech to text on (download the model, progress as `voice-state` events), off
//! (stop the download or delete the model), and transcribe a recording from the composer.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine;
use oah_core::paths;
use oah_voice::VoiceState;
use tauri::{AppHandle, Emitter, Manager};

use crate::{err, AppState, CmdResult};

#[derive(Default)]
pub struct Voice {
    state: Mutex<Option<VoiceState>>,
    cancel: Mutex<Option<Arc<AtomicBool>>>,
}

/// Whether the microphone may be used: voice is on (turning it on in Settings is the consent).
pub static ON: AtomicBool = AtomicBool::new(false);

fn put(app: &AppHandle, s: VoiceState) {
    ON.store(matches!(s, VoiceState::Ready), Ordering::SeqCst);
    *app.state::<AppState>().voice.state.lock().unwrap() = Some(s.clone());
    let _ = app.emit("voice-state", s);
}

/// Saves `voice.enabled`.
fn set_enabled(app: &AppHandle, on: bool) {
    let state = app.state::<AppState>();
    let mut s = state.settings.lock().unwrap();
    if s.voice.enabled != on {
        s.voice.enabled = on;
        let _ = s.save(&paths::settings_file());
    }
}

#[tauri::command]
pub fn voice_state(app: AppHandle) -> VoiceState {
    let state = app.state::<AppState>();
    if let Some(s) = state.voice.state.lock().unwrap().clone() {
        return s;
    }
    let s = if let Some(why) = oah_voice::unsupported() {
        VoiceState::Unsupported { error: why }
    } else if state.settings.lock().unwrap().voice.enabled && oah_voice::model_path().is_some() {
        VoiceState::Ready
    } else {
        VoiceState::Off
    };
    *state.voice.state.lock().unwrap() = Some(s.clone());
    ON.store(matches!(s, VoiceState::Ready), Ordering::SeqCst);
    s
}

/// The composer's mic: oah answers WebView2's microphone prompt itself, allowed while voice is
/// on and refused otherwise, so no browser-style permission box appears. Windows' own privacy
/// switch and mic-in-use indicator still apply. (From Gond Code.)
#[cfg(windows)]
pub fn answer_microphone<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE, COREWEBVIEW2_PERMISSION_STATE_ALLOW, COREWEBVIEW2_PERMISSION_STATE_DENY};
    let _ = window.with_webview(|wv| unsafe {
        let Ok(core) = wv.controller().CoreWebView2() else { return };
        let handler = webview2_com::PermissionRequestedEventHandler::create(Box::new(|_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
            args.PermissionKind(&mut kind)?;
            if kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE {
                args.SetState(if ON.load(Ordering::SeqCst) { COREWEBVIEW2_PERMISSION_STATE_ALLOW } else { COREWEBVIEW2_PERMISSION_STATE_DENY })?;
            }
            Ok(())
        }));
        let mut token = 0i64;
        let _ = core.add_PermissionRequested(&handler, &mut token);
    });
}

#[tauri::command]
pub fn voice_enable(app: AppHandle) -> CmdResult<()> {
    if let Some(why) = oah_voice::unsupported() {
        return Err(why);
    }
    if oah_voice::model_path().is_some() {
        set_enabled(&app, true);
        put(&app, VoiceState::Ready);
        return Ok(());
    }
    let stop = Arc::new(AtomicBool::new(false));
    {
        let state = app.state::<AppState>();
        let mut g = state.voice.cancel.lock().unwrap();
        if g.is_some() {
            return Ok(()); // already downloading
        }
        *g = Some(stop.clone());
    }
    set_enabled(&app, true);
    put(&app, VoiceState::Downloading { percent: 0 });
    std::thread::spawn(move || {
        let mut last = u64::MAX;
        let result = oah_voice::download(&oah_voice::dir(), &stop, |done, total| {
            let pct = done * 100 / total.max(1);
            if pct != last {
                last = pct;
                put(&app, VoiceState::Downloading { percent: pct });
            }
        });
        {
            let state = app.state::<AppState>();
            let mut g = state.voice.cancel.lock().unwrap();
            if g.as_ref().is_some_and(|c| Arc::ptr_eq(c, &stop)) {
                *g = None;
            }
        }
        if stop.load(Ordering::SeqCst) {
            return; // turned off meanwhile
        }
        match result {
            Ok(_) => put(&app, VoiceState::Ready),
            Err(e) => {
                set_enabled(&app, false);
                put(&app, VoiceState::Error { error: e });
            }
        }
    });
    Ok(())
}

#[tauri::command]
pub fn voice_disable(app: AppHandle) -> CmdResult<()> {
    set_enabled(&app, false);
    let cancel = app.state::<AppState>().voice.cancel.lock().unwrap().take();
    match cancel {
        // A download in flight deletes its own part file as it stops.
        Some(stop) => stop.store(true, Ordering::SeqCst),
        None => oah_voice::remove()?,
    }
    put(&app, VoiceState::Off);
    Ok(())
}

/// Text from a recording: 16 kHz mono f32 samples, little-endian, base64.
#[tauri::command]
pub async fn voice_transcribe(audio: String) -> CmdResult<String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(audio.trim()).map_err(|e| format!("the recording isn't base64: {e}"))?;
    let pcm = oah_voice::samples(&bytes)?;
    tauri::async_runtime::spawn_blocking(move || oah_voice::transcribe(&pcm)).await.map_err(err)?
}
