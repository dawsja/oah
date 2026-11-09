//! Full-text search over every chat's log: what was asked, what the agent answered, what it ran
//! and which files it touched (not tool output, which is mostly noise). Each chat's text is
//! pulled out of `events.jsonl` once and kept until the file grows.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use oah_agents::event::ChatEvent;
use oah_core::chats::ChatStore;
use serde::Serialize;

/// Where in a chat a match is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Part {
    User,
    Agent,
    Tool,
}

/// A chat that matched: the best line around the match, and how many places matched.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub chat_id: String,
    pub part: Part,
    pub snippet: String,
    pub matches: usize,
}

/// One searchable piece of a chat, as written and lowercased.
#[derive(Debug)]
struct Piece {
    part: Part,
    text: String,
    lower: String,
}

/// A chat's pieces, and the size of its `events.jsonl` when they were read.
type Cached = (u64, Arc<Vec<Piece>>);

#[derive(Default)]
pub struct Index {
    chats: Mutex<HashMap<String, Cached>>,
}

impl Index {
    pub fn new() -> Index {
        Index::default()
    }

    /// Chats in `store` whose text has every word of `query` (any case), newest activity first,
    /// at most `limit`. A blank query matches nothing.
    pub fn search(&self, store: &ChatStore, query: &str, limit: usize) -> Vec<Hit> {
        let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if terms.is_empty() {
            return vec![];
        }
        let metas = store.list();
        // Deleted chats leave the cache.
        self.chats.lock().unwrap().retain(|id, _| metas.iter().any(|m| &m.id == id));
        let mut hits = vec![];
        for meta in metas {
            let pieces = self.pieces(store, &meta.id);
            if let Some(hit) = find(&meta.id, &pieces, &terms) {
                hits.push(hit);
                if hits.len() >= limit {
                    break;
                }
            }
        }
        hits
    }

    fn pieces(&self, store: &ChatStore, id: &str) -> Arc<Vec<Piece>> {
        let size = std::fs::metadata(store.root().join(id).join("events.jsonl")).map(|m| m.len()).unwrap_or(0);
        if let Some((seen, pieces)) = self.chats.lock().unwrap().get(id) {
            if *seen == size {
                return pieces.clone();
            }
        }
        let pieces = Arc::new(extract(&store.events(id).unwrap_or_default()));
        self.chats.lock().unwrap().insert(id.to_string(), (size, pieces.clone()));
        pieces
    }
}

/// The searchable text of a chat's logged events.
fn extract(events: &[serde_json::Value]) -> Vec<Piece> {
    let mut out = vec![];
    let mut add = |part: Part, text: &str| {
        let text = text.trim();
        if !text.is_empty() {
            out.push(Piece { part, text: text.to_string(), lower: text.to_lowercase() });
        }
    };
    for v in events {
        let Ok(e) = serde_json::from_value::<ChatEvent>(v.clone()) else { continue };
        match &e {
            ChatEvent::UserMessage { text, .. } => add(Part::User, text),
            ChatEvent::Text { text, .. } => add(Part::Agent, text),
            ChatEvent::ToolStarted { tool, .. } => {
                add(Part::Tool, &tool.title);
                if let Some(path) = tool.path.as_deref().filter(|p| !tool.title.contains(*p)) {
                    add(Part::Tool, path);
                }
            }
            ChatEvent::Approval { request } => {
                add(Part::Tool, &request.title);
                if let Some(d) = &request.detail {
                    add(Part::Agent, d);
                }
            }
            ChatEvent::Question { request } => {
                for q in &request.questions {
                    add(Part::Agent, &q.question);
                }
            }
            _ => {}
        }
    }
    out
}

/// The chat's hit when every term is somewhere in it: the snippet is from the piece holding the
/// most terms (the first such one), what the user asked winning ties.
fn find(chat_id: &str, pieces: &[Piece], terms: &[String]) -> Option<Hit> {
    if !terms.iter().all(|t| pieces.iter().any(|p| p.lower.contains(t.as_str()))) {
        return None;
    }
    let matches = pieces.iter().map(|p| terms.iter().map(|t| p.lower.matches(t.as_str()).count()).sum::<usize>()).sum();
    let key = |p: &Piece| (terms.iter().filter(|t| p.lower.contains(t.as_str())).count(), p.part == Part::User);
    let mut best = &pieces[0];
    for p in pieces {
        if key(p) > key(best) {
            best = p;
        }
    }
    let term = terms.iter().find(|t| best.lower.contains(t.as_str())).unwrap();
    Some(Hit { chat_id: chat_id.to_string(), part: best.part, snippet: snippet(best, term), matches })
}

/// A line around the first `term` in the piece: some words before it, more after, on one line.
fn snippet(p: &Piece, term: &str) -> String {
    const BEFORE: usize = 28;
    const AFTER: usize = 100;
    // Lowercasing can change how many chars a string has; then cut the lowercase copy.
    let source = if p.text.chars().count() == p.lower.chars().count() { &p.text } else { &p.lower };
    let at = p.lower.find(term).map(|b| p.lower[..b].chars().count()).unwrap_or(0);
    let chars: Vec<char> = source.chars().collect();
    let mut start = at.saturating_sub(BEFORE);
    // Start on a word.
    if start > 0 {
        if let Some(sp) = chars[start..at].iter().position(|c| c.is_whitespace()) {
            start += sp + 1;
        }
    }
    let end = (at + term.chars().count() + AFTER).min(chars.len());
    let body: String = chars[start..end].iter().collect();
    let body = body.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{}{body}{}", if start > 0 { "…" } else { "" }, if end < chars.len() { "…" } else { "" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oah_core::settings::AgentKind;
    use serde_json::json;

    fn store_with(chats: &[(&str, Vec<serde_json::Value>)]) -> (tempfile::TempDir, ChatStore, Vec<String>) {
        let dir = tempfile::tempdir().unwrap();
        let store = ChatStore::new(dir.path());
        let mut ids = vec![];
        for (i, (title, events)) in chats.iter().enumerate() {
            let mut meta = store.create("p", AgentKind::Claude, "", "/p").unwrap();
            meta.title = title.to_string();
            meta.updated_at = 1000 + i as u64;
            store.save(&meta).unwrap();
            for e in events {
                store.append(&meta.id, e).unwrap();
            }
            ids.push(meta.id);
        }
        (dir, store, ids)
    }

    fn user(text: &str) -> serde_json::Value {
        json!({"type":"userMessage","id":"u","text":text,"images":[],"files":[]})
    }
    fn agent(text: &str) -> serde_json::Value {
        json!({"type":"text","itemId":"a","text":text})
    }
    fn tool(title: &str, path: Option<&str>) -> serde_json::Value {
        json!({"type":"toolStarted","itemId":"t","tool":{"kind":"edit","name":"Edit","title":title,"input":{},"path":path}})
    }

    #[test]
    fn finds_chats_with_every_word() {
        let (_d, store, ids) = store_with(&[
            ("a", vec![user("make the retry logic back off"), agent("Done: exponential backoff.")]),
            ("b", vec![user("fix the login page"), tool("Edit src/retry.rs", Some("src/retry.rs"))]),
            ("c", vec![user("nothing here")]),
        ]);
        let index = Index::new();
        let hits = index.search(&store, "RETRY", 10);
        // Newest first: b, then a.
        assert_eq!(hits.iter().map(|h| h.chat_id.as_str()).collect::<Vec<_>>(), [ids[1].as_str(), ids[0].as_str()]);
        assert_eq!(hits[0].part, Part::Tool);
        assert_eq!(hits[1].part, Part::User);
        assert_eq!(hits[1].snippet, "make the retry logic back off");
        // Words can be in different messages; all must be there.
        assert_eq!(index.search(&store, "retry backoff", 10).len(), 1);
        assert!(index.search(&store, "retry zebra", 10).is_empty());
        assert!(index.search(&store, "   ", 10).is_empty());
        assert_eq!(index.search(&store, "the", 1).len(), 1);
    }

    #[test]
    fn sees_new_events_and_forgets_deleted_chats() {
        let (_d, store, ids) = store_with(&[("a", vec![user("hello")])]);
        let index = Index::new();
        assert!(index.search(&store, "flaky", 10).is_empty());
        store.append(&ids[0], &agent("the flaky test is fixed")).unwrap();
        assert_eq!(index.search(&store, "flaky", 10).len(), 1);
        store.delete(&ids[0]).unwrap();
        assert!(index.search(&store, "flaky", 10).is_empty());
        assert!(index.chats.lock().unwrap().is_empty());
    }

    #[test]
    fn snippets_are_one_line_around_the_match() {
        let long = format!("{} needle {}", "word ".repeat(30), "tail ".repeat(40));
        let p = Piece { part: Part::Agent, lower: long.to_lowercase(), text: long };
        let s = snippet(&p, "needle");
        assert!(s.starts_with("…word") && s.contains("needle") && s.ends_with('…'), "{s}");
        assert!(s.chars().count() < 160);
        let p = Piece { part: Part::User, text: "line one\nTHE Needle".into(), lower: "line one\nthe needle".into() };
        assert_eq!(snippet(&p, "needle"), "line one THE Needle");
    }
}
