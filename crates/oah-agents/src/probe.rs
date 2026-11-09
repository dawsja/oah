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
    /// Set when the version is older than oah needs: what to do about it.
    pub outdated: Option<String>,
}

/// The oldest versions oah works with, one per major line (OpenCode's ACP before these lacks
/// what the chat needs).
fn minimums(kind: AgentKind) -> &'static [&'static str] {
    match kind {
        AgentKind::Opencode => &["1.18.31", "2.0.14"],
        AgentKind::Opencode2 => &["2.0.14"],
        _ => &[],
    }
}

/// The minimum for `version`'s line: the last one whose major is at or below it, else the first.
pub fn minimum(kind: AgentKind, version: &str) -> Option<&'static str> {
    let major = |v: &str| v.trim_start_matches('v').split('.').next().and_then(|m| m.parse::<u64>().ok()).unwrap_or(0);
    let mins = minimums(kind);
    mins.iter().rev().find(|m| major(m) <= major(version)).or(mins.first()).copied()
}

/// `Some(message)` when `version` is older than `kind`'s minimum.
pub fn outdated(kind: AgentKind, version: &str) -> Option<String> {
    let min = minimum(kind, version)?;
    older(version, min).then(|| match kind {
        AgentKind::Opencode | AgentKind::Opencode2 => {
            format!("OpenCode {version} is too old for chats. Update to {min} or newer (`opencode upgrade`).")
        }
        _ => format!("Version {version} is too old for chats. Update to {min} or newer."),
    })
}

/// `1.18.9` < `1.18.31`; a pre-release suffix (`-beta`) is ignored.
pub fn older(a: &str, b: &str) -> bool {
    let nums = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v').split(['-', '+']).next().unwrap_or("").split('.').map(|p| p.parse().unwrap_or(0)).collect()
    };
    let (a, b) = (nums(a), nums(b));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x < y;
        }
    }
    false
}

/// Resolves `binary` (empty: the agent's usual command on PATH).
pub fn resolve(kind: AgentKind, binary: &str) -> Option<PathBuf> {
    let b = binary.trim();
    let b = if b.is_empty() { kind.default_binary() } else { b };
    oah_transport::find_local(b)
}

pub fn probe(kind: AgentKind, binary: &str) -> Probe {
    let Some(path) = resolve(kind, binary) else {
        let shown = if binary.trim().is_empty() { kind.default_binary() } else { binary.trim() };
        return Probe {
            found: false,
            path: None,
            version: None,
            error: Some(format!("{shown} isn't installed or isn't on PATH")),
            outdated: None,
        };
    };
    let shown = path.to_string_lossy().into_owned();
    match run_version(&path, Duration::from_secs(8)) {
        Ok(out) => {
            let version = parse_version(&out);
            let outdated = version.as_deref().and_then(|v| outdated(kind, v));
            Probe { found: true, path: Some(shown), version, error: None, outdated }
        }
        Err(e) => Probe { found: true, path: Some(shown), version: None, error: Some(e), outdated: None },
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
    fn compares_versions() {
        assert!(older("1.18.9", "1.18.31"));
        assert!(older("0.9.0", "1.18.31"));
        assert!(!older("1.18.31", "1.18.31"));
        assert!(!older("1.19.5", "1.18.31"));
        assert!(!older("2.0", "1.18.31"));
        assert!(older("1.18.31-beta", "1.18.32"));
        assert!(outdated(AgentKind::Opencode, "1.17.0").unwrap().contains("opencode upgrade"));
        assert!(outdated(AgentKind::Opencode, "2.0.3").unwrap().contains("2.0.14"));
        assert_eq!(outdated(AgentKind::Opencode, "2.1.0"), None);
        assert!(outdated(AgentKind::Opencode, "0.15.0").unwrap().contains("1.18.31"));
        assert_eq!(outdated(AgentKind::Opencode, "1.19.5"), None);
        assert!(outdated(AgentKind::Opencode2, "1.19.5").unwrap().contains("2.0.14"), "opencode2 is 2.x only");
        assert_eq!(outdated(AgentKind::Opencode2, "2.2.6"), None);
        assert_eq!(outdated(AgentKind::Claude, "0.1.0"), None);
    }

    #[test]
    fn missing_binary_is_not_found() {
        let p = probe(AgentKind::Claude, "definitely-not-a-real-binary-oah");
        assert!(!p.found);
        assert!(p.error.unwrap().contains("isn't installed"));
    }
}
