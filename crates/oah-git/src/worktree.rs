//! Worktrees: a checkout per branch, so several chats can work on one repo at once.

use serde::Serialize;

use crate::Repo;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Worktree {
    pub path: String,
    /// None when detached.
    pub branch: Option<String>,
    pub head: String,
    /// The repository's main checkout.
    pub main: bool,
    pub locked: bool,
    pub prunable: bool,
}

/// `git worktree list --porcelain -z`.
pub fn parse_worktrees(out: &str) -> Vec<Worktree> {
    let mut list = Vec::new();
    let mut cur: Option<Worktree> = None;
    for field in out.split('\0') {
        if field.is_empty() {
            if let Some(w) = cur.take() {
                list.push(w);
            }
            continue;
        }
        let (k, v) = field.split_once(' ').unwrap_or((field, ""));
        match k {
            "worktree" => {
                if let Some(w) = cur.take() {
                    list.push(w);
                }
                cur = Some(Worktree {
                    path: v.into(),
                    branch: None,
                    head: String::new(),
                    main: list.is_empty(),
                    locked: false,
                    prunable: false,
                });
            }
            "HEAD" => cur.iter_mut().for_each(|w| w.head = v.chars().take(12).collect()),
            "branch" => cur.iter_mut().for_each(|w| w.branch = Some(v.trim_start_matches("refs/heads/").into())),
            "locked" => cur.iter_mut().for_each(|w| w.locked = true),
            "prunable" => cur.iter_mut().for_each(|w| w.prunable = true),
            _ => {}
        }
    }
    if let Some(w) = cur {
        list.push(w);
    }
    list
}

pub fn worktrees(repo: &Repo) -> Result<Vec<Worktree>, String> {
    Ok(parse_worktrees(&repo.git(&["worktree", "list", "--porcelain", "-z"])?))
}

/// A branch name made safe for a folder: `feat/new thing` → `feat-new-thing`.
pub fn slug(branch: &str) -> String {
    let s: String = branch.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '-' }).collect();
    s.trim_matches('-').to_string()
}

/// Where a new worktree for `branch` goes: `dir` when set, else `<repo>-worktrees` beside the
/// repo. Uses `/` for SSH hosts and the OS separator here.
pub fn default_dir(repo: &Repo, dir: &str, branch: &str) -> String {
    let remote = !matches!(repo.location, oah_core::settings::Location::Local);
    let sep = if remote || !cfg!(windows) { '/' } else { '\\' };
    let base = repo.path.trim_end_matches(['/', '\\']);
    let parent = if dir.trim().is_empty() {
        let (up, name) = base.rsplit_once(['/', '\\']).unwrap_or(("", base));
        format!("{up}{sep}{name}-worktrees")
    } else {
        dir.trim().trim_end_matches(['/', '\\']).to_string()
    };
    format!("{parent}{sep}{}", slug(branch))
}

/// The branch new work starts from: `base` when set, else the remote's default branch, else
/// the current branch.
pub fn default_base(repo: &Repo, base: &str) -> String {
    if !base.trim().is_empty() {
        return base.trim().to_string();
    }
    if let Ok(r) = repo.git(&["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]) {
        if !r.trim().is_empty() {
            return r.trim().to_string();
        }
    }
    repo.git(&["rev-parse", "--abbrev-ref", "HEAD"]).map(|b| b.trim().to_string()).unwrap_or_else(|_| "HEAD".into())
}

/// Adds a worktree at `path` on a new `branch` from `base` (or on `branch` if it exists).
pub fn add(repo: &Repo, path: &str, branch: &str, base: &str) -> Result<Worktree, String> {
    if branch.trim().is_empty() || branch.starts_with('-') || base.starts_with('-') {
        return Err("not a branch name".into());
    }
    let exists = repo.git(&["rev-parse", "--verify", "-q", &format!("refs/heads/{branch}")]).is_ok();
    if exists {
        repo.git(&["worktree", "add", "--", path, branch])?;
    } else {
        repo.git(&["worktree", "add", "-b", branch, "--", path, base])?;
    }
    worktrees(repo)?
        .into_iter()
        .find(|w| w.branch.as_deref() == Some(branch))
        .ok_or_else(|| "git made the worktree but doesn't list it".into())
}

/// Removes a worktree (`force`: even with uncommitted changes), and its branch when asked.
pub fn remove(repo: &Repo, path: &str, force: bool, delete_branch: bool) -> Result<(), String> {
    let local = !repo.location.is_remote();
    let branch = worktrees(repo)?.into_iter().find(|w| same(&w.path, path) || (local && same_file(&w.path, path))).and_then(|w| w.branch);
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.extend(["--", path]);
    repo.git(&args)?;
    if delete_branch {
        if let Some(b) = branch {
            repo.git(&["branch", if force { "-D" } else { "-d" }, "--", &b])?;
        }
    }
    let _ = repo.git(&["worktree", "prune"]);
    Ok(())
}

/// Same folder on this computer, after resolving links and Windows short (8.3) names, which
/// `git worktree list` never prints.
fn same_file(a: &str, b: &str) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Same path, whichever slashes and (on Windows) case.
pub fn same(a: &str, b: &str) -> bool {
    let n = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_string();
    if cfg!(windows) {
        n(a).eq_ignore_ascii_case(&n(b))
    } else {
        n(a) == n(b)
    }
}
