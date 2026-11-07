// Chats: the list for the sidebar, and a transcript per opened chat, kept current from the
// app's `chat-event` stream.

import { toast } from "sonner"
import { create } from "zustand"
import { emptyTranscript, reduce, replay, type ChatEvent, type ChatMeta, type Decision, type Transcript } from "@/lib/chat"
import { providerFor } from "@/lib/agents"
import { ipc, onChatEvent } from "@/lib/ipc"
import { notify } from "@/lib/notify"
import type { AgentKind, PermissionMode } from "@/lib/types"
import { useApp } from "./app"
import { draftKey, drafts } from "./drafts"

type Image = { mime: string; data: string; name?: string }

/** A message typed while the agent worked: it goes when the turn ends, or now if asked. */
export type Queued = { id: string; text: string; images: Image[] }

type ChatsStore = {
  metas: ChatMeta[]
  transcripts: Record<string, Transcript>
  /** Per-chat permission override, chosen in the composer. */
  permission: Record<string, PermissionMode>
  /** Per chat, messages waiting for the running turn to end, in order. */
  queued: Record<string, Queued[]>

  init(): Promise<void>
  refresh(): Promise<void>
  open(chatId: string): Promise<void>
  /** Sends a prompt; with no chat yet, creates one in the current project first. */
  send(
    chatId: string | null,
    text: string,
    opts: { agent: AgentKind; acpId?: string | null; model?: string; effort?: string; permission: PermissionMode; cwd?: string; images?: { mime: string; data: string }[] },
  ): Promise<void>
  /** `/compact`: the agent summarizes the conversation so far. */
  compact(chatId: string, instructions: string, permission: PermissionMode): Promise<void>
  /** `/clear`: the agent starts a fresh session from the next message. */
  clear(chatId: string): Promise<void>
  interrupt(chatId: string): Promise<void>
  answer(chatId: string, requestId: string, decision: Decision): Promise<void>
  answerQuestion(chatId: string, requestId: string, answers: Record<string, string[]>): Promise<void>
  setModel(chatId: string, model: string): Promise<void>
  /** Reasoning effort for the chat (empty: the model's default). */
  setEffort(chatId: string, effort: string): Promise<void>
  remove(chatId: string): Promise<void>
  rename(chatId: string, title: string): Promise<void>
  setPermission(chatId: string, mode: PermissionMode): void
  /** Holds a message until the chat's turn ends; then the first waiting one is sent. */
  enqueue(chatId: string, text: string, images: Image[]): void
  /** Takes a message off the queue (to edit it, or drop it). */
  unqueue(chatId: string, id: string): Queued | undefined
  /** Sends now, into the turn that's running: Claude Code takes it at its next step and Codex
   *  steers with it; an agent that can't take one mid-turn is stopped first. */
  steer(chatId: string, text: string, images: Image[]): Promise<void>
  /** Sends a queued message now (the first one, without an id). */
  sendQueued(chatId: string, id?: string): Promise<void>
}

let started = false

export const useChats = create<ChatsStore>((set, get) => ({
  metas: [],
  transcripts: {},
  permission: {},
  queued: {},

  async init() {
    if (started) return
    started = true
    await onChatEvent((chatId, event) => apply(chatId, event))
    await get().refresh()
  },

  async refresh() {
    const metas = await ipc.chatList().catch(() => [] as ChatMeta[])
    set({ metas })
  },

  async open(chatId) {
    useApp.setState({ chatId, view: "chat" })
    import("./git").then((m) => m.useGit.getState().open(null))
    if (get().transcripts[chatId]) return
    const events = await ipc.chatEvents(chatId).catch(() => [] as ChatEvent[])
    const running = await ipc.chatRunning().catch(() => [] as string[])
    const t = replay(events)
    set((s) => ({ transcripts: { ...s.transcripts, [chatId]: { ...t, running: running.includes(chatId) && t.running } } }))
  },

  async send(chatId, text, opts) {
    const app = useApp.getState()
    let id = chatId
    try {
      if (!id) {
        if (!app.projectId) return
        const meta = await ipc.chatCreate(app.projectId, opts.agent, opts.acpId ?? null, opts.model ?? null, opts.effort ?? null, opts.cwd ?? null)
        id = meta.id
        set((s) => ({ metas: [meta, ...s.metas], transcripts: { ...s.transcripts, [meta.id]: emptyTranscript() }, permission: { ...s.permission, [meta.id]: opts.permission } }))
        useApp.setState({ chatId: meta.id })
      }
      const chat = id
      set((s) => ({ transcripts: { ...s.transcripts, [chat]: { ...(s.transcripts[chat] ?? emptyTranscript()), running: true } } }))
      const images = (opts.images ?? []).map(({ mime, data }) => ({ mime, data }))
      await ipc.chatSend(chat, { id: crypto.randomUUID(), text, images }, get().permission[chat] ?? opts.permission)
      get().refresh()
    } catch (e) {
      const msg = String(e)
      // Not signed in: the CLI's sign-in is one click away.
      const login = msg.includes("isn't signed in") && (opts.agent === "opencode" || opts.agent === "claude" || opts.agent === "codex") ? opts.agent : null
      // Sign in where the project is: this computer, or inside its WSL distro.
      const meta = get().metas.find((m) => m.id === id)
      const project = useApp.getState().settings?.projects.find((p) => p.id === (meta?.projectId ?? useApp.getState().projectId))
      const where = project?.location.kind === "wsl" ? project.location : null
      const canSignIn = login && project?.location.kind !== "ssh"
      toast.error(msg, canSignIn ? { action: { label: "Sign in", onClick: () => void ipc.providerLogin(login, where).catch((err) => toast.error(String(err))) } } : undefined)
      if (id) apply(id, { type: "error", message: msg })
    }
  },

  async compact(chatId, instructions, permission) {
    await ipc.chatCompact(chatId, instructions, get().permission[chatId] ?? permission).catch((e) => {
      toast.error(String(e))
      apply(chatId, { type: "error", message: String(e) })
    })
  },

  async clear(chatId) {
    await ipc.chatClear(chatId).catch((e) => toast.error(String(e)))
    get().refresh()
  },

  async interrupt(chatId) {
    await ipc.chatInterrupt(chatId).catch((e) => toast.error(String(e)))
  },

  async answer(chatId, requestId, decision) {
    await ipc.chatAnswer(chatId, requestId, decision).catch((e) => toast.error(String(e)))
  },

  async answerQuestion(chatId, requestId, answers) {
    await ipc.chatAnswerQuestion(chatId, requestId, answers).catch((e) => toast.error(String(e)))
  },

  async setModel(chatId, model) {
    const meta = await ipc.chatSetModel(chatId, model).catch((e) => (toast.error(String(e)), null))
    if (meta) set((s) => ({ metas: s.metas.map((m) => (m.id === chatId ? meta : m)) }))
  },

  async setEffort(chatId, effort) {
    const meta = await ipc.chatSetEffort(chatId, effort).catch((e) => (toast.error(String(e)), null))
    if (meta) set((s) => ({ metas: s.metas.map((m) => (m.id === chatId ? meta : m)) }))
  },

  async remove(chatId) {
    await ipc.chatDelete(chatId).catch((e) => toast.error(String(e)))
    set((s) => {
      const { [chatId]: _, ...rest } = s.transcripts
      const { [chatId]: __, ...queued } = s.queued
      return { metas: s.metas.filter((m) => m.id !== chatId), transcripts: rest, queued }
    })
    drafts.drop(draftKey(chatId, ""))
    if (useApp.getState().chatId === chatId) useApp.setState({ chatId: null })
  },

  async rename(chatId, title) {
    const meta = await ipc.chatRename(chatId, title).catch(() => null)
    if (meta) set((s) => ({ metas: s.metas.map((m) => (m.id === chatId ? meta : m)) }))
  },

  setPermission(chatId, mode) {
    set((s) => ({ permission: { ...s.permission, [chatId]: mode } }))
    ipc.chatSetPermission(chatId, mode).catch(() => {})
  },

  enqueue(chatId, text, images) {
    const item = { id: crypto.randomUUID(), text, images }
    set((s) => ({ queued: { ...s.queued, [chatId]: [...(s.queued[chatId] ?? []), item] } }))
  },

  unqueue(chatId, id) {
    const item = get().queued[chatId]?.find((q) => q.id === id)
    if (item) set((s) => ({ queued: { ...s.queued, [chatId]: s.queued[chatId].filter((q) => q.id !== id) } }))
    return item
  },

  async steer(chatId, text, images) {
    const opts = sendOpts(chatId, images)
    if (!opts) return
    if (get().transcripts[chatId]?.running && (opts.agent === "opencode" || opts.agent === "acp")) await get().interrupt(chatId)
    await get().send(chatId, text, opts)
  },

  async sendQueued(chatId, id) {
    const item = get().unqueue(chatId, id ?? get().queued[chatId]?.[0]?.id ?? "")
    if (!item) return
    if (get().transcripts[chatId]?.running) return get().steer(chatId, item.text, item.images)
    const opts = sendOpts(chatId, item.images)
    if (opts) await get().send(chatId, item.text, opts)
  },
}))

/** How an open chat sends: its agent, and the permission chosen for it (else the agent's default). */
function sendOpts(chatId: string, images: Image[]) {
  const meta = useChats.getState().metas.find((m) => m.id === chatId)
  if (!meta) return null
  const settings = useApp.getState().settings
  const provider = settings ? providerFor(settings, { kind: meta.agent, acpId: meta.acpAgent }) : null
  const permission = useChats.getState().permission[chatId] ?? provider?.permissionMode ?? "ask"
  return { agent: meta.agent, acpId: meta.acpAgent, permission, images: images.map(({ mime, data }) => ({ mime, data })) }
}

/** Batches events per animation frame so streaming text doesn't re-render per token. */
let queue: [string, ChatEvent][] = []
let scheduled = false
function apply(chatId: string, event: ChatEvent) {
  notify(event, useChats.getState().metas.find((m) => m.id === chatId))
  if (event.type === "permissionChanged") useChats.setState((s) => ({ permission: { ...s.permission, [chatId]: event.mode } }))
  queue.push([chatId, event])
  if (scheduled) return
  scheduled = true
  requestAnimationFrame(() => {
    scheduled = false
    const batch = queue
    queue = []
    useChats.setState((s) => {
      const transcripts = { ...s.transcripts }
      for (const [id, e] of batch) transcripts[id] = reduce(transcripts[id] ?? emptyTranscript(), e)
      return { transcripts }
    })
    if (batch.some(([, e]) => e.type === "turnFinished" || e.type === "started")) useChats.getState().refresh()
    // A turn that ended by itself sends the next queued message; one that was stopped holds them.
    const ended = new Set(batch.flatMap(([id, e]) => (e.type === "turnFinished" && !e.interrupted ? [id] : [])))
    const s = useChats.getState()
    for (const id of ended) if (s.queued[id]?.length && !s.transcripts[id]?.running) void s.sendQueued(id)
  })
}
