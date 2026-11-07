//! Git commands for the side panel. Each names the project and, optionally, the folder inside
//! it to use (a worktree); git runs where the project is.

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
async fn with<T: Send + 'static>(app: AppHandle, project_id: String, cwd: Option<String>, f: impl FnOnce(&Repo) -> Result<T, String> + Send + 'static) -> CmdResult<T> {
    let r = repo(&app, &project_id, cwd)?;
    tauri::async_runtime::spawn_blocking(move || f(&r)).await.map_err(err)?
}

#[tauri::command]
pub async fn git_status(app: AppHandle, project_id: String, cwd: Option<String>) -> CmdResult<Status> {
    with(app, project_id, cwd, git::status).await
}

#[tauri::command]
pub async fn git_diff(app: AppHandle, project_id: String, cwd: Option<String>, path: String, staged: bool, untracked: bool) -> CmdResult<String> {
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
pub async fn git_discard(app: AppHandle, project_id: String, cwd: Option<String>, tracked: Vec<String>, untracked: Vec<String>) -> CmdResult<()> {
    with(app, project_id, cwd, move |r| git::discard(r, &tracked, &untracked)).await
}

#[tauri::command]
pub async fn git_commit(app: AppHandle, project_id: String, cwd: Option<String>, message: String, amend: bool, all: bool) -> CmdResult<String> {
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
