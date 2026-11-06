// `oah serve`: the engine without a terminal UI, for the desktop app (and any
// other front end). It runs the same Store as the TUI — sessions, commands,
// pickers, permissions, checkpoints, MCP — and speaks newline-delimited JSON:
//
//   stdout  {"t":"hello"…} once, then {"t":"state"…} and {"t":"transcript"…}
//           whenever the store changes, plus {"t":"input"|"insert"|"copy"|"exit"}.
//   stdin   {"op":"submit","text":"…"}, {"op":"interrupt"}, {"op":"pick","value":"…"} …
//
// The protocol is versioned (PROTOCOL); front ends should refuse a newer major.
import * as auth from "./auth"
import { acceptsImages, contextWindow, displayName, effortsFor } from "./catalog"
import { COMMANDS } from "./commands"
import { homeDir, KEY_PROVIDERS, providerInfo, saveSettings, shortPath, VERSION, type Effort, type Mode, type Permission, type Provider, type Settings } from "./config"
import { imageFromFile, MAX_IMAGE_BYTES } from "./images"
import { LOCAL_PROVIDERS } from "./local"
import { Mcp } from "./mcp"
import { Memory } from "./memory"
import { saveSession, type Entry, type SessionData } from "./session"
import { Skills } from "./skills"
import { runningShells } from "./tools/shells"
import { checkForUpdate } from "./update"
import { cacheHitPercent } from "./usage"
import { mcpState, Store } from "./ui/store"

export const PROTOCOL = 1

export type ServerEvent =
  | { t: "hello"; protocol: number; version: string; home: string; cwd: string; commands: { name: string; args: string; desc: string }[]; history: string[] }
  | { t: "state"; s: Snapshot }
  | { t: "transcript"; session: string; reset: boolean; length: number; changes: [number, Entry][] }
  | { t: "input"; text: string }
  | { t: "insert"; text: string }
  | { t: "copy"; text: string }
  | { t: "exit" }
  | { t: "error"; message: string }

export type ClientOp =
  | { op: "submit"; text: string; now?: boolean }
  | { op: "input"; text: string }
  | { op: "interrupt" }
  | { op: "permission"; reply: "once" | "always" | "deny" }
  | { op: "answer"; answer: string }
  | { op: "pick"; value: string }
  | { op: "closePicker" }
  | { op: "model"; value: string }
  | { op: "effort"; value: Effort }
  | { op: "permissionLevel"; value: Permission }
  | { op: "mode"; value: Mode }
  | { op: "settings"; patch: Partial<Pick<Settings, (typeof PATCHABLE)[number]>> }
  | { op: "auth"; action: string }
  | { op: "cancelKey" }
  | { op: "new" }
  | { op: "resume"; id: string }
  | { op: "attach"; path: string }
  | { op: "pasteImage" }
  /** Image bytes from the app (paste, drop, file picker): works when the engine
   * runs on another machine and can't read this one's files or clipboard. */
  | { op: "image"; name: string; mediaType: string; data: string }
  | { op: "sendQueued" }
  | { op: "mcpSignIn"; name: string }
  | { op: "shutdown" }

/** Settings a front end may change directly (the rest go through commands). */
const PATCHABLE = ["autoCompact", "autoCompactAt", "maxCheckpoints", "allowedCommands", "safetyModel"] as const

export function snapshot(store: Store) {
  const s = store.session
  const st = store.authStatus
  const model = store.settings.model
  const p = store.picker
  return {
    cwd: store.cwd,
    cwdShort: shortPath(store.cwd),
    session: {
      id: s.id,
      title: s.title,
      cwd: s.cwd,
      created: s.created,
      updated: s.updated,
      contextTokens: s.contextTokens,
      totalTokens: s.totalTokens,
      todos: s.todos,
      // `elapsed` is the goal's working time so far; `running` while its clock ticks.
      goal: s.goal ? { ...s.goal, elapsed: store.goalElapsed(), running: store.goalRunning } : null,
      compactions: s.compactions,
      cost: s.cost ?? 0,
      usage: s.usage ?? null,
      verification: s.verification ?? null,
      checkpoints: (s.checkpoints ?? []).map((c) => ({ id: c.id, label: c.label, at: c.at, files: c.files })),
    },
    settings: store.settings,
    model: {
      id: model,
      name: model ? displayName(model) : "no model",
      provider: store.provider,
      providerLabel: auth.label(st, store.provider),
      ready: auth.has(st, store.provider),
      contextWindow: store.contextWindow(),
      acceptsImages: model ? acceptsImages(model) : false,
      efforts: model ? effortsFor(model) : [],
    },
    busy: store.busy ? { kind: store.busy.kind, started: store.busy.started } : null,
    activity: store.activity,
    toast: store.toast,
    liveText: store.liveText,
    liveReasoning: store.liveReasoning,
    picker: p ? { kind: p.kind, title: p.title, filter: p.filter, selected: p.selected, items: p.items } : null,
    permission: store.permission ? { title: store.permission.title, kind: store.permission.kind, preview: store.permission.preview } : null,
    question: store.question ? { question: store.question.question, options: store.question.options, multiSelect: store.question.multiSelect } : null,
    waitingPrompts: store.waitingPrompts,
    queue: store.queue,
    steering: store.steers.length,
    inputMode: store.inputMode,
    loginUrl: store.loginUrl ?? null,
    updateAvailable: store.updateAvailable,
    shells: runningShells().length,
    cacheHit: cacheHitPercent(s.usage) ?? null,
    auth: {
      chatgpt: st.chatgpt ?? null,
      available: auth.available(st),
      providers: [...KEY_PROVIDERS, ...LOCAL_PROVIDERS].reduce<{ id: Provider; label: string; blurb: string; billing: string; local: boolean; source: string | null }[]>(
        (out, id) => (out.push({ id, label: providerInfo[id].label, blurb: providerInfo[id].blurb, billing: providerInfo[id].billing, local: (LOCAL_PROVIDERS as readonly Provider[]).includes(id), source: st.keys[id] ?? null }), out),
        [],
      ),
    },
    mcp: store.mcp.servers.map((x) => ({ name: x.name, source: x.source, state: x.state, status: mcpState(x), remote: x.remote, signedIn: !!x.signedIn, tools: x.tools.length })),
    skills: store.skills.list.map((x) => ({ name: x.name, description: x.description, scope: x.scope })),
  }
}

export type Snapshot = ReturnType<typeof snapshot>

/** Cheap change detector for an entry (entries are mutated in place while streaming). */
function signature(e: Entry): string {
  switch (e.kind) {
    case "tool":
      return `t|${e.status}|${e.summary}|${e.ms ?? ""}|${e.detail.length}|${e.detail.at(-1)?.length ?? 0}|${e.title}`
    case "reasoning":
      return `r|${e.text.length}|${e.ms ?? ""}`
    default:
      return `${e.kind}|${e.text.length}`
  }
}

/**
 * Connects a Store to a line-oriented peer: turns store changes into events
 * (state snapshots + transcript diffs) and applies incoming ops.
 */
export class Bridge {
  private sentSession = ""
  private sentSigs: string[] = []
  private sentState = ""
  private timer: ReturnType<typeof setTimeout> | null = null
  private unsubscribe: () => void

  constructor(
    public store: Store,
    private send: (e: ServerEvent) => void,
    private onExit: () => void = () => {},
  ) {
    store.applyInput = (text) => send({ t: "input", text })
    store.insertInput = (text) => send({ t: "insert", text })
    this.unsubscribe = store.subscribe(() => this.schedule())
  }

  /** Coalesce bursts: at most one flush every ~30 ms. */
  private schedule() {
    if (this.timer) return
    this.timer = setTimeout(() => {
      this.timer = null
      this.flush()
    }, 30)
  }

  flush() {
    const store = this.store
    const t = store.session.transcript
    const reset = store.session.id !== this.sentSession || t.length < this.sentSigs.length
    if (reset) this.sentSigs = []
    const changes: [number, Entry][] = []
    for (let i = 0; i < t.length; i++) {
      const sig = signature(t[i]!)
      if (this.sentSigs[i] !== sig) {
        this.sentSigs[i] = sig
        changes.push([i, t[i]!])
      }
    }
    if (reset || changes.length) {
      this.sentSession = store.session.id
      this.send({ t: "transcript", session: store.session.id, reset, length: t.length, changes })
    }
    const state = JSON.stringify(snapshot(store))
    if (state !== this.sentState) {
      this.sentState = state
      this.send({ t: "state", s: JSON.parse(state) })
    }
    if (store.quit) this.onExit()
  }

  apply(o: ClientOp) {
    const store = this.store
    switch (o.op) {
      case "submit":
        return store.submit(String(o.text ?? ""), !!o.now)
      case "input":
        store.input = String(o.text ?? "")
        store.histIdx = null
        return
      case "interrupt":
        return void store.interrupt()
      case "permission":
        return store.answerPermission(o.reply === "always" || o.reply === "once" ? o.reply : "deny")
      case "answer":
        return store.answerQuestion(String(o.answer ?? ""))
      case "pick": {
        const p = store.picker
        const item = p?.items.find((i) => i.value === o.value)
        if (!p || !item) return
        // Store.pick() takes the highlighted visible row: make this item that row.
        p.items = [{ ...item, searchOnly: false }]
        p.filter = ""
        p.selected = 0
        return store.pick()
      }
      case "closePicker":
        store.picker = null
        return store.emit(true)
      case "model":
        return store.setModel(o.value)
      case "effort":
        return store.setEffort(o.value)
      case "permissionLevel":
        return store.setPermission(o.value)
      case "mode":
        return store.setMode(o.value === "plan" ? "plan" : "build")
      case "settings": {
        for (const k of PATCHABLE) if (o.patch && k in o.patch) (store.settings as any)[k] = (o.patch as any)[k]
        saveSettings(store.settings)
        return store.emit(true)
      }
      case "auth":
        return store.authAction(o.action)
      case "cancelKey":
        store.inputMode = "prompt"
        return store.setInput("")
      case "new":
        return store.newConversation()
      case "resume": {
        store.resume(o.id)
        // Switching sessions is routine in a sidebar: don't save a "Resumed session" note each time.
        const t = store.session.transcript
        const last = t.at(-1)
        if (last?.kind === "info" && last.text === `Resumed session ${store.session.id}`) t.pop()
        return store.emit(true)
      }
      case "attach": {
        const img = imageFromFile(o.path, store.cwd)
        if (typeof img === "string") return store.notify(img, true)
        if (!img) return store.notify(`Not an image: ${o.path}`, true)
        return store.attachImage(img)
      }
      case "pasteImage":
        return void store.pasteClipboardImage()
      case "image": {
        if (!/^image\/(png|jpeg|gif|webp)$/.test(o.mediaType)) return store.notify(`Not a supported image: ${o.name}`, true)
        const bytes = Math.floor((o.data.length * 3) / 4)
        if (bytes > MAX_IMAGE_BYTES) return store.notify(`${o.name} is ${(bytes / 1048576).toFixed(1)} MB — images must be under 5 MB`, true)
        return store.attachImage({ data: o.data, mediaType: o.mediaType, name: o.name || "image", bytes })
      }
      case "sendQueued":
        return store.sendQueuedNow()
      case "mcpSignIn":
        return store.mcpSignIn(o.name)
      case "shutdown":
        store.quit = true
        return this.onExit()
      default:
        this.send({ t: "error", message: `unknown op ${JSON.stringify((o as any)?.op)}` })
    }
  }

  close() {
    if (this.timer) clearTimeout(this.timer)
    this.timer = null
    this.unsubscribe()
    this.store.applyInput = null
    this.store.insertInput = null
  }
}

export interface ServeOptions {
  settings: Settings
  session?: SessionData
  cwd: string
  model?: string
}

/** Run the engine over stdin/stdout until the peer disconnects or sends shutdown. */
export async function runServe(o: ServeOptions): Promise<number> {
  ;(globalThis as any).AI_SDK_LOG_WARNINGS = false
  process.emitWarning = () => {}
  // stdout carries the protocol: anything a library logs goes to stderr instead.
  const out = process.stdout
  for (const k of ["log", "info", "debug", "warn"] as const) console[k] = (...a: unknown[]) => console.error(...a)
  const send = (e: ServerEvent) => void out.write(JSON.stringify(e) + "\n")

  let store: Store
  const mcp = new Mcp(o.cwd, () => store?.emit())
  // The desktop owns the clipboard: hand it the text and report success.
  store = new Store(o.settings, o.session, o.cwd, new Skills(o.cwd), new Memory(o.cwd), mcp, (text) => (send({ t: "copy", text }), true))
  if (o.model) store.settings.model = o.model

  let done: (code: number) => void
  const finished = new Promise<number>((r) => (done = r))
  let exiting = false
  const exit = async () => {
    if (exiting) return
    exiting = true
    bridge.close()
    store.shutdown()
    await Promise.race([mcp.close().catch(() => {}), new Promise((r) => setTimeout(r, 2000))])
    if (store.session.messages.length) saveSession(store.session)
    send({ t: "exit" })
    done(0)
  }
  const bridge = new Bridge(store, send, () => void exit())

  send({ t: "hello", protocol: PROTOCOL, version: VERSION, home: homeDir(), cwd: o.cwd, commands: COMMANDS.map(({ name, args, desc }) => ({ name, args, desc })), history: store.history.slice(-500) })
  bridge.flush()

  mcp.start()
  void checkForUpdate().then((v) => {
    if (!v) return
    store.updateAvailable = v
    store.notify(`oah ${v} is available — /update`)
  })
  void mcp.settled().then(() => {
    const need = mcp.servers.filter((x) => x.state === "needs-auth").map((x) => x.name)
    if (need.length) store.notify(`MCP: ${need.join(", ")} need${need.length === 1 ? "s" : ""} authentication — /mcp to sign in`)
    store.emit()
  })

  let buf = ""
  process.stdin.setEncoding("utf8")
  process.stdin.on("data", (chunk: string) => {
    buf += chunk
    for (let nl = buf.indexOf("\n"); nl >= 0; nl = buf.indexOf("\n")) {
      const line = buf.slice(0, nl).trim()
      buf = buf.slice(nl + 1)
      if (!line) continue
      let op: ClientOp
      try {
        op = JSON.parse(line)
      } catch {
        send({ t: "error", message: "invalid JSON" })
        continue
      }
      try {
        bridge.apply(op)
      } catch (e: any) {
        send({ t: "error", message: e?.message ?? String(e) })
      }
    }
  })
  // The front end went away: save and stop.
  process.stdin.on("end", () => void exit())
  process.on("SIGTERM", () => void exit())
  process.on("SIGINT", () => void exit())
  return finished
}
