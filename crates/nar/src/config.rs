//! Paths, persisted settings and shared enums.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const APP_NAME: &str = "nar";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `~/.nar` (or `$NAR_HOME`).
pub fn home_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("NAR_HOME") {
        return PathBuf::from(p);
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".nar")
}

pub fn ensure_dir(p: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(p)
}

pub fn sessions_dir() -> PathBuf {
    home_dir().join("sessions")
}

pub fn history_file() -> PathBuf {
    home_dir().join("history.jsonl")
}

pub fn settings_file() -> PathBuf {
    home_dir().join("settings.json")
}

pub fn auth_file() -> PathBuf {
    home_dir().join("auth.json")
}

pub fn host_id_file() -> PathBuf {
    home_dir().join("host_id")
}

/// Write a file atomically with owner-only permissions on Unix.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Minimal,
    Low,
    #[default]
    Medium,
    High,
    Xhigh,
}

impl Effort {
    pub const ALL: [Effort; 5] = [Effort::Minimal, Effort::Low, Effort::Medium, Effort::High, Effort::Xhigh];
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Minimal => "minimal",
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::Xhigh => "xhigh",
        }
    }
    pub fn describe(self) -> &'static str {
        match self {
            Effort::Minimal => "fastest, almost no reasoning",
            Effort::Low => "quick answers, light reasoning",
            Effort::Medium => "balanced speed and depth (default)",
            Effort::High => "deeper reasoning for hard problems",
            Effort::Xhigh => "maximum reasoning, slowest",
        }
    }
    pub fn parse(s: &str) -> Option<Effort> {
        Effort::ALL.into_iter().find(|e| e.as_str().eq_ignore_ascii_case(s.trim()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    /// Reads and safe commands run; edits and other commands ask.
    #[default]
    Auto,
    /// Every tool call asks first.
    AlwaysAsk,
    /// Nothing asks.
    AlwaysAllow,
}

impl Permission {
    pub const ALL: [Permission; 3] = [Permission::Auto, Permission::AlwaysAsk, Permission::AlwaysAllow];
    pub fn as_str(self) -> &'static str {
        match self {
            Permission::Auto => "auto",
            Permission::AlwaysAsk => "always ask",
            Permission::AlwaysAllow => "always allow",
        }
    }
    pub fn describe(self) -> &'static str {
        match self {
            Permission::Auto => "reads and safe commands run; edits, writes and other commands ask",
            Permission::AlwaysAsk => "ask before every tool call",
            Permission::AlwaysAllow => "never ask (use with care)",
        }
    }
    pub fn parse(s: &str) -> Option<Permission> {
        let s = s.trim().to_ascii_lowercase().replace(['-', '_'], " ");
        match s.as_str() {
            "auto" => Some(Permission::Auto),
            "ask" | "always ask" => Some(Permission::AlwaysAsk),
            "allow" | "always allow" | "yolo" => Some(Permission::AlwaysAllow),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Plan,
    #[default]
    Build,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Plan => "plan",
            Mode::Build => "build",
        }
    }
    pub fn toggle(self) -> Mode {
        match self {
            Mode::Plan => Mode::Build,
            Mode::Build => Mode::Plan,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub model: String,
    pub effort: Effort,
    pub permission: Permission,
    pub mode: Mode,
    /// Override the context window used for usage display and auto-compaction.
    pub context_window: Option<u64>,
    /// Fraction of the context window that triggers auto-compaction.
    pub auto_compact_at: f64,
    pub auto_compact: bool,
    /// Base URL of the Responses API.
    pub api_base: String,
    /// Commands always allowed in auto mode (prefix match), in addition to built-ins.
    pub allowed_commands: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            model: "gpt-5.5".into(),
            effort: Effort::Medium,
            permission: Permission::Auto,
            mode: Mode::Build,
            context_window: None,
            auto_compact_at: 0.85,
            auto_compact: true,
            api_base: "https://api.openai.com/v1".into(),
            allowed_commands: Vec::new(),
        }
    }
}

impl Settings {
    pub fn load() -> Settings {
        std::fs::read(settings_file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) {
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            let _ = write_atomic(&settings_file(), &bytes);
        }
    }
}

/// Context window size for a model slug. Unknown models fall back to a
/// conservative default; users can override with `context_window`.
pub fn context_window_for(model: &str, overridden: Option<u64>) -> u64 {
    if let Some(v) = overridden {
        return v;
    }
    let m = model.to_ascii_lowercase();
    if m.contains("codex-mini") {
        200_000
    } else if m.starts_with("gpt-5") || m.starts_with("gpt-6") || m.contains("codex") {
        272_000
    } else if m.starts_with("gpt-4.1") {
        1_047_576
    } else if m.starts_with("o3") || m.starts_with("o4") {
        200_000
    } else {
        128_000
    }
}

pub fn short_path(p: &Path) -> String {
    let s = p.display().to_string();
    if let Some(home) = dirs::home_dir() {
        let h = home.display().to_string();
        if let Some(rest) = s.strip_prefix(&h) {
            return format!("~{rest}");
        }
    }
    s
}
