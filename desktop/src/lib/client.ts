// The UI's mirror of the engine: the latest state snapshot, the transcript
// (patched from diffs), saved sessions and connection status. React reads it
// through useSyncExternalStore.
import { useSyncExternalStore } from "react"
import { PROTOCOL, type AppInfo, type ClientOp, type EngineExit, type Entry, type Prefs, type ServerEvent, type SessionMeta, type Snapshot, type Started } from "./protocol"
import { transport, type Transport } from "./transport"
import { Updater } from "./updates"

type Hello = Extract<ServerEvent, { t: "hello" }>

export type Status =
  | { kind: "starting" }
  | { kind: "ready" }
  | { kind: "exited"; code: number | null; stderr: string }
  | { kind: "error"; message: string }

export type View = "chat" | "settings"

export class Client {
  version = 0
  private listeners = new Set<() => void>()

  t!: Transport
  status: Status = { kind: "starting" }
  hello: Hello | null = null
  state: Snapshot | null = null
  /** When `state` arrived: the goal clock keeps ticking from there. */
  stateAt = 0
  transcript: Entry[] = []
  started: Started | null = null
  sessions: SessionMeta[] = []
  info: AppInfo | null = null
  prefs: Prefs = {}
  view: View = "chat"
  /** Desktop app updates (background download and install). */
  updater = new Updater(
    () => this.emit(),
    () => this.t.stop(),
    () => this.prefs.autoUpdate !== false,
  )
  history: string[] = []
  /** Bumped on each engine start so stale events are dropped. */
  private gen = 0

  /** The composer registers these (the engine replaces or inserts text). */
  onInput: ((text: string) => void) | null = null
  onInsert: ((text: string) => void) | null = null

  subscribe = (fn: () => void) => {
    this.listeners.add(fn)
    return () => void this.listeners.delete(fn)
  }
  getVersion = () => this.version
  emit() {
    this.version++
    for (const l of this.listeners) l()
  }

  private initialized = false

  async init() {
    if (this.initialized) return // StrictMode runs effects twice in development
    this.initialized = true
    this.t = await transport
    try {
      ;[this.info, this.prefs] = await Promise.all([this.t.info(), this.t.prefs()])
    } catch (e: any) {
      this.status = { kind: "error", message: e?.message ?? String(e) }
      return this.emit()
    }
    this.t.onDrop((paths) => {
      for (const path of paths) if (/\.(png|jpe?g|gif|webp)$/i.test(path)) this.send({ op: "attach", path })
    })
    await this.start({})
    void this.refreshSessions()
    this.updater.start(this.info?.bundle ?? null)
    if (this.t.platform === "web") this.devParams()
    // Sessions also change from the CLI: refresh on focus and now and then.
    window.addEventListener("focus", () => void this.refreshSessions())
    setInterval(() => void this.refreshSessions(), 15_000)
  }

  /** Browser development only: `?session=<id>&view=settings&input=/m&run=/permissions`
   * put the UI in a given state (for screenshots and quick checks). */
  private devParams() {
    const q = new URLSearchParams(location.search)
    if (![...q.keys()].length) return

    const whenReady = (fn: () => void) => {
      const tick = () => (this.status.kind === "ready" && this.state ? fn() : setTimeout(tick, 50))
      tick()
    }
    whenReady(async () => {
      const id = q.get("session")
      if (id) {
        await this.refreshSessions()
        const s = this.sessions.find((x) => x.id === id)
        if (s) this.openSession(s)
      }
      setTimeout(() => {
        if (q.get("view") === "settings") this.setView("settings")
        const input = q.get("input")
        if (input) this.onInput?.(input)
        const run = q.get("run")
        if (run) this.submit(run)
      }, 300)
    })
  }

  async start(opts: { cwd?: string; resume?: string }) {
    const gen = ++this.gen
    this.status = { kind: "starting" }
    this.hello = null
    this.state = null
    this.transcript = []
    this.emit()
    try {
      this.started = await this.t.start(opts, (line) => gen === this.gen && this.receive(line))
      this.prefs.lastCwd = this.started.cwd
    } catch (e: any) {
      if (gen === this.gen) this.status = { kind: "error", message: e?.message ?? String(e) }
    }
    this.emit()
  }

  private receive(line: string) {
    let e: ServerEvent | EngineExit
    try {
      e = JSON.parse(line)
    } catch {
      return
    }
    switch (e.t) {
      case "hello":
        if (e.protocol > PROTOCOL) {
          this.status = { kind: "error", message: `This engine (oah ${e.version}) speaks a newer protocol than this app. Update the oah desktop app.` }
          break
        }
        this.hello = e
        this.history = e.history.slice()
        this.status = { kind: "ready" }
        break
      case "state": {
        const prev = this.state
        this.state = e.s
        this.stateAt = Date.now()
        // A session was saved or switched: the sidebar should know.
        if (prev && (prev.session.id !== e.s.session.id || (prev.busy && !e.s.busy))) void this.refreshSessions()
        break
      }
      case "transcript": {
        const next = e.reset ? [] : this.transcript.slice(0, e.length)
        for (const [i, entry] of e.changes) next[i] = entry
        next.length = e.length
        this.transcript = next
        break
      }
      case "input":
        this.onInput?.(e.text)
        return
      case "insert":
        this.onInsert?.(e.text)
        return
      case "copy":
        void this.t.copy(e.text)
        return
      case "exit":
        // /exit closes the window, like quitting the CLI. (A replaced engine's
        // exit never gets here: its events are dropped by the generation check.)
        this.t.win.close()
        return
      case "error":
        console.warn("engine:", e.message)
        return
      case "engine-exit":
        if (this.status.kind !== "error") this.status = { kind: "exited", code: e.code, stderr: e.stderr }
        break
    }
    this.emit()
  }

  send(op: ClientOp) {
    void this.t.send(op).catch((err) => console.warn(err))
  }

  submit(text: string, now = false) {
    const trimmed = text.replace(/\s+$/, "")
    if (trimmed && this.state?.inputMode === "prompt" && !this.state.question && this.history.at(-1) !== trimmed) this.history.push(trimmed)
    this.send({ op: "submit", text, now })
  }

  async refreshSessions() {
    try {
      this.sessions = await this.t.sessions()
      this.emit()
    } catch {}
  }

  /** Open a saved session: in this engine when it's the same folder, else restart there. */
  openSession(s: SessionMeta) {
    this.view = "chat"
    if (this.state?.session.id === s.id) return this.emit()
    if (this.started && s.cwd === this.started.cwd && this.status.kind === "ready") {
      this.send({ op: "resume", id: s.id })
      return this.emit()
    }
    void this.start({ cwd: s.cwd, resume: s.id })
  }

  /** A new conversation, here or in another folder. */
  newSession(cwd?: string) {
    this.view = "chat"
    if (!cwd || cwd === this.started?.cwd) {
      if (this.status.kind === "ready") this.send({ op: "new" })
      else void this.start({ cwd: this.started?.cwd })
      return this.emit()
    }
    void this.start({ cwd })
  }

  async openFolder() {
    const dir = await this.t.pickFolder()
    if (dir) this.newSession(dir)
  }

  restart() {
    void this.start({ cwd: this.started?.cwd, resume: this.state?.session.id })
  }

  /** Time spent on the goal so far, ticking while it runs (Store.goalElapsed). */
  goalElapsed(): number {
    const g = this.state?.session.goal
    if (!g) return 0
    return g.elapsed + (g.running ? Date.now() - this.stateAt : 0)
  }

  setView(v: View) {
    this.view = v
    this.emit()
  }

  setPref<K extends keyof Prefs>(k: K, v: Prefs[K]) {
    this.prefs[k] = v
    void this.t.setPrefs({ [k]: v ?? null })
    this.emit()
  }
}

export const client = new Client()

export function useClient(): Client {
  useSyncExternalStore(client.subscribe, client.getVersion)
  return client
}
