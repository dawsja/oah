//! Optional, phone-local dictation. No audio is sent through the desktop RPC connection.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine;
use oah_voice::VoiceState;
use tauri::{AppHandle, Emitter, Manager, State};

const MAX_BYTES: usize = 30 * 16_000 * 4;

struct Inner {
    state: VoiceState,
    download: Option<Arc<AtomicBool>>,
    transcribing: bool,
}

pub struct Voice {
    dir: PathBuf,
    inner: Mutex<Inner>,
}

impl Voice {
    pub fn new(dir: PathBuf) -> Self {
        let state = if !cfg!(target_arch = "aarch64") {
            VoiceState::Unsupported { error: "On-device voice requires a 64-bit ARM phone.".into() }
        } else if dir.join(oah_voice::FILE).metadata().is_ok_and(|m| m.len() == oah_voice::BYTES) {
            VoiceState::Ready
        } else {
            VoiceState::Off
        };
        Self { dir, inner: Mutex::new(Inner { state, download: None, transcribing: false }) }
    }
}

fn put(app: &AppHandle, inner: &mut Inner, state: VoiceState) {
    inner.state = state.clone();
    let _ = app.emit("voice-state", state);
}

#[tauri::command]
pub fn voice_state(voice: State<Voice>) -> VoiceState {
    voice.inner.lock().unwrap().state.clone()
}

#[tauri::command]
pub fn voice_enable(app: AppHandle, voice: State<Voice>) -> Result<(), String> {
    let stop = Arc::new(AtomicBool::new(false));
    {
        let mut inner = voice.inner.lock().unwrap();
        if let VoiceState::Unsupported { error } = &inner.state {
            return Err(error.clone());
        }
        if inner.transcribing {
            return Err("Wait for transcription to finish.".into());
        }
        if let Some(cancel) = &inner.download {
            return if cancel.load(Ordering::SeqCst) {
                Err("The previous download is still stopping. Try again shortly.".into())
            } else {
                Ok(())
            };
        }
        if matches!(inner.state, VoiceState::Ready) {
            return Ok(());
        }
        inner.download = Some(stop.clone());
        put(&app, &mut inner, VoiceState::Downloading { percent: 0 });
    }
    let dir = voice.dir.clone();
    std::thread::spawn(move || {
        let mut last = u64::MAX;
        let result = oah_voice::download(&dir, &stop, |done, total| {
            let percent = (done.saturating_mul(100) / total.max(1)).min(100);
            if percent == last {
                return;
            }
            last = percent;
            let voice = app.state::<Voice>();
            let mut inner = voice.inner.lock().unwrap();
            if !stop.load(Ordering::SeqCst) {
                put(&app, &mut inner, VoiceState::Downloading { percent });
            }
        });
        let voice = app.state::<Voice>();
        let mut inner = voice.inner.lock().unwrap();
        // Serialize cleanup with a subsequent enable, including cancellation after rename.
        if stop.load(Ordering::SeqCst) {
            if let Err(e) = remove_model(&dir) {
                put(&app, &mut inner, VoiceState::Error { error: e });
            }
        } else {
            let state = match result {
                Ok(_) => VoiceState::Ready,
                Err(error) => VoiceState::Error { error },
            };
            put(&app, &mut inner, state);
        }
        inner.download = None;
    });
    Ok(())
}

fn remove_model(dir: &std::path::Path) -> Result<(), String> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Couldn't remove the voice model: {e}")),
    }
}

#[tauri::command]
pub fn voice_disable(app: AppHandle, voice: State<Voice>) -> Result<(), String> {
    let mut inner = voice.inner.lock().unwrap();
    if inner.transcribing {
        return Err("Wait for transcription to finish.".into());
    }
    if let Some(stop) = &inner.download {
        stop.store(true, Ordering::SeqCst);
    } else {
        oah_voice::unload();
        remove_model(&voice.dir)?;
    }
    if !matches!(inner.state, VoiceState::Unsupported { .. }) {
        put(&app, &mut inner, VoiceState::Off);
    }
    Ok(())
}

fn decode_audio(audio: &str) -> Result<Vec<f32>, String> {
    if audio.len() > MAX_BYTES.div_ceil(3) * 4 {
        return Err("Record up to 30 seconds at a time.".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(audio).map_err(|_| "Invalid recording.".to_string())?;
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err("Record between 0 and 30 seconds of audio.".into());
    }
    let pcm = oah_voice::samples(&bytes)?;
    if pcm.iter().any(|s| !s.is_finite() || s.abs() > 1.0) {
        return Err("Invalid audio samples.".into());
    }
    Ok(pcm)
}

#[tauri::command]
pub async fn voice_transcribe(app: AppHandle, audio: String) -> Result<String, String> {
    let pcm = decode_audio(&audio)?;
    {
        let voice = app.state::<Voice>();
        let mut inner = voice.inner.lock().unwrap();
        if !matches!(inner.state, VoiceState::Ready) {
            return Err("Download the voice model in Chat options first.".into());
        }
        if inner.transcribing {
            return Err("Already transcribing.".into());
        }
        inner.transcribing = true;
    }
    let handle = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        #[cfg(target_arch = "aarch64")]
        let result = oah_voice::transcribe_at(&handle.state::<Voice>().dir.join(oah_voice::FILE), &pcm);
        #[cfg(not(target_arch = "aarch64"))]
        let result = {
            let _ = (&handle, pcm);
            Err("On-device voice requires a 64-bit ARM phone.".into())
        };
        // Phones should not keep a 0.6B model resident between recordings.
        oah_voice::unload();
        result
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    app.state::<Voice>().inner.lock().unwrap().transcribing = false;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_recordings_before_loading_the_model() {
        let encode = |values: &[f32]| {
            base64::engine::general_purpose::STANDARD.encode(values.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>())
        };
        assert_eq!(decode_audio(&encode(&[0.0, -1.0, 1.0])).unwrap(), [0.0, -1.0, 1.0]);
        for values in [&[f32::NAN][..], &[f32::INFINITY], &[1.1]] {
            assert!(decode_audio(&encode(values)).is_err());
        }
        assert!(decode_audio("").is_err());
        assert!(decode_audio("AA==").is_err());
        assert!(decode_audio("!").is_err());
        assert!(decode_audio(&"A".repeat(MAX_BYTES.div_ceil(3) * 4 + 4)).is_err());
    }
}
