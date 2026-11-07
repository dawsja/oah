// Chats: the list for the sidebar, and a transcript per opened chat, kept current from the
// app's `chat-event` stream.

import { toast } from "sonner"
import { create } from "zustand"
import { emptyTranscript, reduce, replay, type ChatEvent, type ChatMeta, type Decision, type Transcript } from "@/lib/chat"
import { ipc, onChatEvent } from "@/lib/ipc"
import { notify } from "@/lib/notify"
import type { AgentKind, PermissionMode } from "@/lib/types"
import { useApp } from "./app"

type ChatsStore = {
  metas: ChatMeta[]
  transcripts: Record<string, Transcript>
  /** Per-chat permission override, chosen in the composer. */
  permission: Record<string, PermissionMode>

  init(): Promise<void>
  refresh(): Promise<void>
  open(chatId: string): Promise<void>
  /** Sends a prompt; with no chat yet, creates one in the current project first. */
  send(
    chatId: string | null,
    text: string,
    opts: { agent: AgentKind; acpId?: string | null; model?: string; permission: PermissionMode; cwd?: string; images?: { mime: string; data: string }[] },
  ): Promise<void>
  /** `/compact`: the agent summarizes the conversation so far. */
  compact(chatId: string, instructions: string, permission: PermissionMode): Promise<void>
  /** `/clear`: the agent starts a fresh session from the next message. */
  clear(chatId: string): Promise<void>
  interrupt(chatId: string): Promise<void>
  answer(chatId: string, requestId: string, decision: Decision): Promise<void>
  answerQuestion(chatId: string, requestId: string, answers: Record<string, string[]>): Promise<void>
  setModel(chatId: string, model: string): Promise<void>
  remove(chatId: string): Promise<void>
  rename(chatId: string, title: string): Promise<void>
  setPermission(chatId: string, mode: PermissionMode): void
}

let started = false

export const useChats = create<ChatsStore>((set, get) => ({
  metas: [],
  transcripts: {},
  permission: {},

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
        const meta = await ipc.chatCreate(app.projectId, opts.agent, opts.acpId ?? null, opts.model ?? null, opts.cwd ?? null)
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
      toast.error(msg, login ? { action: { label: "Sign in", onClick: () => void ipc.providerLogin(login).catch((err) => toast.error(String(err))) } } : undefined)
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

  async remove(chatId) {
    await ipc.chatDelete(chatId).catch((e) => toast.error(String(e)))
    set((s) => {
      const { [chatId]: _, ...rest } = s.transcripts
      return { metas: s.metas.filter((m) => m.id !== chatId), transcripts: rest }
    })
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
}))

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
  })
}
