//! Saved sessions (`~/.nar/sessions/<id>.json`) and the transcript model.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::Usage;
use crate::config::{self, Effort, Mode};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ToolStatus {
    Pending,
    Running,
    Ok,
    Failed,
    Denied,
}

/// One block of the visible transcript.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Entry {
    User { text: String },
    Assistant { text: String },
    Reasoning { text: String },
    Tool { call_id: String, title: String, summary: String, detail: Vec<String>, status: ToolStatus },
    Info { text: String },
    Error { text: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Goal {
    pub text: String,
    pub turns: u32,
    pub done: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionData {
    pub id: String,
    pub created: i64,
    pub updated: i64,
    pub cwd: String,
    pub title: String,
    pub model: String,
    pub effort: Effort,
    pub mode: Mode,
    /// Responses API conversation items (user messages, outputs, tool results).
    pub items: Vec<Value>,
    pub transcript: Vec<Entry>,
    pub usage: Usage,
    /// Tokens the next request is expected to carry (drives the context meter).
    pub context_tokens: u64,
    pub total_tokens: u64,
    pub goal: Option<Goal>,
    pub todos: Vec<(String, String)>,
    pub compactions: u32,
}

impl Default for SessionData {
    fn default() -> Self {
        let now = chrono::Utc::now().timestamp();
        SessionData {
            id: new_id(),
            created: now,
            updated: now,
            cwd: std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default(),
            title: String::new(),
            model: String::new(),
            effort: Effort::Medium,
            mode: Mode::Build,
            items: vec![],
            transcript: vec![],
            usage: Usage::default(),
            context_tokens: 0,
            total_tokens: 0,
            goal: None,
            todos: vec![],
            compactions: 0,
        }
    }
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

impl SessionData {
    pub fn path(&self) -> std::path::PathBuf {
        config::sessions_dir().join(format!("{}.json", self.id))
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn save(&mut self) {
        if self.is_empty() {
            return;
        }
        self.updated = chrono::Utc::now().timestamp();
        if let Ok(bytes) = serde_json::to_vec(self) {
            let _ = config::write_private(&self.path(), &bytes);
        }
    }

    pub fn load(id_or_prefix: &str) -> Option<SessionData> {
        let dir = config::sessions_dir();
        let exact = dir.join(format!("{id_or_prefix}.json"));
        let path = if exact.exists() {
            exact
        } else {
            let mut matches: Vec<_> = std::fs::read_dir(&dir)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.file_stem().map(|s| s.to_string_lossy().starts_with(id_or_prefix)).unwrap_or(false))
                .collect();
            if matches.len() != 1 {
                return None;
            }
            matches.remove(0)
        };
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    }

    /// Most recent session for `cwd`, if any.
    pub fn latest(cwd: &str) -> Option<SessionData> {
        list().into_iter().find(|s| s.cwd == cwd).and_then(|s| SessionData::load(&s.id))
    }
}

#[derive(Clone, Debug)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub updated: i64,
    pub messages: usize,
}

pub fn list() -> Vec<SessionSummary> {
    #[derive(Deserialize)]
    struct Head {
        id: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        cwd: String,
        #[serde(default)]
        updated: i64,
        #[serde(default)]
        transcript: Vec<serde::de::IgnoredAny>,
    }
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(config::sessions_dir()) {
        for e in rd.flatten() {
            if e.path().extension().map(|x| x != "json").unwrap_or(true) {
                continue;
            }
            if let Ok(bytes) = std::fs::read(e.path()) {
                if let Ok(h) = serde_json::from_slice::<Head>(&bytes) {
                    out.push(SessionSummary { id: h.id, title: h.title, cwd: h.cwd, updated: h.updated, messages: h.transcript.len() });
                }
            }
        }
    }
    out.sort_by(|a, b| b.updated.cmp(&a.updated));
    out
}

pub fn relative_time(ts: i64) -> String {
    let d = chrono::Utc::now().timestamp() - ts;
    if d < 60 {
        "just now".into()
    } else if d < 3600 {
        format!("{}m ago", d / 60)
    } else if d < 86_400 {
        format!("{}h ago", d / 3600)
    } else {
        format!("{}d ago", d / 86_400)
    }
}

/// Prompt history shared across sessions (`~/.nar/history.jsonl`).
pub fn load_history() -> Vec<String> {
    std::fs::read_to_string(config::history_file())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<String>(l).ok())
        .collect()
}

pub fn append_history(entry: &str) {
    use std::io::Write;
    let path = config::history_file();
    let _ = config::ensure_dir(path.parent().unwrap());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{}", serde_json::to_string(entry).unwrap_or_default());
    }
}
