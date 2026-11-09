//! Git commands for the side panel. Each names the project and, optionally, the folder inside
//! it to use (a worktree); git runs where the project is.

use oah_core::settings::{AgentKind, Location};
use oah_git::{self as git, Branch, Change, GraphRow, GrepHit, Repo, Status, SyncOp};
use tauri::{AppHandle, Manager};

use crate::{err, AppState, CmdResult};

fn repo(app: &AppHandle, project_id: &str, cwd: Option<String>) -> CmdResult<Repo> {
    let state = app.state::<AppState>();
    let s = state.settings.lock().unwrap();
    let p = s.projects.iter().find(|p| p.id == project_id).ok_or("no such project")?;
    Ok(Repo::new(p.location.clone(), cwd.filter(|c| !c.is_empty()).unwrap_or_else(|| p.path.clone())))
}

/// Runs `f` on a blocking thread with the project's repo.
async fn with<T: Send + 'static>(
    app: AppHandle,
    project_id: String,
    cwd: Option<String>,
    f: impl FnOnce(&Repo) -> Result<T, String> + Send + 'static,
) -> CmdResult<T> {
    let r = repo(&app, &project_id, cwd)?;
    tauri::async_runtime::spawn_blocking(move || f(&r)).await.map_err(err)?
}

#[tauri::command]
pub async fn git_status(app: AppHandle, project_id: String, cwd: Option<String>) -> CmdResult<Status> {
    with(app, project_id, cwd, git::status).await
}

#[tauri::command]
pub async fn git_diff(
    app: AppHandle,
    project_id: String,
    cwd: Option<String>,
    path: String,
    staged: bool,
    untracked: bool,
) -> CmdResult<String> {
    with(app, project_id, cwd, move |r| git::diff(r, &path, staged, untracked)).await
}

#[tauri::command]
pub async fn git_stage(app: AppHandle, project_id: String, cwd: Option<String>, paths: Vec<String>) -> CmdResult<()> {
    with(app, project_id, cwd, move |r| git::stage(r, &paths)).await
}

#[tauri::command]
pub async fn git_unstage(app: AppHandle, project_id: String, cwd: Option<String>, paths: Vec<String>) -> CmdResult<()> {
    with(app, project_id, cwd, move |r| git::unstage(r, &paths)).await
}

#[tauri::command]
pub async fn git_discard(
    app: AppHandle,
    project_id: String,
    cwd: Option<String>,
    tracked: Vec<String>,
    untracked: Vec<String>,
) -> CmdResult<()> {
    with(app, project_id, cwd, move |r| git::discard(r, &tracked, &untracked)).await
}

#[tauri::command]
pub async fn git_commit(
    app: AppHandle,
    project_id: String,
    cwd: Option<String>,
    message: String,
    amend: bool,
    all: bool,
) -> CmdResult<String> {
    with(app, project_id, cwd, move |r| git::commit(r, &message, amend, all)).await
}

#[tauri::command]
pub async fn git_branches(app: AppHandle, project_id: String, cwd: Option<String>) -> CmdResult<Vec<Branch>> {
    with(app, project_id, cwd, git::branches).await
}

#[tauri::command]
pub async fn git_switch(app: AppHandle, project_id: String, cwd: Option<String>, branch: String, create: bool) -> CmdResult<()> {
    with(app, project_id, cwd, move |r| git::switch(r, &branch, create)).await
}

#[tauri::command]
pub async fn git_sync(app: AppHandle, project_id: String, cwd: Option<String>, op: SyncOp) -> CmdResult<String> {
    with(app, project_id, cwd, move |r| git::sync(r, op)).await
}

#[tauri::command]
pub async fn git_log(app: AppHandle, project_id: String, cwd: Option<String>, limit: usize) -> CmdResult<Vec<GraphRow>> {
    with(app, project_id, cwd, move |r| git::log(r, limit.min(2000))).await
}

#[tauri::command]
pub async fn git_commit_files(app: AppHandle, project_id: String, cwd: Option<String>, hash: String) -> CmdResult<Vec<Change>> {
    with(app, project_id, cwd, move |r| git::commit_files(r, &hash)).await
}

#[tauri::command]
pub async fn git_commit_diff(app: AppHandle, project_id: String, cwd: Option<String>, hash: String, path: String) -> CmdResult<String> {
    with(app, project_id, cwd, move |r| git::commit_diff(r, &hash, &path)).await
}

#[tauri::command]
pub async fn files_list(app: AppHandle, project_id: String, cwd: Option<String>) -> CmdResult<Vec<String>> {
    with(app, project_id, cwd, |r| git::files(r, 50_000)).await
}

#[tauri::command]
pub async fn files_grep(app: AppHandle, project_id: String, cwd: Option<String>, pattern: String) -> CmdResult<Vec<GrepHit>> {
    with(app, project_id, cwd, move |r| git::grep(r, &pattern, 500)).await
}

#[tauri::command]
pub async fn file_read(app: AppHandle, project_id: String, cwd: Option<String>, path: String) -> CmdResult<String> {
    with(app, project_id, cwd, move |r| git::read_file(r, &path, 512 * 1024)).await
}

/// An image file as a `data:` URL, for the file viewer (up to 20 MB).
#[tauri::command]
pub async fn file_read_image(app: AppHandle, project_id: String, cwd: Option<String>, path: String) -> CmdResult<String> {
    with(app, project_id, cwd, move |r| git::read_image(r, &path, 20 * 1024 * 1024)).await
}

/// A commit message for what's staged (or everything, when nothing is), written by the project's
/// agent with its cheapest model. Returns the message and which model wrote it.
#[tauri::command]
pub async fn git_suggest_message(app: AppHandle, project_id: String, cwd: Option<String>) -> CmdResult<(String, String)> {
    let r = repo(&app, &project_id, cwd)?;
    let settings = app.state::<crate::AppState>().settings.lock().unwrap().clone();
    let project = settings.projects.iter().find(|p| p.id == project_id).cloned().ok_or("no such project")?;
    tauri::async_runtime::spawn_blocking(move || {
        let context = git::commit_context(&r, 40_000)?;
        if context.trim_end().ends_with("changes:") {
            return Err("There's nothing to commit.".to_string());
        }
        let question = format!("{COMMIT_PROMPT}\n\n{context}");
        // The project's agent first, then the others; each with its cheapest model.
        let first = project.default_agent.unwrap_or(settings.providers.default_agent);
        let mut tried = Vec::new();
        for kind in std::iter::once(first).chain([AgentKind::Claude, AgentKind::Codex, AgentKind::Opencode]) {
            if tried.contains(&kind) {
                continue;
            }
            tried.push(kind);
            let provider = settings.providers.get(kind);
            if !provider.enabled {
                continue;
            }
            let binary = provider.binary_on(&r.location);
            if r.location == Location::Local && oah_agents::probe::resolve(kind, &binary).is_none() {
                continue;
            }
            let cheap = oah_agents::oneshot::cheapest(kind, &binary, &r.location, &r.path);
            match oah_agents::oneshot::ask(kind, &binary, &r.location, &r.path, &cheap, &question) {
                Ok(reply) => {
                    let msg = oah_agents::oneshot::clean_message(&reply);
                    if !msg.is_empty() {
                        return Ok((msg, format!("{} · {}", kind_label(kind), cheap.label)));
                    }
                }
                Err(e) if e.contains("isn't installed") => continue,
                Err(e) => return Err(format!("{} couldn't write it: {e}", kind_label(kind))),
            }
        }
        Err("No agent CLI is set up to write it (Settings → Providers).".to_string())
    })
    .await
    .map_err(err)?
}

const COMMIT_PROMPT: &str = "Write the git commit message for the change below. Reply with the message only: no quotes, \
no code fences, no preamble. First line: a summary in the imperative mood, at most 72 characters, \
in the style of the recent subjects. Then, only if the change needs explaining, a blank line and \
a few short lines on what changed and why. Don't describe what's obvious from the summary.";

fn kind_label(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Claude => "Claude Code",
        AgentKind::Codex => "Codex",
        AgentKind::Opencode => "OpenCode",
        AgentKind::Opencode2 => "OpenCode 2",
    }
}

// ---- Worktrees and pull requests -------------------------------------------------------------

use oah_git::github::{self, PrState};
use oah_git::worktree::{self, Worktree};

#[tauri::command]
pub async fn worktree_list(app: AppHandle, project_id: String) -> CmdResult<Vec<Worktree>> {
    with(app, project_id, None, worktree::worktrees).await
}

/// A new worktree on a new branch `branch`, from the project's base branch, in its worktree
/// folder.
#[tauri::command]
pub async fn worktree_create(app: AppHandle, project_id: String, branch: String) -> CmdResult<Worktree> {
    let git_opts = {
        let state = app.state::<AppState>();
        let s = state.settings.lock().unwrap();
        s.projects.iter().find(|p| p.id == project_id).map(|p| p.git.clone()).ok_or("no such project")?
    };
    with(app, project_id, None, move |r| {
        let path = worktree::default_dir(r, &git_opts.worktree_dir, &branch);
        let base = worktree::default_base(r, &git_opts.base_branch);
        worktree::add(r, &path, branch.trim(), &base)
    })
    .await
}

#[tauri::command]
pub async fn worktree_remove(app: AppHandle, project_id: String, path: String, force: bool) -> CmdResult<()> {
    let delete_branch = {
        let state = app.state::<AppState>();
        let s = state.settings.lock().unwrap();
        s.projects.iter().find(|p| p.id == project_id).map(|p| p.git.delete_branch_on_remove).unwrap_or(false)
    };
    with(app, project_id, None, move |r| worktree::remove(r, &path, force, delete_branch)).await
}

#[tauri::command]
pub async fn pr_state(app: AppHandle, project_id: String, cwd: Option<String>) -> CmdResult<PrState> {
    with(app, project_id, cwd, |r| Ok(github::pr_state(r))).await
}

#[tauri::command]
pub async fn pr_create(
    app: AppHandle,
    project_id: String,
    cwd: Option<String>,
    title: String,
    body: String,
    base: String,
    draft: bool,
) -> CmdResult<String> {
    with(app, project_id, cwd, move |r| github::pr_create(r, &title, &body, &base, draft)).await
}
