//! The sidebar's view of saved sessions (`<oah home>/sessions/<id>.json`, written
//! by the CLI and by the desktop's engine alike). Only the header fields are
//! kept; files are re-read only when their size or mtime changes.
//!
//! Like the CLI's `listSessions`, it reads the engine's small summary files
//! (`sessions/meta/<id>.json`) when they match the session file, so a long
//! history lists without parsing every session. The newest sessions are still
//! read in full for the details the sidebar shows (model, mode, goal).

use serde::{de::IgnoredAny, Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub cwd_short: String,
    pub created: f64,
    pub updated: f64,
    pub model: String,
    pub mode: String,
    /// Transcript entries, for "(empty)" detection and a size hint.
    pub entries: usize,
    pub has_goal: bool,
    /// False when only the summary was read: `model`/`mode`/`entries`/`has_goal` are unknown.
    pub detailed: bool,
}

/// Sessions read in full (newest first) for the sidebar's details.
const DETAILED: usize = 60;

#[derive(Deserialize)]
struct Summary {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    updated: f64,
    /// The session file's mtime (ms) and size when the summary was written.
    mtime: f64,
    size: u64,
}

#[derive(Deserialize)]
struct Raw {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    created: f64,
    #[serde(default)]
    updated: f64,
    #[serde(default)]
    model: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    transcript: Vec<IgnoredAny>,
    #[serde(default)]
    goal: Option<Goal>,
}

#[derive(Deserialize)]
struct Goal {
    #[serde(default)]
    done: bool,
}

fn parse(path: &Path) -> Option<SessionMeta> {
    let text = fs::read_to_string(path).ok()?;
    let raw: Raw = serde_json::from_str(&text).ok()?;
    Some(SessionMeta {
        cwd_short: crate::paths::short_path(&raw.cwd),
        id: raw.id,
        title: raw.title,
        cwd: raw.cwd,
        created: raw.created,
        updated: raw.updated,
        model: raw.model,
        mode: raw.mode.unwrap_or_else(|| "build".into()),
        entries: raw.transcript.len(),
        has_goal: raw.goal.map(|g| !g.done).unwrap_or(false),
        detailed: true,
    })
}

fn mtime_ms(t: Option<SystemTime>) -> Option<f64> {
    Some(t?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64() * 1000.0)
}

/// The engine's summary for `path`, if it still describes that exact file.
fn summary(path: &Path, stamp: &Stamp) -> Option<SessionMeta> {
    let id = path.file_stem()?.to_str()?;
    let meta = path.parent()?.join("meta").join(format!("{id}.json"));
    let s: Summary = serde_json::from_str(&fs::read_to_string(meta).ok()?).ok()?;
    let fresh = s.size == stamp.0 && mtime_ms(stamp.1).is_some_and(|m| (m - s.mtime).abs() < 1.0);
    // Only saved sessions (with messages) get a summary.
    fresh.then(|| SessionMeta {
        cwd_short: crate::paths::short_path(&s.cwd),
        id: s.id,
        title: s.title,
        cwd: s.cwd,
        created: 0.0,
        updated: s.updated,
        model: String::new(),
        mode: String::new(),
        entries: 1,
        has_goal: false,
        detailed: false,
    })
}

type Stamp = (u64, Option<SystemTime>);

/// Caches parsed headers by file so refreshing the sidebar stays cheap.
#[derive(Default)]
pub struct SessionIndex {
    cache: Mutex<HashMap<PathBuf, (Stamp, Option<SessionMeta>)>>,
}

impl SessionIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every saved session in `dir`, newest first. Unreadable files are skipped.
    pub fn list(&self, dir: &Path) -> Vec<SessionMeta> {
        let Ok(rd) = fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let mut seen = Vec::new();
        let mut out: Vec<(PathBuf, Stamp, SessionMeta)> = Vec::new();
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(md) = entry.metadata() else { continue };
            if !md.is_file() {
                continue;
            }
            let stamp = (md.len(), md.modified().ok());
            let meta = match cache.get(&path) {
                Some((s, m)) if *s == stamp => m.clone(),
                _ => {
                    let m = summary(&path, &stamp).or_else(|| parse(&path));
                    cache.insert(path.clone(), (stamp, m.clone()));
                    m
                }
            };
            seen.push(path.clone());
            if let Some(m) = meta {
                out.push((path, stamp, m));
            }
        }
        cache.retain(|p, _| seen.contains(p));
        out.sort_by(|a, b| b.2.updated.total_cmp(&a.2.updated));
        // Fill in the details of the newest sessions (cached until they change).
        for (path, stamp, m) in out.iter_mut().take(DETAILED) {
            if m.detailed {
                continue;
            }
            if let Some(full) = parse(path) {
                cache.insert(path.clone(), (*stamp, Some(full.clone())));
                *m = full;
            }
        }
        out.into_iter().map(|(_, _, m)| m).collect()
    }
}

/// Delete a saved session (and its summary). `id` must look like a session id.
pub fn delete(dir: &Path, id: &str) -> std::io::Result<()> {
    if !crate::remote::valid_session_id(id) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("not a session id: {id:?}")));
    }
    for f in [dir.join(format!("{id}.json")), dir.join("meta").join(format!("{id}.json"))] {
        match fs::remove_file(&f) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// Sessions on another machine, from the summaries `remote::SESSIONS` prints
/// (home folder first, then one JSON summary per session), newest first.
pub fn from_remote(out: &str) -> Vec<SessionMeta> {
    let mut lines = out.lines().filter(|l| !l.starts_with("__OAH_PATH__"));
    let home = lines.next().unwrap_or_default().trim().to_string();
    let rest: String = lines.collect::<Vec<_>>().join("\n");
    let mut list: Vec<SessionMeta> = serde_json::Deserializer::from_str(&rest)
        .into_iter::<Summary>()
        .map_while(Result::ok)
        .map(|s| SessionMeta {
            cwd_short: crate::remote::short_with_home(&s.cwd, &home),
            id: s.id,
            title: s.title,
            cwd: s.cwd,
            created: 0.0,
            updated: s.updated,
            model: String::new(),
            mode: String::new(),
            entries: 1,
            has_goal: false,
            detailed: false,
        })
        .collect();
    list.sort_by(|a, b| b.updated.total_cmp(&a.updated));
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, id: &str, updated: f64, extra: &str) {
        fs::write(
            dir.join(format!("{id}.json")),
            format!(
                r#"{{"id":"{id}","created":1,"updated":{updated},"cwd":"/w/{id}","title":"t {id}","model":"anthropic/x","effort":"medium","mode":"plan","messages":[{{"role":"user","content":"hi"}}],"transcript":[{{"kind":"user","text":"hi"}},{{"kind":"info","text":"x"}}],"contextTokens":0,"totalTokens":0,"todos":[],"compactions":0{extra}}}"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn deletes_sessions_and_reads_remote_summaries() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "aaa", 10.0, "");
        fs::create_dir_all(dir.path().join("meta")).unwrap();
        fs::write(dir.path().join("meta/aaa.json"), "{}").unwrap();
        assert!(delete(dir.path(), "../aaa").is_err());
        delete(dir.path(), "aaa").unwrap();
        assert!(!dir.path().join("aaa.json").exists() && !dir.path().join("meta/aaa.json").exists());
        delete(dir.path(), "aaa").unwrap(); // already gone: fine
        let out = "/home/me\n{\"id\":\"a\",\"title\":\"A\",\"cwd\":\"/home/me/x\",\"updated\":1,\"mtime\":1,\"size\":2}\n{\"id\":\"b\",\"cwd\":\"/srv\",\"updated\":5,\"mtime\":1,\"size\":2}\n";
        let l = from_remote(out);
        assert_eq!(l.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(l[1].cwd_short, "~/x");
        assert!(from_remote("/home/me\n").is_empty());
    }

    #[test]
    fn lists_newest_first_and_refreshes_on_change() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "aaa", 10.0, "");
        write(dir.path(), "bbb", 20.0, r#","goal":{"text":"g","turns":1,"done":false}"#);
        fs::write(dir.path().join("broken.json"), "{").unwrap();
        fs::write(dir.path().join("notes.txt"), "x").unwrap();

        let idx = SessionIndex::new();
        let list = idx.list(dir.path());
        assert_eq!(list.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["bbb", "aaa"]);
        assert_eq!(list[0].entries, 2);
        assert_eq!(list[0].mode, "plan");
        assert!(list[0].has_goal);
        assert!(!list[1].has_goal);

        // A changed file is re-read; a removed one disappears.
        write(dir.path(), "aaa", 30.0, r#","extra":"padding to change the size""#);
        fs::remove_file(dir.path().join("bbb.json")).unwrap();
        let list = idx.list(dir.path());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].updated, 30.0);
    }

    #[test]
    fn uses_fresh_summaries_and_ignores_stale_ones() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("meta")).unwrap();
        let summary_for = |id: &str, title: &str, updated: f64| {
            let md = fs::metadata(dir.path().join(format!("{id}.json"))).unwrap();
            let mtime = mtime_ms(md.modified().ok()).unwrap();
            fs::write(
                dir.path().join("meta").join(format!("{id}.json")),
                format!(r#"{{"id":"{id}","title":"{title}","cwd":"/w","updated":{updated},"mtime":{mtime},"size":{}}}"#, md.len()),
            )
            .unwrap();
        };
        // More sessions than are read in full: the oldest come from their summaries.
        for i in 0..DETAILED + 2 {
            write(dir.path(), &format!("s{i:03}"), 1000.0 + i as f64, "");
            summary_for(&format!("s{i:03}"), &format!("summary {i}"), 1000.0 + i as f64);
        }
        let list = SessionIndex::new().list(dir.path());
        assert_eq!(list.len(), DETAILED + 2, "the meta folder itself is not a session");
        assert!(list[0].detailed && list[0].title == format!("t s{:03}", DETAILED + 1), "newest read in full");
        let oldest = list.last().unwrap();
        assert!(!oldest.detailed);
        assert_eq!(oldest.title, "summary 0");

        // A summary that no longer matches its file is ignored.
        write(dir.path(), "s000", 5000.0, r#","x":"changed size""#);
        let list = SessionIndex::new().list(dir.path());
        assert_eq!(list[0].id, "s000");
        assert_eq!(list[0].title, "t s000");
    }

    #[test]
    fn missing_dir_is_empty() {
        assert!(SessionIndex::new().list(Path::new("/definitely/not/here")).is_empty());
    }
}
