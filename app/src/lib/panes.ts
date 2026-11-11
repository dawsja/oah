// The main area's panes: one, or two side by side, each showing a chat (or a new one) in any
// project. One of them is active: the sidebar, the side panel, the terminal and the title bar
// follow it. A chat is on screen in one pane at most.

import type { Viewer } from "@/store/git"

/** What one pane shows. */
export type Pane = {
  /** Stays the pane's for as long as it's open. */
  id: string
  projectId: string | null
  chatId: string | null
  /** The folder a new (unsent) chat will work in: a worktree, or null for the project. */
  draftCwd: string | null
  /** Something opened from the side panel, shown in the pane instead of its chat. */
  viewer: Viewer | null
}

export type PaneTarget = Partial<Omit<Pane, "id">>

export type Panes = { panes: Pane[]; activePane: string }

export const MAX_PANES = 2

/** The pane with `id`; the active one without an id, or when that pane has closed. */
export function paneOf(s: Panes, id?: string | null): Pane {
  return (id ? s.panes.find((p) => p.id === id) : undefined) ?? s.panes.find((p) => p.id === s.activePane) ?? s.panes[0]
}

/** `pane` showing `target`. Another project starts clean: no chat, worktree or file of the
 *  last one, unless the target names them. */
function show(pane: Pane, target: PaneTarget): Pane {
  const next = { ...pane, ...target }
  if (target.projectId !== undefined && target.projectId !== pane.projectId) {
    if (target.chatId === undefined) next.chatId = null
    if (target.draftCwd === undefined) next.draftCwd = null
    if (target.viewer === undefined) next.viewer = null
  }
  return next
}

/** Changes what a pane shows (the active one without an id). */
export function patchPane(s: Panes, target: PaneTarget, id?: string | null): Panes {
  const pane = paneOf(s, id)
  return { ...s, panes: s.panes.map((p) => (p.id === pane.id ? show(p, target) : p)) }
}

/** Shows `target` beside the active pane and moves there: in a new pane (`newId`) on the
 *  right, or in the other one when there are two already. Without a target, a new chat in the
 *  active pane's project. */
export function splitPanes(s: Panes, newId: string, target: PaneTarget = {}): Panes {
  const from = paneOf(s)
  const fresh: PaneTarget = { projectId: from.projectId, chatId: null, draftCwd: null, viewer: null, ...target }
  const other = s.panes.find((p) => p.id !== from.id)
  if (other && s.panes.length >= MAX_PANES) return { panes: s.panes.map((p) => (p.id === other.id ? { ...p, ...fresh } : p)), activePane: other.id }
  const pane: Pane = { id: newId, projectId: null, chatId: null, draftCwd: null, viewer: null, ...fresh }
  return { panes: [...s.panes, pane], activePane: pane.id }
}

/** Closes a pane (the active one without an id); the last one stays. */
export function closePane(s: Panes, id?: string | null): Panes {
  if (s.panes.length < 2) return s
  const gone = paneOf(s, id)
  const panes = s.panes.filter((p) => p.id !== gone.id)
  return { panes, activePane: panes.some((p) => p.id === s.activePane) ? s.activePane : panes[0].id }
}

/** Where a draft is kept: the chat's, or a new chat's in `projectId` (and worktree `cwd`). */
export const draftKey = (chatId: string | null, projectId: string, cwd?: string | null) => (chatId ? `chat:${chatId}` : `new:${projectId}:${cwd ?? ""}`)

const sameNewChat = (a: Pane, b: Pane) => a.chatId === null && b.chatId === null && a.projectId === b.projectId && (a.draftCwd ?? "") === (b.draftCwd ?? "")

/** Where a pane's new-chat draft is kept. Two panes on the same new chat each keep their own:
 *  the left one the usual key, the other one beside it. */
export function newChatKey(panes: Pane[], pane: Pane): string {
  const key = draftKey(null, pane.projectId ?? "", pane.draftCwd)
  const first = panes.find((p) => sameNewChat(p, pane))
  return first && first.id !== pane.id ? `${key}#2` : key
}

/** Draft keys to swap when the panes go from `prev` to `next`: a pane still on the same new
 *  chat whose key changed with what the other pane shows (it closed, or came to the same new
 *  chat) keeps what was typed in it. */
export function draftSwaps(prev: Pane[], next: Pane[]): [string, string][] {
  const swaps: [string, string][] = []
  for (const pane of next) {
    const before = prev.find((p) => p.id === pane.id)
    if (!before || !sameNewChat(before, pane)) continue
    const [from, to] = [newChatKey(prev, before), newChatKey(next, pane)]
    if (from !== to && !swaps.some(([a, b]) => a === to && b === from)) swaps.push([from, to])
  }
  return swaps
}
