// Chats: the list for the sidebar, and a transcript per opened chat, kept current from the
// app's `chat-event` stream.

import { toast } from "sonner"
import { create } from "zustand"
import { emptyTranscript, reduce, replay, type ChatEvent, type ChatMeta, type Decision, type Transcript, type Waiting } from "@/lib/chat"

export { needsYou, waitingOf } from "@/lib/chat"
import { ipc, onChatEvent, onChatsChanged } from "@/lib/ipc"
import { notify, setQueuedCheck } from "@/lib/notify"
import { isAcp, type AgentKind, type PermissionMode } from "@/lib/types"
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
  /** What each running chat waits on the user for, as the app last said (`chat_waiting`):
   *  it knows about chats this window never opened. */
  waiting: Record<string, Waiting>
  /** Chats with background work running, as the app last said (`chat_background`). */
  background: string[]
  /** Chats mid-turn, as the app last said (`chat_busy`): it knows about chats this window
   *  never opened (a phone or MCP runs them). */
  busy: string[]
  /** A chat opened to answer it: its waiting card takes the cursor once it shows. */
  focus: string | null

  init(): Promise<void>
  refresh(): Promise<void>
  open(chatId: string): Promise<void>
  /** Opens the chat on its approval or question, wherever the window was. */
  answerChat(chatId: string): Promise<void>
  /** Sends a prompt; with no chat yet, creates one in the current project first. True once
   *  it's sent; false (after saying why) when it wasn't. A new chat whose first message didn't
   *  go is removed again, and the message goes back in the new-chat box. */
  send(
    chatId: string | null,
    text: string,
    opts: { agent: AgentKind; model?: string; effort?: string; permission: PermissionMode; cwd?: string; images?: Image[] },
  ): Promise<boolean>
  /** `/compact`: the agent summarizes the conversation so far. */
  compact(chatId: string, instructions: string, permission: PermissionMode): Promise<void>
  /** `/clear`: the agent starts a fresh session from the next message. */
  clear(chatId: string): Promise<void>
  sleep(chatId: string): Promise<void>
  interrupt(chatId: string): Promise<void>
  /** Stops one background task. */
  stopTask(chatId: string, taskId: string): Promise<void>
  answer(chatId: string, requestId: string, decision: Decision): Promise<void>
  answerQuestion(chatId: string, requestId: string, answers: Record<string, string[]>): Promise<void>
  setModel(chatId: string, model: string): Promise<void>
  /** Reasoning effort for the chat (empty: the model's default). */
  setEffort(chatId: string, effort: string): Promise<void>
  remove(chatId: string): Promise<void>
  /** Asks first ("Delete … ?"), then removes it. */
  confirmRemove(chatId: string): Promise<void>
  rename(chatId: string, title: string): Promise<void>
  pin(chatId: string, pinned: boolean): Promise<void>
  setPermission(chatId: string, mode: PermissionMode): void
  /** Holds a message until the chat's turn ends; then the first waiting one is sent. */
  enqueue(chatId: string, text: string, images: Image[]): void
  /** Takes a message off the queue (to edit it, or drop it). */
  unqueue(chatId: string, id: string): Queued | undefined
  /** Sends now, into the turn that's running: Claude Code takes it at its next step and Codex
   *  steers with it; an agent that can't take one mid-turn is stopped first. */
  steer(chatId: string, text: string, images: Image[]): Promise<boolean>
  /** Sends a queued message now (the first one, without an id). */
  sendQueued(chatId: string, id?: string): Promise<void>
}

let started = false
setQueuedCheck((id) => (useChats.getState().queued[id]?.length ?? 0) > 0)

export const useChats = create<ChatsStore>((set, get) => ({
  metas: [],
  transcripts: {},
  permission: {},
  queued: {},
  waiting: {},
  background: [],
  busy: [],
  focus: null,

  async init() {
    if (started) return
    started = true
    await onChatEvent((chatId, event, seq) => apply(chatId, event, seq))
    // A chat started, renamed or deleted elsewhere (a phone) shows up here too.
    await onChatsChanged(() => void get().refresh())
    await get().refresh()
  },

  async refresh() {
    const [metas, waiting, background, busy] = await Promise.all([
      ipc.chatList().catch(() => [] as ChatMeta[]),
      ipc.chatWaiting().catch(() => ({})),
      ipc.chatBackground().catch(() => [] as string[]),
      ipc.chatBusy().catch(() => [] as string[]),
    ])
    set({ metas, waiting, background, busy })
  },

  async open(chatId) {
    useApp.setState({ chatId, view: "chat" })
    import("./git").then((m) => m.useGit.getState().open(null))
    if (get().transcripts[chatId] || loading.has(chatId)) return
    // Events that arrive while the snapshot loads wait, then the ones it doesn't have apply.
    loading.set(chatId, [])
    const snap = await ipc.chatSnapshot(chatId).catch(() => ({ events: [] as ChatEvent[], seq: 0, live: false, busy: false }))
    const held = loading.get(chatId) ?? []
    loading.delete(chatId)
    snapshotSeq.set(chatId, snap.seq)
    const t = held.filter(([seq]) => seq > snap.seq).reduce((t, [, e]) => reduce(t, e), replay(snap.events, snap.busy, snap.live))
    set((s) => ({ transcripts: { ...s.transcripts, [chatId]: t } }))
  },

  async answerChat(chatId) {
    const meta = get().metas.find((m) => m.id === chatId)
    if (!meta) return
    useApp.getState().selectProject(meta.projectId)
    set({ focus: chatId })
    await get().open(chatId)
  },

  async send(chatId, text, opts) {
    const app = useApp.getState()
    let id = chatId
    try {
      if (!id) {
        if (!app.projectId) return false
        const meta = await ipc.chatCreate(app.projectId, opts.agent, opts.model ?? null, opts.effort ?? null, opts.cwd ?? null)
        id = meta.id
        set((s) => ({ metas: [meta, ...s.metas], transcripts: { ...s.transcripts, [meta.id]: emptyTranscript() }, permission: { ...s.permission, [meta.id]: opts.permission } }))
        useApp.setState({ chatId: meta.id })
      }
      const chat = id
      set((s) => ({ transcripts: { ...s.transcripts, [chat]: { ...(s.transcripts[chat] ?? emptyTranscript()), running: true } } }))
      const images = (opts.images ?? []).map(({ mime, data }) => ({ mime, data }))
      await ipc.chatSend(chat, { id: crypto.randomUUID(), text, images }, get().permission[chat] ?? opts.permission)
      get().refresh()
      return true
    } catch (e) {
      const msg = String(e)
      // Not signed in: the CLI's sign-in is one click away.
      const login = msg.includes("isn't signed in") ? opts.agent : null
      // Sign in where the project is: this computer, or inside its WSL distro.
      const meta = get().metas.find((m) => m.id === id)
      const project = useApp.getState().settings?.projects.find((p) => p.id === (meta?.projectId ?? useApp.getState().projectId))
      const where = project?.location.kind === "wsl" ? project.location : null
      const canSignIn = login && project?.location.kind !== "ssh"
      toast.error(msg, canSignIn ? { action: { label: "Sign in", onClick: () => void ipc.providerLogin(login, where).catch((err) => toast.error(String(err))) } } : undefined)
      if (id && !chatId && app.projectId) {
        // Its first message never went: no empty chat is left, and the message is back in the
        // box it was typed in.
        drafts.set(draftKey(null, app.projectId, opts.cwd), text, (opts.images ?? []).map((i) => ({ ...i, name: i.name ?? "image" })))
        await get().remove(id)
        useApp.setState({ chatId: null, draftCwd: opts.cwd ?? null })
      } else if (id) {
        apply(id, { type: "error", message: msg })
      }
      return false
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

  async sleep(chatId) {
    try {
      await ipc.chatSleep(chatId)
      await get().refresh()
    } catch (e) {
      toast.error(String(e))
    }
  },

  async interrupt(chatId) {
    await ipc.chatInterrupt(chatId).catch((e) => toast.error(String(e)))
  },

  async stopTask(chatId, taskId) {
    await ipc.chatStopTask(chatId, taskId).catch((e) => toast.error(String(e)))
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
    // A delete that failed leaves the chat listed (it's still there).
    const ok = await ipc.chatDelete(chatId).then(
      () => true,
      (e) => (toast.error(`Couldn't delete the chat: ${e}`), false),
    )
    if (!ok) return
    set((s) => {
      const { [chatId]: _, ...rest } = s.transcripts
      const { [chatId]: __, ...queued } = s.queued
      return { metas: s.metas.filter((m) => m.id !== chatId), transcripts: rest, queued }
    })
    drafts.drop(draftKey(chatId, ""))
    if (useApp.getState().chatId === chatId) useApp.setState({ chatId: null })
  },

  async confirmRemove(chatId) {
    const meta = get().metas.find((m) => m.id === chatId)
    const running = get().transcripts[chatId]?.running || get().busy.includes(chatId)
    const { confirmAction } = await import("@/components/Prompt")
    const ok = await confirmAction({
      title: `Delete ${meta?.title ? `"${meta.title}"` : "this chat"}?`,
      description: `Its messages and attached images are deleted for good.${running ? " Its agent is working and stops now." : ""}`,
      confirm: "Delete",
      danger: true,
    })
    if (ok) await get().remove(chatId)
  },

  async rename(chatId, title) {
    const meta = await ipc.chatRename(chatId, title).catch(() => null)
    if (meta) set((s) => ({ metas: s.metas.map((m) => (m.id === chatId ? meta : m)) }))
  },

  async pin(chatId, pinned) {
    const meta = await ipc.chatPin(chatId, pinned).catch((e) => (toast.error(String(e)), null))
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
    if (!opts) return false
    // OpenCode and Antigravity take one prompt at a time: the running turn stops, then this goes.
    if (get().transcripts[chatId]?.running && isAcp(opts.agent)) await get().interrupt(chatId)
    return get().send(chatId, text, opts)
  },

  async sendQueued(chatId, id) {
    const item = get().unqueue(chatId, id ?? get().queued[chatId]?.[0]?.id ?? "")
    if (!item) return
    const opts = sendOpts(chatId, item.images)
    const ok = get().transcripts[chatId]?.running ? await get().steer(chatId, item.text, item.images) : opts ? await get().send(chatId, item.text, opts) : false
    // Didn't go: it stays first in line instead of disappearing.
    if (!ok) set((s) => ({ queued: { ...s.queued, [chatId]: [item, ...(s.queued[chatId] ?? [])] } }))
  },
}))

/** Copies the chat as Markdown (to paste into an issue or a PR description). */
export async function copyChatMarkdown(chatId: string) {
  try {
    const md = await ipc.chatMarkdown(chatId)
    const { writeText } = await import("@tauri-apps/plugin-clipboard-manager")
    await writeText(md)
    toast.success("Copied the chat as Markdown")
  } catch (e) {
    toast.error(`Couldn't copy the chat: ${e}`)
  }
}

/** How an open chat sends: its agent, and the permission chosen for it (else the agent's default). */
function sendOpts(chatId: string, images: Image[]): { agent: AgentKind; permission: PermissionMode; images: Image[] } | null {
  const meta: ChatMeta | undefined = useChats.getState().metas.find((m) => m.id === chatId)
  if (!meta) return null
  const settings = useApp.getState().settings
  const provider = settings?.providers[meta.agent]
  const permission = useChats.getState().permission[chatId] ?? meta.permission ?? provider?.permissionMode ?? "ask"
  return { agent: meta.agent, permission, images: images.map(({ mime, data }) => ({ mime, data })) }
}

/** Chats whose snapshot is loading, and the events (with their numbers) that came meanwhile. */
const loading = new Map<string, [number, ChatEvent][]>()

/** Batches events per animation frame so streaming text doesn't re-render per token. */
let queue: [string, ChatEvent, number | undefined][] = []
/** Per chat, the number of the last event its snapshot had: an event queued before the
 *  snapshot came back, and already in it, isn't applied twice. */
const snapshotSeq = new Map<string, number>()
let scheduled = false
function apply(chatId: string, event: ChatEvent, seq?: number) {
  notify(event, useChats.getState().metas.find((m) => m.id === chatId))
  if (event.type === "permissionChanged") useChats.setState((s) => ({ permission: { ...s.permission, [chatId]: event.mode } }))
  const held = loading.get(chatId)
  if (held) {
    if (seq !== undefined) held.push([seq, event])
    return
  }
  queue.push([chatId, event, seq])
  if (scheduled) return
  scheduled = true
  // Once a frame while it's shown; while hidden there are no frames (a minimized window would
  // hold every event, and the queued messages waiting on a turn's end), so a timer instead.
  // A frame that never comes (the window was hidden after this was scheduled) is covered too:
  // whichever runs first applies the batch.
  if (!document.hidden) requestAnimationFrame(flushEvents)
  setTimeout(flushEvents, 250)
}

function flushEvents() {
  if (!scheduled) return
  scheduled = false
  const batch = queue
  queue = []
  useChats.setState((s) => {
    const transcripts = { ...s.transcripts }
    // A chat that isn't open here (a phone drives it) loads whole when it's opened.
    for (const [id, e, seq] of batch) {
      if (!transcripts[id] || (seq !== undefined && seq <= (snapshotSeq.get(id) ?? 0))) continue
      transcripts[id] = reduce(transcripts[id], e)
    }
    return { transcripts }
  })
  if (batch.some(([, e]) => e.type === "turnFinished" || e.type === "started")) useChats.getState().refresh()
  // A turn that ended by itself sends the next queued message; one that was stopped holds them.
  const ended = new Set(batch.flatMap(([id, e]) => (e.type === "turnFinished" && !e.interrupted ? [id] : [])))
  const s = useChats.getState()
  for (const id of ended) if (s.queued[id]?.length && !s.transcripts[id]?.running) void s.sendQueued(id)
}
