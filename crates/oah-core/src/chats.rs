//! Chats on disk: `<app dir>/chats/<id>/meta.json` (what the sidebar lists) and `events.jsonl`
//! (everything the chat showed, one event per line, replayed when it's reopened).

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::settings::AgentKind;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChatMeta {
    pub id: String,
    pub project_id: String,
    pub agent: AgentKind,
    /// Empty: the agent's default.
    pub model: String,
    pub title: String,
    /// The folder the agent works in: the project, or one of its worktrees.
    pub cwd: String,
    /// The CLI's own session id, to resume the conversation.
    pub agent_session_id: Option<String>,
    /// Unix milliseconds.
    pub created_at: u64,
    pub updated_at: u64,
    pub tokens: u64,
    pub cost_usd: f64,
}

impl Default for ChatMeta {
    fn default() -> Self {
        ChatMeta {
            id: String::new(),
            project_id: String::new(),
            agent: AgentKind::Claude,
            model: String::new(),
            title: String::new(),
            cwd: String::new(),
            agent_session_id: None,
            created_at: 0,
            updated_at: 0,
            tokens: 0,
            cost_usd: 0.0,
        }
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// A chat id: hex, safe in a path.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// The chats folder; tests point it elsewhere.
#[derive(Debug, Clone)]
pub struct ChatStore {
    root: PathBuf,
}

impl ChatStore {
    pub fn new(root: impl Into<PathBuf>) -> ChatStore {
        ChatStore { root: root.into() }
    }

    fn dir(&self, id: &str) -> Result<PathBuf> {
        if !valid_id(id) {
            return Err(Error::Invalid(format!("not a chat id: {id:?}")));
        }
        Ok(self.root.join(id))
    }

    pub fn create(&self, project_id: &str, agent: AgentKind, model: &str, cwd: &str) -> Result<ChatMeta> {
        let now = now_ms();
        let meta = ChatMeta {
            id: uuid::Uuid::new_v4().simple().to_string()[..16].to_string(),
            project_id: project_id.into(),
            agent,
            model: model.into(),
            cwd: cwd.into(),
            created_at: now,
            updated_at: now,
            ..Default::default()
        };
        self.save(&meta)?;
        Ok(meta)
    }

    pub fn save(&self, meta: &ChatMeta) -> Result<()> {
        let dir = self.dir(&meta.id)?;
        std::fs::create_dir_all(&dir)?;
        let tmp = dir.join("meta.json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(meta)?)?;
        std::fs::rename(&tmp, dir.join("meta.json"))?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<ChatMeta> {
        let text = std::fs::read_to_string(self.dir(id)?.join("meta.json"))?;
        Ok(serde_json::from_str(&text)?)
    }

    /// Every chat, newest activity first. Unreadable ones are skipped.
    pub fn list(&self) -> Vec<ChatMeta> {
        let Ok(entries) = std::fs::read_dir(&self.root) else { return vec![] };
        let mut out: Vec<ChatMeta> = entries
            .flatten()
            .filter_map(|e| std::fs::read_to_string(e.path().join("meta.json")).ok())
            .filter_map(|t| serde_json::from_str(&t).ok())
            .collect();
        out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        out
    }

    /// Appends one event line.
    pub fn append(&self, id: &str, event: &impl Serialize) -> Result<()> {
        let dir = self.dir(id)?;
        std::fs::create_dir_all(&dir)?;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("events.jsonl"))?;
        let mut line = serde_json::to_vec(event)?;
        line.push(b'\n');
        f.write_all(&line)?;
        Ok(())
    }

    /// The chat's events as JSON values (a torn last line is dropped).
    pub fn events(&self, id: &str) -> Result<Vec<serde_json::Value>> {
        let path = self.dir(id)?.join("events.jsonl");
        let f = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e.into()),
        };
        Ok(std::io::BufReader::new(f).lines().map_while(|l| l.ok()).filter_map(|l| serde_json::from_str(&l).ok()).collect())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let dir = self.dir(id)?;
        match std::fs::remove_dir_all(&dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// A title from the first prompt: its first line, cut at a word near 60 characters.
pub fn title_from(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("New chat");
    if line.chars().count() <= 60 {
        return line.to_string();
    }
    let cut: String = line.chars().take(60).collect();
    let cut = cut.rsplit_once(' ').map(|(a, _)| a).filter(|a| a.len() > 30).unwrap_or(&cut);
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_lists_and_replays() {
        let dir = tempfile::tempdir().unwrap();
        let store = ChatStore::new(dir.path());
        let mut a = store.create("p1", AgentKind::Claude, "", "/code").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        let b = store.create("p1", AgentKind::Codex, "gpt-5.5", "/code").unwrap();
        assert_eq!(store.list().iter().map(|c| c.id.clone()).collect::<Vec<_>>(), [b.id.clone(), a.id.clone()]);

        a.updated_at = now_ms() + 10;
        a.title = "Fix the tests".into();
        store.save(&a).unwrap();
        assert_eq!(store.list()[0].title, "Fix the tests");

        store.append(&a.id, &serde_json::json!({"type": "textDelta", "text": "hi"})).unwrap();
        store.append(&a.id, &serde_json::json!({"type": "turnFinished"})).unwrap();
        let mut f = std::fs::OpenOptions::new().append(true).open(dir.path().join(&a.id).join("events.jsonl")).unwrap();
        f.write_all(b"{\"type\":\"tor").unwrap();
        assert_eq!(store.events(&a.id).unwrap().len(), 2, "torn last line is dropped");
        assert!(store.events(&b.id).unwrap().is_empty());

        store.delete(&a.id).unwrap();
        assert_eq!(store.list().len(), 1);
        assert!(store.get("../etc").is_err());
    }

    #[test]
    fn titles() {
        assert_eq!(title_from("\n  fix the build \nmore"), "fix the build");
        let long = "please look at every place where we read the settings file and make it robust";
        let t = title_from(long);
        assert!(t.ends_with('…') && t.chars().count() <= 61, "{t}");
    }
}
