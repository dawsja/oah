//! Is an agent CLI installed, and which version: `<binary> --version`, with a time limit.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use oah_core::settings::AgentKind;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub found: bool,
    /// The resolved executable.
    pub path: Option<String>,
    /// The first version-looking token of `--version`, e.g. `2.1.319`.
    pub version: Option<String>,
    pub error: Option<String>,
}

/// Resolves `binary` (empty: the agent's usual command on PATH).
pub fn resolve(kind: AgentKind, binary: &str) -> Option<PathBuf> {
    let b = binary.trim();
    let b = if b.is_empty() { kind.default_binary() } else { b };
    which::which(b).ok()
}

pub fn probe(kind: AgentKind, binary: &str) -> Probe {
    let Some(path) = resolve(kind, binary) else {
        let shown = if binary.trim().is_empty() { kind.default_binary() } else { binary.trim() };
        return Probe { found: false, path: None, version: None, error: Some(format!("{shown} isn't installed or isn't on PATH")) };
    };
    let shown = path.to_string_lossy().into_owned();
    match run_version(&path, Duration::from_secs(8)) {
        Ok(out) => Probe { found: true, path: Some(shown), version: parse_version(&out), error: None },
        Err(e) => Probe { found: true, path: Some(shown), version: None, error: Some(e) },
    }
}

fn run_version(path: &PathBuf, limit: Duration) -> Result<String, String> {
    let mut cmd = Command::new(path);
    cmd.arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    no_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("couldn't run it: {e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > limit => {
                let _ = child.kill();
                return Err("`--version` didn't answer".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(30)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let mut out = String::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_string(&mut out);
    }
    Ok(out)
}

/// No console window flashing up on Windows.
pub fn no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let _ = cmd;
}

/// `2.1.319 (Claude Code)` → `2.1.319`; `codex-cli 0.166.0` → `0.166.0`.
pub fn parse_version(out: &str) -> Option<String> {
    out.split_whitespace()
        .map(|w| w.trim_start_matches('v'))
        .find(|w| w.split('.').count() >= 2 && w.split('.').all(|p| !p.is_empty() && p.chars().next().is_some_and(|c| c.is_ascii_digit())))
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_versions() {
        assert_eq!(parse_version("2.1.319 (Claude Code)\n").as_deref(), Some("2.1.319"));
        assert_eq!(parse_version("codex-cli 0.166.0").as_deref(), Some("0.166.0"));
        assert_eq!(parse_version("v1.2.3-beta").as_deref(), Some("1.2.3-beta"));
        assert_eq!(parse_version("no version"), None);
    }

    #[test]
    fn missing_binary_is_not_found() {
        let p = probe(AgentKind::Claude, "definitely-not-a-real-binary-oah");
        assert!(!p.found);
        assert!(p.error.unwrap().contains("isn't installed"));
    }
}
