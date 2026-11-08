// The phone app's state: paired desktops, the link to the one in use, its projects and chats,
// and the open chat's transcript (folded with the desktop's own `reduce`).

import { toast } from "sonner"
import { create } from "zustand"
import { emptyTranscript, reduce, replay, type ChatEvent, type ChatMeta, type Decision, type Transcript } from "@/lib/chat"
import type { AgentKind, PermissionMode, Project } from "@/lib/types"
import { api, native, phoneName, type AgentInfo, type Catalog, type Desktop, type Image, type LinkEvent, type LinkState, type Push, type Waiting } from "./rpc"
import { go } from "./route"

/** A message typed while the agent worked: it goes when the turn ends, or now if asked. */
export type Queued = { id: string; text: string; images: Image[] }

type Store = {
  desktops: Desktop[]
  loaded: boolean
  desktopId: string | null
  link: LinkState
  linkError: string | null
  desktopName: string
  projects: Project[]
  agents: AgentInfo[]
  defaultAgent: AgentKind
  metas: ChatMeta[]
  /** Chats with their agent running on the desktop, and those mid-turn. */
  running: string[]
  busy: string[]
  /** Chats waiting for an answer from you, and for what. */
  waiting: Record<string, Waiting>
  /** The chat open on screen. */
  chatId: string | null
  transcripts: Record<string, Transcript>
  queued: Record<string, Queued[]>
  /** Per chat, the permission picked here (else the agent's default on the desktop). */
  permission: Record<string, PermissionMode>
  catalogs: Record<string, Catalog>

  init(): Promise<void>
  pair(o: { link?: string; addresses?: string[]; code?: string; name?: string }): Promise<Desktop>
  forget(id: string): Promise<void>
  connect(id: string): Promise<void>
  refresh(): Promise<void>
  open(chatId: string): Promise<void>
  leave(): void
  /** Sends a prompt; with no chat yet (`chatId` null), creates one in `projectId` first. */
  send(chatId: string | null, text: string, images: Image[], o?: { projectId?: string; agent?: AgentInfo; model?: string }): Promise<string | null>
  steer(chatId: string, text: string, images: Image[]): Promise<void>
  enqueue(chatId: string, text: string, images: Image[]): void
  unqueue(chatId: string, id: string): void
  sendQueued(chatId: string): Promise<void>
  interrupt(chatId: string): Promise<void>
  answer(chatId: string, requestId: string, decision: Decision): Promise<void>
  answerQuestion(chatId: string, requestId: string, answers: Record<string, string[]>): Promise<void>
  setModel(chatId: string, model: string, effort: string): Promise<void>
  setPermission(chatId: string, mode: PermissionMode): void
  compact(chatId: string): Promise<void>
  clear(chatId: string): Promise<void>
  catalog(agent: AgentInfo, projectId: string | null): Promise<Catalog | null>
}

const LAST = "oah.mobile.desktop"
const fail = (e: unknown) => toast.error(String(e))

export const useMobile = create<Store>((set, get) => ({
  desktops: [],
  loaded: false,
  desktopId: null,
  link: "idle",
  linkError: null,
  desktopName: "",
  projects: [],
  agents: [],
  defaultAgent: "claude",
  metas: [],
  running: [],
  busy: [],
  waiting: {},
  chatId: null,
  transcripts: {},
  queued: {},
  permission: {},
  catalogs: {},

  async init() {
    const desktops = await native.desktops().catch(() => [] as Desktop[])
    set({ desktops, loaded: true })
    const last = localStorage.getItem(LAST)
    const pick = desktops.find((d) => d.id === last) ?? (desktops.length === 1 ? desktops[0] : null)
    if (pick) await get().connect(pick.id)
    // Back from the background: check the link (the phone may have slept through it dropping).
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "visible" && get().desktopId) void native.wake().catch(() => {})
    })
  },

  async pair(o) {
    const d = await native.pair({ ...o, name: o.name || phoneName() })
    set((s) => ({ desktops: [d, ...s.desktops.filter((x) => x.id !== d.id)] }))
    return d
  },

  async forget(id) {
    await native.forget(id)
    if (localStorage.getItem(LAST) === id) localStorage.removeItem(LAST)
    localStorage.removeItem(cacheKey(id))
    set((s) => ({ desktops: s.desktops.filter((d) => d.id !== id), ...(s.desktopId === id ? { desktopId: null, link: "idle" as const, metas: [], projects: [], transcripts: {} } : {}) }))
  },

  async connect(id) {
    if (get().desktopId === id && get().link !== "idle" && get().link !== "revoked") return
    localStorage.setItem(LAST, id)
    const d = get().desktops.find((x) => x.id === id)
    // The last list seen from this desktop shows (read-only) until it answers.
    set({ desktopId: id, link: "connecting", linkError: null, desktopName: d?.name ?? "", transcripts: {}, running: [], busy: [], waiting: {}, ...cached(id) })
    await native.connect(id, onLink).catch((e) => set({ link: "offline", linkError: String(e) }))
  },

  async refresh() {
    const [chats, projects, agents] = await Promise.all([api.chats(), api.projects(), api.agents()]).catch((e) => {
      set({ linkError: String(e) })
      return [null, null, null]
    })
    if (!chats || !projects || !agents) return
    const next = {
      metas: [...chats.metas].sort((a, b) => b.updatedAt - a.updatedAt),
      projects,
      agents: agents.agents,
      defaultAgent: agents.defaultAgent,
    }
    set({ ...next, running: chats.running, busy: chats.busy, waiting: chats.waiting ?? {} })
    const id = get().desktopId
    if (id) cache(id, next)
  },

  async open(chatId) {
    const prev = get().chatId
    if (prev && prev !== chatId) void api.close(prev).catch(() => {})
    set({ chatId })
    if (get().link !== "connected") return
    try {
      const snap = await api.open(chatId)
      if (get().chatId !== chatId) return
      const t = replay(snap.events, snap.busy)
      // Events that came in while it loaded were held by the desktop and follow this answer.
      set((s) => ({ transcripts: { ...s.transcripts, [chatId]: t } }))
    } catch (e) {
      fail(e)
    }
  },

  leave() {
    const id = get().chatId
    if (id) void api.close(id).catch(() => {})
    set({ chatId: null })
  },

  async send(chatId, text, images, o = {}) {
    let id = chatId
    try {
      if (!id) {
        if (!o.projectId || !o.agent) return null
        const meta = await api.create(o.projectId, o.agent.kind, o.agent.acpId, o.model || null)
        id = meta.id
        // Open it before sending, so every event of the first turn arrives here.
        const snap = await api.open(meta.id)
        set((s) => ({ metas: [meta, ...s.metas], transcripts: { ...s.transcripts, [meta.id]: replay(snap.events, snap.busy) }, chatId: meta.id }))
      }
      const chat = id
      set((s) => ({ transcripts: { ...s.transcripts, [chat]: { ...(s.transcripts[chat] ?? emptyTranscript()), running: true } } }))
      await api.send(chat, text, images, get().permission[chat] ?? null)
      return chat
    } catch (e) {
      fail(e)
      if (id) push(id, { type: "error", message: String(e) })
      return id
    }
  },

  async steer(chatId, text, images) {
    const meta = get().metas.find((m) => m.id === chatId)
    // An agent that can't take a message mid-turn is stopped first (as on the desktop).
    if (get().transcripts[chatId]?.running && (meta?.agent === "opencode" || meta?.agent === "acp")) await get().interrupt(chatId)
    await get().send(chatId, text, images)
  },

  enqueue(chatId, text, images) {
    set((s) => ({ queued: { ...s.queued, [chatId]: [...(s.queued[chatId] ?? []), { id: crypto.randomUUID(), text, images }] } }))
  },

  unqueue(chatId, id) {
    set((s) => ({ queued: { ...s.queued, [chatId]: (s.queued[chatId] ?? []).filter((q) => q.id !== id) } }))
  },

  async sendQueued(chatId) {
    const item = get().queued[chatId]?.[0]
    if (!item) return
    get().unqueue(chatId, item.id)
    if (get().transcripts[chatId]?.running) await get().steer(chatId, item.text, item.images)
    else await get().send(chatId, item.text, item.images)
  },

  async interrupt(chatId) {
    await api.interrupt(chatId).catch(fail)
  },
  async answer(chatId, requestId, decision) {
    await api.answer(chatId, requestId, decision).catch(fail)
  },
  async answerQuestion(chatId, requestId, answers) {
    await api.answerQuestion(chatId, requestId, answers).catch(fail)
  },

  async setModel(chatId, model, effort) {
    const meta = get().metas.find((m) => m.id === chatId)
    let next: ChatMeta | null = null
    if (model !== meta?.model) next = await api.setModel(chatId, model).catch((e) => (fail(e), null))
    if (effort !== meta?.effort) next = (await api.setEffort(chatId, effort).catch((e) => (fail(e), null))) ?? next
    if (next) set((s) => ({ metas: s.metas.map((m) => (m.id === chatId ? next! : m)) }))
  },

  setPermission(chatId, mode) {
    set((s) => ({ permission: { ...s.permission, [chatId]: mode } }))
    api.setPermission(chatId, mode).catch(() => {})
  },

  async compact(chatId) {
    await api.compact(chatId, "", get().permission[chatId] ?? null).catch(fail)
  },
  async clear(chatId) {
    await api.clear(chatId).catch(fail)
  },

  async catalog(agent, projectId) {
    const key = `${agent.kind}:${agent.acpId ?? ""}:${projectId ?? ""}`
    if (get().catalogs[key]) return get().catalogs[key]
    const c = await api.catalog(agent.kind, projectId).catch(() => null)
    if (c) set((s) => ({ catalogs: { ...s.catalogs, [key]: c } }))
    return c
  },
}))

type Cached = Pick<Store, "metas" | "projects" | "agents" | "defaultAgent">
const cacheKey = (id: string) => `oah.mobile.cache.${id}`
function cache(id: string, c: Cached) {
  try {
    localStorage.setItem(cacheKey(id), JSON.stringify({ ...c, metas: c.metas.slice(0, 200) }))
  } catch {
    // Full storage: the list just won't show offline.
  }
}
function cached(id: string): Cached {
  try {
    const c = JSON.parse(localStorage.getItem(cacheKey(id)) ?? "null") as Cached | null
    if (c?.metas && c.projects) return c
  } catch {
    // A damaged cache is the same as none.
  }
  return { metas: [], projects: [], agents: [], defaultAgent: "claude" }
}

function onLink(e: LinkEvent) {
  if (e.kind === "push") return onPush(e.push)
  const s = useMobile.getState()
  if (e.state === "connected") {
    useMobile.setState({ link: "connected", linkError: null, desktopName: e.desktopName || s.desktopName })
    void s.refresh()
    // Reopen the chat on screen: whatever happened while the link was down comes with it.
    if (s.chatId) void s.open(s.chatId)
  } else if (e.state === "revoked") {
    useMobile.setState({ link: "revoked", linkError: e.error ?? null })
  } else {
    useMobile.setState({ link: e.state, linkError: e.error ?? s.linkError })
  }
}

function onPush(p: Push) {
  const s = useMobile.getState()
  if (p.push === "chats") return void s.refresh()
  if (p.push === "resync") {
    if (s.chatId) void s.open(s.chatId)
    return
  }
  if (p.push === "chat") push(p.chatId, p.event)
}

/** Batches events per animation frame so streaming text doesn't re-render per token. */
let queue: [string, ChatEvent][] = []
let scheduled = false
function push(chatId: string, event: ChatEvent) {
  if (event.type === "permissionChanged") useMobile.setState((s) => ({ permission: { ...s.permission, [chatId]: event.mode } }))
  queue.push([chatId, event])
  if (scheduled) return
  scheduled = true
  requestAnimationFrame(() => {
    scheduled = false
    const batch = queue
    queue = []
    useMobile.setState((s) => {
      const transcripts = { ...s.transcripts }
      for (const [id, e] of batch) if (transcripts[id]) transcripts[id] = reduce(transcripts[id], e)
      return { transcripts }
    })
    // A turn that ended by itself sends the next queued message; one that was stopped holds them.
    const ended = new Set(batch.flatMap(([id, e]) => (e.type === "turnFinished" && !e.interrupted ? [id] : [])))
    const st = useMobile.getState()
    for (const id of ended) if (st.queued[id]?.length && !st.transcripts[id]?.running) void st.sendQueued(id)
  })
}

/** Leaves for the desktops list (the link stays up). */
export function toDesktops() {
  go("/desktops")
}
