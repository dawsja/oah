// Git state for the workspace on screen: its status (polled while visible and refreshed after
// every agent turn and git action), and what the main area is showing besides the chat.

import { useEffect } from "react"
import { toast } from "sonner"
import { create } from "zustand"
import { git, worktrees as wt, type Scope, type Status, type Worktree } from "@/lib/git"
import { useApp, useProject } from "./app"
import { useChats } from "./chats"

/** Something opened from the side panel, shown in the main area instead of the chat. */
export type Viewer =
  | { kind: "diff"; path: string; staged: boolean; untracked: boolean }
  | { kind: "commit"; hash: string; path: string; subject: string }
  | { kind: "file"; path: string; line?: number }

type GitStore = {
  status: Record<string, Status | null>
  errors: Record<string, string | null>
  busy: string | null
  viewer: Viewer | null
  /** Worktrees per project (empty for folders that aren't repositories). */
  worktrees: Record<string, Worktree[]>
  loadWorktrees(projectId: string): Promise<void>
  refresh(scope: Scope): Promise<void>
  /** Runs a git action with a busy label, then refreshes. */
  run<T>(scope: Scope, label: string, f: () => Promise<T>): Promise<T | undefined>
  open(v: Viewer | null): void
}

export const key = (s: Scope) => `${s.projectId}|${s.cwd ?? ""}`

const inflight = new Set<string>()
const again = new Set<string>()

/** One timer per folder, however many panels show its status. */
const pollers = new Map<string, { users: number; timer: ReturnType<typeof setInterval> }>()

function watch(scope: Scope, every: number): () => void {
  const k = key(scope)
  const p = pollers.get(k)
  if (p) p.users++
  else {
    const refresh = () => document.visibilityState === "visible" && void useGit.getState().refresh(scope)
    pollers.set(k, { users: 1, timer: setInterval(refresh, every) })
  }
  return () => {
    const p = pollers.get(k)
    if (p && --p.users === 0) {
      clearInterval(p.timer)
      pollers.delete(k)
    }
  }
}

export const useGit = create<GitStore>((set, get) => ({
  status: {},
  errors: {},
  busy: null,
  viewer: null,
  worktrees: {},

  async loadWorktrees(projectId) {
    const list = await wt.list(projectId).catch(() => [] as Worktree[])
    set((s) => ({ worktrees: { ...s.worktrees, [projectId]: list } }))
  },

  async refresh(scope) {
    const k = key(scope)
    // One `git status` per folder at a time (three processes, or ssh connections); asks that
    // come meanwhile get one more run after it, not one each.
    if (inflight.has(k)) {
      again.add(k)
      return
    }
    inflight.add(k)
    try {
      const st = await git.status(scope)
      // Unchanged (most polls): nothing re-renders.
      const same = JSON.stringify(get().status[k]) === JSON.stringify(st)
      if (!same || get().errors[k]) set((s) => ({ status: same ? s.status : { ...s.status, [k]: st }, errors: { ...s.errors, [k]: null } }))
    } catch (e) {
      set((s) => ({ errors: { ...s.errors, [k]: String(e) } }))
    } finally {
      inflight.delete(k)
      if (again.delete(k)) void get().refresh(scope)
    }
  },

  async run(scope, label, f) {
    set({ busy: label })
    try {
      return await f()
    } catch (e) {
      toast.error(String(e))
      return undefined
    } finally {
      set({ busy: null })
      await get().refresh(scope)
    }
  },

  open(viewer) {
    set({ viewer })
  },
}))

function scopeOf(project: { id: string; path: string } | null, chatId: string | null, meta: { projectId: string; cwd: string } | undefined, draftCwd: string | null): Scope | null {
  if (!project) return null
  const open = meta && meta.projectId === project.id ? meta.cwd : chatId ? null : draftCwd
  const cwd = open && open !== project.path ? open : null
  return { projectId: project.id, cwd }
}

/** The folder the side panel works on: the open chat's (a worktree) or the project's. */
export function useScope(): Scope | null {
  const project = useProject()
  const chatId = useApp((s) => s.chatId)
  const meta = useChats((s) => s.metas.find((m) => m.id === chatId))
  const draftCwd = useApp((s) => s.draftCwd)
  return scopeOf(project, chatId, meta, draftCwd)
}

/** `useScope` outside React, as things are now. */
export function currentScope(): Scope | null {
  const { settings, projectId, chatId, draftCwd } = useApp.getState()
  const project = settings?.projects.find((p) => p.id === projectId) ?? null
  return scopeOf(project, chatId, useChats.getState().metas.find((m) => m.id === chatId), draftCwd)
}

/** The scope's status, kept fresh: polled every few seconds (slower over SSH) and after turns. */
export function useStatus(scope: Scope | null) {
  const status = useGit((s) => (scope ? s.status[key(scope)] : undefined))
  const error = useGit((s) => (scope ? s.errors[key(scope)] : null))
  const remote = useProject()?.location.kind === "ssh"
  const k = scope ? key(scope) : ""
  const turns = useChats((s) => Object.values(s.transcripts).filter((t) => !t.running).length)
  useEffect(() => {
    if (!scope) return
    return watch(scope, remote ? 15_000 : 4_000)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [k, remote])
  // Now, and after each turn (the agent may have changed files); shared by every panel.
  useEffect(() => {
    if (scope && document.visibilityState === "visible") void useGit.getState().refresh(scope)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [k, turns])
  return { status: status ?? null, error: error ?? null, loading: status === undefined && !error }
}
