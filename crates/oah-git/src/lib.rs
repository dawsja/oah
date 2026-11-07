//! Git for the side panel and worktrees, through the `git` and `gh` CLIs run where the project
//! is ([`oah_transport`]), so a project on an SSH host works the same as a local one.
//!
//! Every function takes a [`Repo`] (where and which folder) and returns plain data for the UI.

pub mod github;
mod graph;
mod parse;
pub mod worktree;

use std::time::Duration;

use oah_core::settings::{EnvVar, Location};
pub use oah_transport::Output;
use oah_transport::Spec;
use serde::Serialize;

pub use graph::{lanes, GraphRow};
pub use parse::*;

/// A folder on a machine.
#[derive(Debug, Clone)]
pub struct Repo {
    pub location: Location,
    pub path: String,
}

const QUICK: Duration = Duration::from_secs(20);
const NETWORK: Duration = Duration::from_secs(180);

impl Repo {
    pub fn new(location: Location, path: impl Into<String>) -> Repo {
        Repo { location, path: path.into() }
    }

    fn env() -> Vec<EnvVar> {
        [("GIT_OPTIONAL_LOCKS", "0"), ("GIT_TERMINAL_PROMPT", "0"), ("LC_ALL", "C"), ("GIT_PAGER", "cat")]
            .iter()
            .map(|(k, v)| EnvVar { key: (*k).into(), value: (*v).into(), in_keychain: false })
            .collect()
    }

    /// Runs `program args…` in the repo folder.
    pub fn exec(&self, program: &str, args: &[&str], input: Option<&[u8]>, limit: Duration) -> Result<Output, String> {
        let env = Repo::env();
        let spec =
            Spec { location: &self.location, cwd: &self.path, program, args: args.iter().map(|a| a.to_string()).collect(), env: &env };
        oah_transport::run(&spec, input, limit)
    }

    /// `git args…`, failing with git's own message.
    pub fn git(&self, args: &[&str]) -> Result<String, String> {
        self.git_with(args, None, QUICK)
    }

    fn git_with(&self, args: &[&str], input: Option<&[u8]>, limit: Duration) -> Result<String, String> {
        let out = self.exec("git", args, input, limit)?;
        if out.ok() {
            Ok(out.stdout)
        } else {
            Err(git_error(&out))
        }
    }
}

fn git_error(out: &Output) -> String {
    let text = if out.stderr.trim().is_empty() { &out.stdout } else { &out.stderr };
    let lines: Vec<&str> =
        text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("warning: in the working copy")).collect();
    let msg = lines.iter().map(|l| l.trim_start_matches("fatal: ").trim_start_matches("error: ")).collect::<Vec<_>>().join("\n");
    if msg.is_empty() {
        format!("git exited with {:?}", out.code)
    } else {
        msg
    }
}

// ---- Status ----------------------------------------------------------------------------------

/// The working tree: branch, sync state and changed files with line counts.
pub fn status(repo: &Repo) -> Result<Status, String> {
    let out = repo.exec("git", &["status", "--porcelain=v2", "--branch", "-z", "--untracked-files=all"], None, QUICK)?;
    if !out.ok() {
        if out.stderr.contains("not a git repository") {
            return Ok(Status { is_repo: false, ..Default::default() });
        }
        return Err(git_error(&out));
    }
    let mut st = parse_status(&out.stdout);
    let unstaged = repo.git(&["diff", "--numstat", "-z"]).unwrap_or_default();
    let staged = repo.git(&["diff", "--cached", "--numstat", "-z"]).unwrap_or_default();
    apply_numstat(&mut st.unstaged, &parse_numstat(&unstaged));
    apply_numstat(&mut st.staged, &parse_numstat(&staged));
    Ok(st)
}

/// One file's diff: staged (index vs HEAD) or not (worktree vs index); untracked files diff
/// against nothing.
pub fn diff(repo: &Repo, path: &str, staged: bool, untracked: bool) -> Result<String, String> {
    if untracked {
        // `--no-index` exits 1 when the files differ, which they always do here.
        let out = repo.exec("git", &["diff", "--no-color", "--no-index", "--", "/dev/null", path], None, QUICK)?;
        return if out.code == Some(1) || out.ok() { Ok(out.stdout) } else { Err(git_error(&out)) };
    }
    let mut args = vec!["diff", "--no-color", "--no-ext-diff"];
    if staged {
        args.push("--cached");
    }
    args.extend(["--", path]);
    repo.git(&args)
}

pub fn stage(repo: &Repo, paths: &[String]) -> Result<(), String> {
    with_paths(repo, &["add", "--all", "--"], paths)
}

pub fn unstage(repo: &Repo, paths: &[String]) -> Result<(), String> {
    // `restore --staged` needs a HEAD; before the first commit, `rm --cached` does the same.
    if repo.git(&["rev-parse", "--verify", "-q", "HEAD"]).is_ok() {
        with_paths(repo, &["restore", "--staged", "--"], paths)
    } else {
        with_paths(repo, &["rm", "--cached", "-r", "-q", "--"], paths)
    }
}

/// Throws away changes: tracked files go back to the index; untracked ones are deleted.
pub fn discard(repo: &Repo, tracked: &[String], untracked: &[String]) -> Result<(), String> {
    if !tracked.is_empty() {
        with_paths(repo, &["restore", "--worktree", "--"], tracked)?;
    }
    if !untracked.is_empty() {
        with_paths(repo, &["clean", "-f", "-q", "--"], untracked)?;
    }
    Ok(())
}

fn with_paths(repo: &Repo, base: &[&str], paths: &[String]) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut args: Vec<&str> = base.to_vec();
    args.extend(paths.iter().map(String::as_str));
    repo.git(&args).map(|_| ())
}

/// Commits what's staged (`all`: stages everything first). Returns the new commit's short hash.
pub fn commit(repo: &Repo, message: &str, amend: bool, all: bool) -> Result<String, String> {
    if message.trim().is_empty() && !amend {
        return Err("Write a commit message first.".into());
    }
    if all {
        repo.git(&["add", "--all"])?;
    }
    let mut args = vec!["commit", "-q", "-F", "-"];
    if amend {
        args.push("--amend");
        if message.trim().is_empty() {
            args = vec!["commit", "-q", "--amend", "--no-edit"];
        }
    }
    repo.git_with(&args, Some(message.as_bytes()), QUICK)?;
    Ok(repo.git(&["rev-parse", "--short", "HEAD"])?.trim().to_string())
}

// ---- Branches and sync -----------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Branch {
    pub name: String,
    pub remote: bool,
    pub current: bool,
    pub upstream: Option<String>,
    /// Unix seconds of the last commit.
    pub updated: i64,
}

pub fn branches(repo: &Repo) -> Result<Vec<Branch>, String> {
    let out = repo.git(&[
        "for-each-ref",
        "--sort=-committerdate",
        "--format=%(refname)%1f%(HEAD)%1f%(upstream:short)%1f%(committerdate:unix)",
        "refs/heads",
        "refs/remotes",
    ])?;
    Ok(parse_branches(&out))
}

pub fn switch(repo: &Repo, branch: &str, create: bool) -> Result<(), String> {
    if branch.starts_with('-') {
        return Err("not a branch name".into());
    }
    let args: Vec<&str> = if create { vec!["switch", "-c", branch] } else { vec!["switch", branch] };
    repo.git(&args).map(|_| ())
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncOp {
    Fetch,
    Pull,
    Push,
    /// Pull then push.
    Sync,
}

pub fn sync(repo: &Repo, op: SyncOp) -> Result<String, String> {
    let has_upstream = repo.git(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).is_ok();
    let push = |repo: &Repo| {
        if has_upstream {
            repo.git_with(&["push"], None, NETWORK)
        } else {
            repo.git_with(&["push", "-u", "origin", "HEAD"], None, NETWORK)
        }
    };
    match op {
        SyncOp::Fetch => repo.git_with(&["fetch", "--prune"], None, NETWORK),
        SyncOp::Pull => repo.git_with(&["pull", "--ff-only"], None, NETWORK),
        SyncOp::Push => push(repo),
        SyncOp::Sync => {
            if has_upstream {
                repo.git_with(&["pull", "--ff-only"], None, NETWORK)?;
            }
            push(repo)
        }
    }
}

// ---- History ---------------------------------------------------------------------------------

/// The last `limit` commits reachable from HEAD (and its upstream), newest first, with lanes.
pub fn log(repo: &Repo, limit: usize) -> Result<Vec<GraphRow>, String> {
    let n = format!("-n{limit}");
    let mut args = vec!["log", "--topo-order", "--format=%H%x1f%P%x1f%an%x1f%at%x1f%s%x1f%D%x1e", n.as_str(), "HEAD"];
    if repo.git(&["rev-parse", "--verify", "-q", "@{u}"]).is_ok() {
        args.push("@{u}");
    }
    match repo.git(&args) {
        Ok(out) => Ok(lanes(parse_log(&out))),
        Err(e) if e.contains("does not have any commits") || e.contains("bad default revision") || e.contains("unknown revision") => {
            Ok(vec![])
        }
        Err(e) => Err(e),
    }
}

/// The files a commit changed, with line counts.
pub fn commit_files(repo: &Repo, hash: &str) -> Result<Vec<Change>, String> {
    if !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("not a commit hash".into());
    }
    let out = repo.git(&["show", "--no-color", "--format=", "--numstat", "-z", "--find-renames", hash])?;
    Ok(parse_numstat(&out)
        .into_iter()
        .map(|(path, (added, removed, orig))| Change { path, orig_path: orig, kind: ChangeKind::Modified, added, removed })
        .collect())
}

pub fn commit_diff(repo: &Repo, hash: &str, path: &str) -> Result<String, String> {
    if !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("not a commit hash".into());
    }
    repo.git(&["show", "--no-color", "--format=", hash, "--", path])
}

// ---- Files -----------------------------------------------------------------------------------

/// Every file in the repo the user would see: tracked plus untracked-but-not-ignored.
pub fn files(repo: &Repo, limit: usize) -> Result<Vec<String>, String> {
    let out = repo.git(&["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--deduplicate"])?;
    let mut files: Vec<String> = out.split('\0').filter(|f| !f.is_empty()).map(String::from).take(limit).collect();
    files.sort_by_key(|f| f.to_lowercase());
    files.dedup();
    Ok(files)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GrepHit {
    pub path: String,
    pub line: u32,
    pub text: String,
}

/// Lines matching `pattern` (fixed string, case-insensitive unless it has capitals).
pub fn grep(repo: &Repo, pattern: &str, limit: usize) -> Result<Vec<GrepHit>, String> {
    if pattern.is_empty() {
        return Ok(vec![]);
    }
    let mut args = vec!["grep", "-n", "-I", "--no-color", "--untracked", "-F", "--full-name"];
    if !pattern.chars().any(char::is_uppercase) {
        args.push("-i");
    }
    args.extend(["-e", pattern]);
    let out = repo.exec("git", &args, None, QUICK)?;
    // 1 means no matches.
    if out.code == Some(1) {
        return Ok(vec![]);
    }
    if !out.ok() {
        return Err(git_error(&out));
    }
    Ok(parse_grep(&out.stdout, limit))
}

/// A text file's contents (up to `max` bytes), read with `git`-free `cat`-like access.
pub fn read_file(repo: &Repo, path: &str, max: usize) -> Result<String, String> {
    if path.split(['/', '\\']).any(|p| p == "..") {
        return Err("path leaves the project".into());
    }
    match &repo.location {
        Location::Local => {
            let full = std::path::Path::new(&repo.path).join(path);
            let bytes = std::fs::read(&full).map_err(|e| e.to_string())?;
            if bytes.iter().take(8000).any(|b| *b == 0) {
                return Err("binary file".into());
            }
            let n = bytes.len().min(max);
            Ok(String::from_utf8_lossy(&bytes[..n]).into_owned())
        }
        Location::Ssh { .. } | Location::Wsl { .. } => {
            let out = repo.exec("head", &["-c", &max.to_string(), "--", path], None, QUICK)?;
            if out.ok() {
                Ok(out.stdout)
            } else {
                Err(git_error(&out))
            }
        }
    }
}

#[cfg(test)]
mod tests;
