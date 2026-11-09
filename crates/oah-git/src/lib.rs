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

/// Reading: status, diffs, logs. Killing one of these loses nothing.
const QUICK: Duration = Duration::from_secs(20);
const NETWORK: Duration = Duration::from_secs(180);
/// Changing the repo: a checkout, a commit's hooks, a worktree. Killed half-way, these leave
/// `index.lock` or a half-made worktree behind, so they get as long as they need, within reason.
const WRITE: Duration = Duration::from_secs(15 * 60);
/// Paths per git command line, by length: Windows caps a command line at 32,767 characters
/// and Linux one ssh argument at 128 KiB.
const PATHS_PER_CALL: usize = 8_000;

impl Repo {
    pub fn new(location: Location, path: impl Into<String>) -> Repo {
        Repo { location, path: path.into() }
    }

    fn env() -> Vec<EnvVar> {
        // Paths are file names, never patterns: discarding a file named `*` must not discard
        // every file.
        [("GIT_OPTIONAL_LOCKS", "0"), ("GIT_TERMINAL_PROMPT", "0"), ("LC_ALL", "C"), ("GIT_PAGER", "cat"), ("GIT_LITERAL_PATHSPECS", "1")]
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

    /// `git args…` that changes the repo (see [`WRITE`]).
    pub fn git_write(&self, args: &[&str]) -> Result<String, String> {
        self.git_with(args, None, WRITE)
    }

    /// Where the project folder is inside its repo (`frontend/`), empty at the top. Git prints
    /// some paths from the top; oah shows and passes them from the project folder.
    fn prefix(&self) -> String {
        static KNOWN: std::sync::Mutex<Vec<(Location, String, String)>> = std::sync::Mutex::new(Vec::new());
        if let Some((_, _, p)) = KNOWN.lock().unwrap().iter().find(|(l, d, _)| *l == self.location && *d == self.path) {
            return p.clone();
        }
        let Ok(p) = self.git(&["rev-parse", "--show-prefix"]).map(|p| p.trim().to_string()) else { return String::new() };
        KNOWN.lock().unwrap().push((self.location.clone(), self.path.clone(), p.clone()));
        p
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
    // Only the project folder's changes (it can be a folder inside a bigger repo).
    let out = repo.exec("git", &["status", "--porcelain=v2", "--branch", "-z", "--untracked-files=all", "--", "."], None, QUICK)?;
    if !out.ok() {
        if out.stderr.contains("not a git repository") {
            return Ok(Status { is_repo: false, ..Default::default() });
        }
        return Err(git_error(&out));
    }
    let mut st = parse_status(&out.stdout);
    // Status paths are from the top of the repo; everything else here is from the folder.
    let prefix = repo.prefix();
    if !prefix.is_empty() {
        let strip = |p: &mut String| {
            if let Some(rest) = p.strip_prefix(&prefix) {
                *p = rest.to_string();
            }
        };
        for c in st.staged.iter_mut().chain(&mut st.unstaged).chain(&mut st.untracked).chain(&mut st.conflicted) {
            strip(&mut c.path);
            if let Some(o) = c.orig_path.as_mut() {
                strip(o);
            }
        }
    }
    let unstaged = repo.git(&["diff", "--numstat", "-z", "--relative"]).unwrap_or_default();
    let staged = repo.git(&["diff", "--cached", "--numstat", "-z", "--relative"]).unwrap_or_default();
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

/// `git base… paths…`, in as many calls as the command line needs (see [`PATHS_PER_CALL`]).
fn with_paths(repo: &Repo, base: &[&str], paths: &[String]) -> Result<(), String> {
    let mut rest = paths;
    while !rest.is_empty() {
        let mut len = 0;
        let n = rest.iter().take_while(|p| {
            len += p.len() + 3;
            len <= PATHS_PER_CALL
        });
        let n = n.count().max(1);
        let mut args: Vec<&str> = base.to_vec();
        args.extend(rest[..n].iter().map(String::as_str));
        repo.git_write(&args)?;
        rest = &rest[n..];
    }
    Ok(())
}

/// Commits what's staged (`all`: stages everything first). Returns the new commit's short hash.
pub fn commit(repo: &Repo, message: &str, amend: bool, all: bool) -> Result<String, String> {
    if message.trim().is_empty() && !amend {
        return Err("Write a commit message first.".into());
    }
    if all {
        // The project folder's changes, not the whole repo's when it's a folder inside one.
        repo.git_write(&["add", "--all", "--", "."])?;
    }
    let mut args = vec!["commit", "-q", "-F", "-"];
    if amend {
        args.push("--amend");
        if message.trim().is_empty() {
            args = vec!["commit", "-q", "--amend", "--no-edit"];
        }
    }
    repo.git_with(&args, Some(message.as_bytes()), WRITE)?;
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
    repo.git_write(&args).map(|_| ())
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

/// The branch's upstream (`origin/feat`), if it has one.
pub(crate) fn upstream(repo: &Repo) -> Option<String> {
    repo.git(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).ok().map(|u| u.trim().to_string()).filter(|u| !u.is_empty())
}

/// Whether a plain `git push` goes where the user means: the upstream is the branch of the same
/// name. A branch made from `origin/main` can track it, and git then refuses a plain push.
fn pushes_to_own_name(repo: &Repo, upstream: &str) -> bool {
    let local = repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]).map(|b| b.trim().to_string()).unwrap_or_default();
    upstream.split_once('/').is_some_and(|(_, name)| name == local)
}

pub fn sync(repo: &Repo, op: SyncOp) -> Result<String, String> {
    let upstream = upstream(repo);
    let has_upstream = upstream.is_some();
    let push = |repo: &Repo| match &upstream {
        Some(u) if pushes_to_own_name(repo, u) => repo.git_with(&["push"], None, NETWORK),
        // No upstream, or one under another name: publish this branch under its own.
        _ => repo.git_with(&["push", "-u", "origin", "HEAD"], None, NETWORK),
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
    let out = match repo.git(&["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--deduplicate"]) {
        Ok(out) => out,
        // `--deduplicate` is git 2.31+; the list is deduplicated below anyway.
        Err(e) if e.contains("unknown option") || e.contains("usage: git ls-files") => {
            repo.git(&["ls-files", "-z", "--cached", "--others", "--exclude-standard"])?
        }
        Err(e) => return Err(e),
    };
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
    // Paths from the project folder, which is where they're opened from.
    // `-z`: names as they are (not quoted, so `café.txt` and `a:b.txt` open), split by NULs.
    let mut args = vec!["grep", "-n", "-z", "-I", "--no-color", "--untracked", "-F"];
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
            use std::io::Read;
            let full = std::path::Path::new(&repo.path).join(path);
            // Only as much as is shown: a multi-gigabyte log isn't read whole to show 512 KB.
            let mut bytes = Vec::new();
            std::fs::File::open(&full).and_then(|f| f.take(max as u64).read_to_end(&mut bytes)).map_err(|e| e.to_string())?;
            if bytes.iter().take(8000).any(|b| *b == 0) {
                return Err("binary file".into());
            }
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        }
        Location::Ssh { .. } | Location::Wsl { .. } => {
            let out = repo.exec("head", &["-c", &max.to_string(), "--", path], None, QUICK)?;
            if !out.ok() {
                return Err(git_error(&out));
            }
            if out.stdout.chars().take(8000).any(|c| c == '\0') {
                return Err("binary file".into());
            }
            Ok(out.stdout)
        }
    }
}

/// What a commit would hold, for writing its message: the staged diff (or every change when
/// nothing is staged), new files' contents, and recent subjects for the repo's style. Cut to
/// about `max` characters.
pub fn commit_context(repo: &Repo, max: usize) -> Result<String, String> {
    let st = status(repo)?;
    let staged = !st.staged.is_empty();
    let mut out = String::new();
    let subjects = repo.git(&["log", "-8", "--format=%s"]).unwrap_or_default();
    if !subjects.trim().is_empty() {
        out.push_str("Recent commit subjects (match their style):\n");
        out.push_str(subjects.trim_end());
        out.push_str("\n\n");
    }
    let range: Vec<&str> = if staged {
        vec!["diff", "--cached"]
    } else if st.head.is_some() {
        vec!["diff", "HEAD"]
    } else {
        vec!["diff"]
    };
    let stat = repo.git(&[range.clone(), vec!["--stat"]].concat()).unwrap_or_default();
    // One line of context is enough to say what changed, and fewer tokens than git's three.
    let diff = repo.git(&[range, vec!["--no-color", "--no-ext-diff", "-U1"]].concat())?;
    out.push_str(if staged { "The staged changes:\n" } else { "The changes:\n" });
    out.push_str(stat.trim_end());
    out.push_str("\n\n");
    out.push_str(&trim_diff_for_message(&diff, PER_FILE));
    if !staged {
        let new: Vec<&str> = st.untracked.iter().map(|c| c.path.as_str()).collect();
        for path in new.iter().take(12) {
            if out.len() > max {
                break;
            }
            match read_file(repo, path, 8 * 1024) {
                Ok(text) => out.push_str(&format!("\nNew file {path}:\n{text}\n")),
                Err(_) => out.push_str(&format!("\nNew file {path} (binary)\n")),
            }
        }
        if new.len() > 12 {
            out.push_str(&format!("\n…and {} more new files\n", new.len() - 12));
        }
    }
    if out.len() > max {
        let mut cut = max;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out.push_str("\n…(cut)\n");
    }
    Ok(out)
}

/// Most of one file's diff that goes into a commit-message prompt.
const PER_FILE: usize = 6 * 1024;

/// Lockfiles and minified or generated files: their diffs say nothing a commit message needs
/// (the stat above already lists them) and can push the real changes past the prompt's cap.
fn generated(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.ends_with(".lock")
        || matches!(
            name,
            "package-lock.json"
                | "pnpm-lock.yaml"
                | "yarn.lock"
                | "bun.lockb"
                | "go.sum"
                | "composer.lock"
                | "Gemfile.lock"
                | "poetry.lock"
        )
        || name.ends_with(".min.js")
        || name.ends_with(".min.css")
        || name.ends_with(".map")
        || name.ends_with(".snap")
}

/// A diff for a commit-message prompt: generated files as one line each, every other file's
/// part cut to `per_file` bytes.
pub fn trim_diff_for_message(diff: &str, per_file: usize) -> String {
    let mut out = String::with_capacity(diff.len().min(64 * 1024));
    let mut parts = diff.split("\ndiff --git ").peekable();
    let mut first = true;
    while let Some(part) = parts.next() {
        let part = if first { part.strip_prefix("diff --git ").unwrap_or(part) } else { part };
        first = false;
        if part.trim().is_empty() {
            continue;
        }
        // `a/path b/path` on the header line.
        let header = part.lines().next().unwrap_or("");
        let path = header.rsplit(" b/").next().unwrap_or(header);
        if generated(path) {
            out.push_str(&format!("diff --git {header}\n(generated file changed; not shown)\n"));
            continue;
        }
        out.push_str("diff --git ");
        if part.len() > per_file {
            let mut cut = per_file;
            while !part.is_char_boundary(cut) {
                cut -= 1;
            }
            out.push_str(&part[..cut]);
            out.push_str("\n…(rest of this file's changes cut)\n");
        } else {
            out.push_str(part);
            if parts.peek().is_some() {
                out.push('\n');
            }
        }
    }
    out
}

/// An image file's type, from its extension (`None`: not one oah shows).
pub fn image_mime(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        _ => return None,
    })
}

/// An image file in the project as a `data:` URL (up to `max` bytes), read here or on the
/// project's machine (`base64` there).
pub fn read_image(repo: &Repo, path: &str, max: usize) -> Result<String, String> {
    use base64::Engine;
    if path.split(['/', '\\']).any(|p| p == "..") {
        return Err("path leaves the project".into());
    }
    let mime = image_mime(path).ok_or("not an image")?;
    let data = match &repo.location {
        Location::Local => {
            let full = std::path::Path::new(&repo.path).join(path);
            let len = std::fs::metadata(&full).map_err(|e| e.to_string())?.len() as usize;
            if len > max {
                return Err(format!("too big to show ({} MB)", len / 1_000_000));
            }
            base64::engine::general_purpose::STANDARD.encode(std::fs::read(&full).map_err(|e| e.to_string())?)
        }
        Location::Ssh { .. } | Location::Wsl { .. } => {
            let out = repo.exec(
                "sh",
                &[
                    "-c",
                    r#"s=$(wc -c < "$1") || exit 1; [ "$s" -le "$2" ] || { echo "too big to show" >&2; exit 3; }; base64 < "$1""#,
                    "oah",
                    path,
                    &max.to_string(),
                ],
                None,
                QUICK,
            )?;
            if !out.ok() {
                return Err(git_error(&out));
            }
            out.stdout.split_whitespace().collect()
        }
    };
    Ok(format!("data:{mime};base64,{data}"))
}

#[cfg(test)]
mod tests;
