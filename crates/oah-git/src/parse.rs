//! Parsers for git's machine-readable output.

use std::collections::HashMap;

use serde::Serialize;

use crate::Branch;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Conflicted,
    Untracked,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub path: String,
    /// The old path of a rename or copy.
    pub orig_path: Option<String>,
    pub kind: ChangeKind,
    pub added: u32,
    pub removed: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub is_repo: bool,
    /// None on a detached HEAD.
    pub branch: Option<String>,
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub staged: Vec<Change>,
    pub unstaged: Vec<Change>,
    pub untracked: Vec<Change>,
    pub conflicted: Vec<Change>,
}

fn kind(c: char) -> Option<ChangeKind> {
    Some(match c {
        'M' => ChangeKind::Modified,
        'A' => ChangeKind::Added,
        'D' => ChangeKind::Deleted,
        'R' => ChangeKind::Renamed,
        'C' => ChangeKind::Copied,
        'T' => ChangeKind::TypeChanged,
        _ => return None,
    })
}

/// `git status --porcelain=v2 --branch -z`.
pub fn parse_status(out: &str) -> Status {
    let mut st = Status { is_repo: true, ..Default::default() };
    let mut fields = out.split('\0').peekable();
    while let Some(rec) = fields.next() {
        if rec.is_empty() {
            continue;
        }
        if let Some(h) = rec.strip_prefix("# ") {
            let (k, v) = h.split_once(' ').unwrap_or((h, ""));
            match k {
                "branch.oid" if v != "(initial)" => st.head = Some(v.chars().take(12).collect()),
                "branch.head" if v != "(detached)" => st.branch = Some(v.to_string()),
                "branch.upstream" => st.upstream = Some(v.to_string()),
                "branch.ab" => {
                    for p in v.split(' ') {
                        if let Some(a) = p.strip_prefix('+') {
                            st.ahead = a.parse().unwrap_or(0);
                        } else if let Some(b) = p.strip_prefix('-') {
                            st.behind = b.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let tag = rec.chars().next().unwrap_or(' ');
        match tag {
            '1' | '2' => {
                // 1 XY sub mH mI mW hH hI path / 2 XY sub mH mI mW hH hI Xscore path\0orig
                let n = if tag == '1' { 8 } else { 9 };
                let parts: Vec<&str> = rec.splitn(n + 1, ' ').collect();
                let xy: Vec<char> = parts.get(1).unwrap_or(&"..").chars().collect();
                let path = parts.get(n).unwrap_or(&"").to_string();
                let orig = if tag == '2' { fields.next().map(String::from) } else { None };
                if let Some(k) = kind(xy[0]) {
                    st.staged.push(Change { path: path.clone(), orig_path: orig.clone(), kind: k, added: 0, removed: 0 });
                }
                if let Some(k) = kind(xy[1]) {
                    st.unstaged.push(Change { path, orig_path: orig, kind: k, added: 0, removed: 0 });
                }
            }
            'u' => {
                let path = rec.splitn(11, ' ').nth(10).unwrap_or("").to_string();
                st.conflicted.push(Change { path, orig_path: None, kind: ChangeKind::Conflicted, added: 0, removed: 0 });
            }
            '?' => {
                st.untracked.push(Change { path: rec[2..].to_string(), orig_path: None, kind: ChangeKind::Untracked, added: 0, removed: 0 })
            }
            _ => {}
        }
    }
    st
}

/// `git diff --numstat -z`: path → (added, removed, old path of a rename). Binary files count 0.
pub fn parse_numstat(out: &str) -> HashMap<String, (u32, u32, Option<String>)> {
    let mut map = HashMap::new();
    let mut fields = out.split('\0');
    while let Some(rec) = fields.next() {
        let rec = rec.trim_start_matches('\n');
        if rec.is_empty() {
            continue;
        }
        let mut p = rec.splitn(3, '\t');
        let (a, r, path) = (p.next().unwrap_or("0"), p.next().unwrap_or("0"), p.next().unwrap_or(""));
        let (a, r) = (a.parse().unwrap_or(0), r.parse().unwrap_or(0));
        if path.is_empty() {
            // A rename: the old and new paths follow as their own fields.
            let old = fields.next().unwrap_or("").to_string();
            let new = fields.next().unwrap_or("").to_string();
            map.insert(new, (a, r, Some(old)));
        } else {
            map.insert(path.to_string(), (a, r, None));
        }
    }
    map
}

pub fn apply_numstat(changes: &mut [Change], counts: &HashMap<String, (u32, u32, Option<String>)>) {
    for c in changes {
        if let Some((a, r, _)) = counts.get(&c.path) {
            c.added = *a;
            c.removed = *r;
        }
    }
}

pub fn parse_branches(out: &str) -> Vec<Branch> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.split('\x1f');
            let refname = f.next()?;
            let head = f.next().unwrap_or("");
            let upstream = f.next().filter(|u| !u.is_empty()).map(String::from);
            let updated = f.next().and_then(|t| t.trim().parse().ok()).unwrap_or(0);
            let (name, remote) =
                if let Some(n) = refname.strip_prefix("refs/heads/") { (n, false) } else { (refname.strip_prefix("refs/remotes/")?, true) };
            if remote && name.ends_with("/HEAD") {
                return None;
            }
            Some(Branch { name: name.into(), remote, current: head == "*", upstream, updated })
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    /// Unix seconds.
    pub time: i64,
    pub subject: String,
    /// Branch and tag names pointing here (`HEAD -> main`, `origin/main`, `tag: v1`).
    pub refs: Vec<String>,
}

/// `git log --format=%H%x1f%P%x1f%an%x1f%at%x1f%s%x1f%D%x1e`.
pub fn parse_log(out: &str) -> Vec<Commit> {
    out.split('\x1e')
        .filter_map(|rec| {
            let rec = rec.trim_start_matches(['\n', '\r']);
            if rec.is_empty() {
                return None;
            }
            let f: Vec<&str> = rec.split('\x1f').collect();
            if f.len() < 6 {
                return None;
            }
            Some(Commit {
                hash: f[0].into(),
                parents: f[1].split_whitespace().map(String::from).collect(),
                author: f[2].into(),
                time: f[3].trim().parse().unwrap_or(0),
                subject: f[4].into(),
                refs: f[5].trim().split(", ").filter(|r| !r.is_empty()).map(String::from).collect(),
            })
        })
        .collect()
}

/// `git grep -n` lines: `path:line:text`.
pub fn parse_grep(out: &str, limit: usize) -> Vec<crate::GrepHit> {
    out.lines()
        .filter_map(|l| {
            let mut p = l.splitn(3, ':');
            let path = p.next()?;
            let line = p.next()?.parse().ok()?;
            let text = p.next().unwrap_or("");
            let text: String = text.trim().chars().take(240).collect();
            Some(crate::GrepHit { path: path.into(), line, text })
        })
        .take(limit)
        .collect()
}
