//! Chats on disk: `<app dir>/chats/<id>/meta.json` (what the sidebar lists) and `events.jsonl`
//! (everything the chat showed, one event per line, replayed when it's reopened).

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::settings::{permission_or_none, AgentKind, PermissionMode};
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChatMeta {
    pub id: String,
    pub project_id: String,
    pub agent: AgentKind,
    /// Empty: the agent's default.
    pub model: String,
    /// Reasoning effort (`low` … `max`); empty: the model's default.
    #[serde(default)]
    pub effort: String,
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
    /// Listed first in its project, above the unpinned chats.
    pub pinned: bool,
    pub sleeping: bool,
    /// The permission mode the chat last ran with (from the window, a phone or MCP); none: the
    /// agent's default in Settings → Providers.
    #[serde(deserialize_with = "permission_or_none", skip_serializing_if = "Option::is_none")]
    pub permission: Option<PermissionMode>,
}

impl Default for ChatMeta {
    fn default() -> Self {
        ChatMeta {
            id: String::new(),
            project_id: String::new(),
            agent: AgentKind::Claude,
            model: String::new(),
            effort: String::new(),
            title: String::new(),
            cwd: String::new(),
            agent_session_id: None,
            created_at: 0,
            updated_at: 0,
            tokens: 0,
            cost_usd: 0.0,
            pinned: false,
            sleeping: false,
            permission: None,
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
        crate::write_atomic(&dir.join("meta.json"), serde_json::to_string_pretty(meta)?.as_bytes())
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
        out.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
        out
    }

    /// Appends one event line, to a chat that exists (a deleted chat's late events are
    /// refused rather than bringing its folder back).
    pub fn append(&self, id: &str, event: &impl Serialize) -> Result<()> {
        let dir = self.dir(id)?;
        if !dir.join("meta.json").is_file() {
            return Err(Error::Invalid(format!("no chat {id}")));
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("events.jsonl"))?;
        let mut line = serde_json::to_vec(event)?;
        line.push(b'\n');
        f.write_all(&line)?;
        Ok(())
    }

    /// Removes chat folders with no `meta.json` left by older versions: a deleted chat's
    /// agent wrote its last events after the folder was removed. Only folders holding nothing
    /// but an `events.jsonl` of such end-of-run events go; returns how many.
    pub fn prune_orphans(&self) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.root) else { return 0 };
        let mut n = 0;
        for dir in entries.flatten().map(|e| e.path()).filter(|p| p.is_dir() && !p.join("meta.json").exists()) {
            let only_log = std::fs::read_dir(&dir).map(|d| d.flatten().all(|e| e.file_name() == "events.jsonl")).unwrap_or(false);
            let ending = std::fs::read_to_string(dir.join("events.jsonl")).is_ok_and(|t| {
                t.lines().filter(|l| !l.trim().is_empty()).all(|l| {
                    serde_json::from_str::<serde_json::Value>(l)
                        .is_ok_and(|v| matches!(v["type"].as_str(), Some("exited" | "turnFinished" | "tasks" | "notice")))
                })
            });
            if only_log && ending && std::fs::remove_dir_all(&dir).is_ok() {
                n += 1;
            }
        }
        n
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

    /// Where a chat's attached images are kept (`<chat>/images/`), created on first use.
    pub fn images_dir(&self, id: &str) -> Result<PathBuf> {
        let dir = self.dir(id)?.join("images");
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// A saved image's path, if `file` is one of ours (a plain name, no folders).
    pub fn image_path(&self, id: &str, file: &str) -> Result<PathBuf> {
        let ok = !file.is_empty()
            && file.len() <= 80
            && file.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
            && !file.starts_with('.');
        if !ok {
            return Err(Error::Invalid(format!("not an image name: {file:?}")));
        }
        Ok(self.dir(id)?.join("images").join(file))
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
        // A deleted chat's late event doesn't bring its folder back.
        assert!(store.append(&a.id, &serde_json::json!({"type": "exited"})).is_err());
        assert!(!dir.path().join(&a.id).exists());
    }

    #[test]
    fn prunes_only_folders_older_versions_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let store = ChatStore::new(dir.path());
        let kept = store.create("p1", AgentKind::Claude, "", "/code").unwrap();
        let orphan = |name: &str, log: &str| {
            std::fs::create_dir_all(dir.path().join(name)).unwrap();
            std::fs::write(dir.path().join(name).join("events.jsonl"), log).unwrap();
        };
        orphan("deadbeef00000001", "{\"type\":\"exited\",\"code\":null}\n");
        orphan("deadbeef00000002", "{\"type\":\"turnFinished\"}\n{\"type\":\"exited\"}\n");
        // Anything with real content stays, meta or not.
        orphan("deadbeef00000003", "{\"type\":\"userMessage\",\"text\":\"hi\"}\n");
        assert_eq!(store.prune_orphans(), 2);
        assert!(dir.path().join("deadbeef00000003").exists());
        assert!(dir.path().join(&kept.id).exists());
    }

    #[test]
    fn titles() {
        assert_eq!(title_from("\n  fix the build \nmore"), "fix the build");
        let long = "please look at every place where we read the settings file and make it robust";
        let t = title_from(long);
        assert!(t.ends_with('…') && t.chars().count() <= 61, "{t}");
    }
}
