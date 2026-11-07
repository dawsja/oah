// Git types (crates/oah-git) and the commands that run them in a project's folder.


export type ChangeKind = "modified" | "added" | "deleted" | "renamed" | "copied" | "typechanged" | "conflicted" | "untracked"
export type Change = { path: string; origPath: string | null; kind: ChangeKind; added: number; removed: number }
export type Status = {
  isRepo: boolean
  branch: string | null
  head: string | null
  upstream: string | null
  ahead: number
  behind: number
  staged: Change[]
  unstaged: Change[]
  untracked: Change[]
  conflicted: Change[]
}
export type Branch = { name: string; remote: boolean; current: boolean; upstream: string | null; updated: number }
export type Commit = { hash: string; parents: string[]; author: string; time: number; subject: string; refs: string[] }
export type GraphRow = { commit: Commit; lane: number; through: number[]; into: number[]; out: number[]; width: number }
export type GrepHit = { path: string; line: number; text: string }
export type SyncOp = "fetch" | "pull" | "push" | "sync"
export type Worktree = { path: string; branch: string | null; head: string; main: boolean; locked: boolean; prunable: boolean }
export type Check = { name: string; state: string; url: string | null }
export type PullRequest = {
  number: number
  title: string
  state: string
  draft: boolean
  url: string
  base: string
  head: string
  additions: number
  deletions: number
  review: string
  mergeState: string
  checks: Check[]
}
export type PrState = { kind: "unavailable"; reason: string } | { kind: "none"; branch: string; base: string } | { kind: "open"; pr: PullRequest }

/** Which folder: a project, and optionally a worktree inside it. */
export type Scope = { projectId: string; cwd: string | null }

async function call<T>(cmd: string, scope: Scope, args: Record<string, unknown> = {}): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core")
  return invoke<T>(cmd, { projectId: scope.projectId, cwd: scope.cwd, ...args })
}

export const git = {
  status: (s: Scope) => call<Status>("git_status", s),
  diff: (s: Scope, path: string, staged: boolean, untracked: boolean) => call<string>("git_diff", s, { path, staged, untracked }),
  stage: (s: Scope, paths: string[]) => call<void>("git_stage", s, { paths }),
  unstage: (s: Scope, paths: string[]) => call<void>("git_unstage", s, { paths }),
  discard: (s: Scope, tracked: string[], untracked: string[]) => call<void>("git_discard", s, { tracked, untracked }),
  commit: (s: Scope, message: string, amend: boolean, all: boolean) => call<string>("git_commit", s, { message, amend, all }),
  branches: (s: Scope) => call<Branch[]>("git_branches", s),
  switch: (s: Scope, branch: string, create: boolean) => call<void>("git_switch", s, { branch, create }),
  sync: (s: Scope, op: SyncOp) => call<string>("git_sync", s, { op }),
  log: (s: Scope, limit: number) => call<GraphRow[]>("git_log", s, { limit }),
  commitFiles: (s: Scope, hash: string) => call<Change[]>("git_commit_files", s, { hash }),
  commitDiff: (s: Scope, hash: string, path: string) => call<string>("git_commit_diff", s, { hash, path }),
  files: (s: Scope) => call<string[]>("files_list", s),
  grep: (s: Scope, pattern: string) => call<GrepHit[]>("files_grep", s, { pattern }),
  read: (s: Scope, path: string) => call<string>("file_read", s, { path }),
  prState: (s: Scope) => call<PrState>("pr_state", s),
  prCreate: (s: Scope, title: string, body: string, base: string, draft: boolean) => call<string>("pr_create", s, { title, body, base, draft }),
}

async function project<T>(cmd: string, projectId: string, args: Record<string, unknown> = {}): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core")
  return invoke<T>(cmd, { projectId, ...args })
}

export const worktrees = {
  list: (projectId: string) => project<Worktree[]>("worktree_list", projectId),
  create: (projectId: string, branch: string) => project<Worktree>("worktree_create", projectId, { branch }),
  remove: (projectId: string, path: string, force: boolean) => project<void>("worktree_remove", projectId, { path, force }),
}

/** Same folder, whichever slashes (and case on Windows paths). */
export function samePath(a: string | null | undefined, b: string | null | undefined): boolean {
  if (!a || !b) return false
  const n = (s: string) => s.replace(/\\/g, "/").replace(/\/+$/, "")
  const win = /^[a-z]:/i.test(a)
  return win ? n(a).toLowerCase() === n(b).toLowerCase() : n(a) === n(b)
}

export const KIND_LETTER: Record<ChangeKind, string> = {
  modified: "M",
  added: "A",
  deleted: "D",
  renamed: "R",
  copied: "C",
  typechanged: "T",
  conflicted: "!",
  untracked: "U",
}

export const KIND_COLOR: Record<ChangeKind, string> = {
  modified: "text-warn",
  added: "text-green",
  deleted: "text-red",
  renamed: "text-cyan",
  copied: "text-cyan",
  typechanged: "text-warn",
  conflicted: "text-red",
  untracked: "text-green",
}

export function changeCount(s: Status | null): number {
  if (!s) return 0
  return new Set([...s.staged, ...s.unstaged, ...s.untracked, ...s.conflicted].map((c) => c.path)).size
}
