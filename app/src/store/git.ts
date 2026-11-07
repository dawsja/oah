// Git state for the workspace on screen: its status (polled while visible and refreshed after
// every agent turn and git action), and what the main area is showing besides the chat.

import { useEffect } from "react"
import { toast } from "sonner"
import { create } from "zustand"
import { git, type Scope, type Status } from "@/lib/git"
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
  refresh(scope: Scope): Promise<void>
  /** Runs a git action with a busy label, then refreshes. */
  run<T>(scope: Scope, label: string, f: () => Promise<T>): Promise<T | undefined>
  open(v: Viewer | null): void
}

export const key = (s: Scope) => `${s.projectId}|${s.cwd ?? ""}`

export const useGit = create<GitStore>((set, get) => ({
  status: {},
  errors: {},
  busy: null,
  viewer: null,

  async refresh(scope) {
    try {
      const st = await git.status(scope)
      set((s) => ({ status: { ...s.status, [key(scope)]: st }, errors: { ...s.errors, [key(scope)]: null } }))
    } catch (e) {
      set((s) => ({ errors: { ...s.errors, [key(scope)]: String(e) } }))
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

/** The folder the side panel works on: the open chat's (a worktree) or the project's. */
export function useScope(): Scope | null {
  const project = useProject()
  const chatId = useApp((s) => s.chatId)
  const meta = useChats((s) => s.metas.find((m) => m.id === chatId))
  if (!project) return null
  const cwd = meta && meta.projectId === project.id && meta.cwd && meta.cwd !== project.path ? meta.cwd : null
  return { projectId: project.id, cwd }
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
    const refresh = () => document.visibilityState === "visible" && useGit.getState().refresh(scope)
    refresh()
    const t = setInterval(refresh, remote ? 15_000 : 4_000)
    return () => clearInterval(t)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [k, remote, turns])
  return { status: status ?? null, error: error ?? null, loading: status === undefined && !error }
}
