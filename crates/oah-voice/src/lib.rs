//! Speech to text on this computer: NVIDIA's Parakeet Unified English 0.6B, run through
//! transcribe.cpp. Turning voice on downloads the model to `<oah app dir>/speech`; turning it
//! off deletes it. The composer records 16 kHz mono and sends the samples here.
//!
//! Ported from Gond Code's `speech.rs`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};

/// The model: Q8_0 of `handy-computer/parakeet-unified-en-0.6b-gguf`, pinned to a revision and
/// checked by its hash.
pub const FILE: &str = "parakeet-unified-en-0.6b-Q8_0.gguf";
const URL: &str = "https://huggingface.co/handy-computer/parakeet-unified-en-0.6b-gguf/resolve/7e948f21b7bdbac698d3318db9d350f1096f3b6c/parakeet-unified-en-0.6b-Q8_0.gguf";
pub const BYTES: u64 = 731_357_568;
const SHA256: &str = "4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795";

/// Where voice stands, for the Settings page and the mic button.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum VoiceState {
    Off,
    Downloading { percent: u64 },
    Ready,
    Error { error: String },
    /// This processor can't run the engine.
    Unsupported { error: String },
}

/// Why this computer can't run speech to text, if it can't: the engine is built for an x86
/// floor of AVX2, FMA, F16C and BMI2 (`.cargo/config.toml`); without them it would crash.
pub fn unsupported() -> Option<String> {
    #[cfg(target_arch = "x86_64")]
    {
        let missing: Vec<&str> = [
            ("AVX2", std::arch::is_x86_feature_detected!("avx2")),
            ("FMA", std::arch::is_x86_feature_detected!("fma")),
            ("F16C", std::arch::is_x86_feature_detected!("f16c")),
            ("BMI2", std::arch::is_x86_feature_detected!("bmi2")),
        ]
        .into_iter()
        .filter(|(_, has)| !has)
        .map(|(name, _)| name)
        .collect();
        if !missing.is_empty() {
            return Some(format!("This computer's processor can't run speech to text: it doesn't have {} (processors from about 2013 on do).", missing.join(", ")));
        }
    }
    None
}

pub fn dir() -> PathBuf {
    oah_core::paths::app_dir().join("speech")
}

/// The model file, if it's downloaded.
pub fn model_path() -> Option<PathBuf> {
    Some(dir().join(FILE)).filter(|p| p.is_file())
}

/// Downloads the model to `dir`, reporting `(done, total)` bytes. A partial file is kept aside
/// until its hash checks out; `cancel` stops it and deletes the part.
pub fn download(dir: &Path, cancel: &AtomicBool, mut progress: impl FnMut(u64, u64)) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let part = dir.join(format!("{FILE}.part"));
    let fail = |e: String| format!("couldn't download the speech model: {e}");
    let agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(20))).build().new_agent();
    let mut resp = agent.get(URL).call().map_err(|e| fail(e.to_string()))?;
    let total = resp.body().content_length().unwrap_or(BYTES);
    let mut body = resp.body_mut().as_reader();
    let mut out = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let (mut done, mut buf) = (0u64, vec![0u8; 1 << 16]);
    loop {
        if cancel.load(Ordering::SeqCst) {
            drop(out);
            let _ = std::fs::remove_file(&part);
            return Err("the download was stopped".into());
        }
        let n = body.read(&mut buf).map_err(|e| fail(e.to_string()))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        hash.update(&buf[..n]);
        done += n as u64;
        progress(done, total);
    }
    out.flush().map_err(|e| e.to_string())?;
    drop(out);
    let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != SHA256 {
        let _ = std::fs::remove_file(&part);
        return Err(fail("the file didn't match its checksum".into()));
    }
    let path = dir.join(FILE);
    std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// The loaded model and a session on it, kept between recordings.
struct Loaded {
    _model: transcribe_cpp::Model,
    session: transcribe_cpp::Session,
}

static LOADED: Mutex<Option<Loaded>> = Mutex::new(None);

/// Text from 16 kHz mono samples in [-1, 1]. Loads the model on first use.
pub fn transcribe(pcm: &[f32]) -> Result<String, String> {
    if let Some(why) = unsupported() {
        return Err(why);
    }
    let path = model_path().ok_or("Voice is off: turn it on in Settings → Voice.")?;
    let mut g = LOADED.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        let model = transcribe_cpp::Model::load(&path).map_err(|e| format!("couldn't load the speech model: {e}"))?;
        let session = model.session().map_err(|e| format!("couldn't start speech to text: {e}"))?;
        *g = Some(Loaded { _model: model, session });
    }
    let loaded = g.as_mut().expect("loaded above");
    let out = loaded.session.run(pcm, &transcribe_cpp::RunOptions::default()).map_err(|e| format!("couldn't transcribe: {e}"))?;
    Ok(out.text.trim().to_string())
}

/// Turns voice off: the model leaves memory and the disk.
pub fn remove() -> Result<(), String> {
    *LOADED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let d = dir();
    if d.exists() {
        std::fs::remove_dir_all(d).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Samples from the composer: little-endian f32 bytes.
pub fn samples(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if bytes.len() % 4 != 0 {
        return Err("the recording isn't f32 samples".into());
    }
    Ok(bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_samples() {
        let b: Vec<u8> = [0.5f32, -1.0].iter().flat_map(|x| x.to_le_bytes()).collect();
        assert_eq!(samples(&b).unwrap(), [0.5, -1.0]);
        assert!(samples(&[0, 1, 2]).is_err());
    }

    #[test]
    fn states_serialize_with_a_tag() {
        let s = serde_json::to_string(&VoiceState::Downloading { percent: 40 }).unwrap();
        assert_eq!(s, r#"{"state":"downloading","percent":40}"#);
    }

    /// On a real PC (ignored; downloads 731 MB): the model from download to text.
    #[test]
    #[ignore]
    fn real_speech_round_trip() {
        if model_path().is_none() {
            let stop = AtomicBool::new(false);
            download(&dir(), &stop, |_, _| {}).unwrap();
        }
        let text = transcribe(&vec![0.0; 16_000]).unwrap();
        eprintln!("heard: {text:?}");
    }
}
