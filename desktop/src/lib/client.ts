// The UI's mirror of the engine: the latest state snapshot, the transcript
// (patched from diffs), saved sessions and connection status. React reads it
// through useSyncExternalStore.
import { useSyncExternalStore } from "react"
import { toast } from "sonner"
import { LOCAL, PROTOCOL, sameTarget, targetKey, targetLabel, type AppInfo, type ClientOp, type EngineExit, type Entry, type Prefs, type RemoteCheck, type ServerEvent, type SessionMeta, type Snapshot, type Started, type Target, type Targets } from "./protocol"
import { transport, type Transport } from "./transport"
import { Updater } from "./updates"

type Hello = Extract<ServerEvent, { t: "hello" }>

export type Status =
  | { kind: "starting" }
  | { kind: "ready" }
  | { kind: "exited"; code: number | null; stderr: string }
  | { kind: "error"; message: string }

export type View = "chat" | "settings"

/** Connecting to a remote target: checking for oah, or it's missing/old/unreachable. */
export type RemoteState = { target: Target; checking?: boolean; installing?: boolean; check?: RemoteCheck; error?: string }

/** `~/…` for paths under `home`. */
export function shortWithHome(p: string, home: string): string {
  const h = home.replace(/\/+$/, "")
  if (!h) return p
  if (p === h) return "~"
  return p.startsWith(h + "/") ? "~" + p.slice(h.length) : p
}

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
  /** Where the engine runs. */
  target: Target = LOCAL
  /** Set while connecting to a remote target, or when it needs attention. */
  remote: RemoteState | null = null
  /** WSL distros and ~/.ssh/config hosts, for the location menu. */
  targets: Targets = { wsl: [], sshConfig: [] }
  /** The folder browser for remote targets. */
  browsing = false
  /** $HOME on the remote target (from its check), for ~/ paths. */
  private remoteHome = ""
  /** Desktop app updates (background download and install). */
  updater = new Updater(
    () => this.emit(),
    () => this.stopEngine(),
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
    this.t.onDrop((paths) => void this.attachFiles(paths.filter((p) => /\.(png|jpe?g|gif|webp)$/i.test(p))))
    void this.loadTargets()
    const t = this.prefs.target
    if (t && t.kind !== "local") await this.switchTarget(t, { startup: true })
    else await this.start({})
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

  async loadTargets() {
    try {
      this.targets = await this.t.remote.targets()
      this.emit()
    } catch {}
  }

  /** Every SSH host to offer: added by hand, then from ~/.ssh/config. */
  sshHosts(): string[] {
    return [...new Set([...(this.prefs.sshHosts ?? []), ...this.targets.sshConfig])]
  }

  /** Work on this computer, a WSL distro or an SSH host. Remote targets are
   * checked first: oah must be installed there (and new enough). */
  async switchTarget(t: Target, opts: { startup?: boolean } = {}) {
    this.view = "chat"
    if (t.kind === "ssh" && !this.prefs.sshHosts?.includes(t.host) && !this.targets.sshConfig.includes(t.host)) this.setPref("sshHosts", [...(this.prefs.sshHosts ?? []), t.host])
    if (t.kind === "local") {
      if (this.target.kind === "local" && this.started && !opts.startup) {
        this.remote = null
        return this.emit()
      }
      this.remote = null
      this.target = LOCAL
      this.setPref("target", undefined)
      this.sessions = []
      await this.start({ cwd: this.prefs.lastCwd })
      return void this.refreshSessions()
    }
    if (!opts.startup && sameTarget(t, this.target) && !this.remote) return this.emit()
    this.remote = { target: t, checking: true }
    this.emit()
    let check: RemoteCheck
    try {
      check = await this.t.remote.check(t)
    } catch (e: any) {
      this.remote = { target: t, error: e?.message ?? String(e) }
      return this.emit()
    }
    if (!check.ok) {
      this.remote = { target: t, check }
      return this.emit()
    }
    this.remote = null
    this.target = t
    this.remoteHome = check.home
    this.setPref("target", t)
    this.sessions = []
    await this.start({ cwd: this.prefs.lastCwds?.[targetKey(t)] ?? "" })
    void this.refreshSessions()
  }

  /** Install (or update) oah on the remote target that needs it, then connect. */
  async installRemote() {
    const r = this.remote
    if (!r) return
    this.remote = { ...r, installing: true, error: undefined }
    this.emit()
    try {
      const check = await this.t.remote.install(r.target)
      if (!check.ok) {
        this.remote = { target: r.target, check, error: `oah ${check.version ?? "?"} was installed, but this app needs ${check.minVersion} or newer.` }
        return this.emit()
      }
      toast.success(`oah ${check.version} is installed on ${targetLabel(r.target)}`)
      await this.switchTarget(r.target, { startup: true })
    } catch (e: any) {
      this.remote = { ...r, installing: false, error: e?.message ?? String(e) }
      this.emit()
    }
  }

  /** Give up on a remote target: back to the previous one (or this computer). */
  cancelRemote() {
    const r = this.remote
    this.remote = null
    if (r && this.status.kind === "ready" && !sameTarget(r.target, this.target)) return this.emit()
    void this.switchTarget(LOCAL)
  }

  async start(opts: { cwd?: string; resume?: string }) {
    const gen = ++this.gen
    this.status = { kind: "starting" }
    this.hello = null
    this.state = null
    this.transcript = []
    this.emit()
    try {
      this.started = await this.t.start({ ...opts, target: this.target }, (line) => gen === this.gen && this.receive(line))
      if (this.target.kind === "local") this.prefs.lastCwd = this.started.cwd
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
        // A remote engine knows its real folder and home; remember the folder.
        if (this.started && this.target.kind !== "local") {
          this.started = { ...this.started, cwd: e.cwd, cwdShort: shortWithHome(e.cwd, this.remoteHome) }
          this.setPref("lastCwds", { ...this.prefs.lastCwds, [targetKey(this.target)]: e.cwd })
        }
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
    const t = this.target
    try {
      const list = await this.t.sessions(t)
      if (sameTarget(t, this.target)) this.sessions = list
      this.emit()
    } catch {}
  }

  /** Delete a saved session (the open one is replaced by a new conversation first). */
  async deleteSession(s: SessionMeta) {
    if (this.state?.session.id === s.id) this.send({ op: "new" })
    try {
      await this.t.deleteSession(this.target, s.id)
      this.sessions = this.sessions.filter((x) => x.id !== s.id)
      toast.success(`Deleted "${s.title || "untitled session"}"`)
    } catch (e: any) {
      toast.error(`Couldn't delete the session: ${e?.message ?? e}`)
    }
    this.emit()
    void this.refreshSessions()
  }

  private folderKey = (cwd: string) => `${targetKey(this.target)}\n${cwd}`
  isFolderClosed = (cwd: string) => (this.prefs.closedFolders ?? []).includes(this.folderKey(cwd))
  private reopenFolder(cwd: string) {
    if (this.isFolderClosed(cwd)) this.setPref("closedFolders", (this.prefs.closedFolders ?? []).filter((k) => k !== this.folderKey(cwd)))
  }

  /** Hide a folder's sessions from the sidebar (they stay on disk; opening the folder again brings them back). */
  closeFolder(cwd: string) {
    this.setPref("closedFolders", [...new Set([...(this.prefs.closedFolders ?? []), this.folderKey(cwd)])])
    // Closing the open folder: start over in the home folder.
    if (this.started?.cwd === cwd) void this.start({ cwd: this.target.kind === "local" ? this.info?.homeDir : "" })
  }

  /** Images from files: by path for a local engine, as bytes for a remote one. */
  async attachFiles(paths: string[]) {
    for (const path of paths) {
      if (this.target.kind === "local") this.send({ op: "attach", path })
      else {
        try {
          const img = await this.t.readImage(path)
          this.send({ op: "image", ...img })
        } catch (e: any) {
          toast.error(e?.message ?? String(e))
        }
      }
    }
  }

  /** An image from the clipboard or a drop, as a browser File. */
  async attachBlob(file: File) {
    const buf = new Uint8Array(await file.arrayBuffer())
    let bin = ""
    for (let i = 0; i < buf.length; i += 0x8000) bin += String.fromCharCode(...buf.subarray(i, i + 0x8000))
    this.send({ op: "image", name: file.name || "pasted image", mediaType: file.type, data: btoa(bin) })
  }

  /** Open a saved session: in this engine when it's the same folder, else restart there. */
  openSession(s: SessionMeta) {
    this.view = "chat"
    this.reopenFolder(s.cwd)
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
    if (cwd) this.reopenFolder(cwd)
    if (!cwd || cwd === this.started?.cwd) {
      if (this.status.kind === "ready") this.send({ op: "new" })
      else void this.start({ cwd: this.started?.cwd })
      return this.emit()
    }
    void this.start({ cwd })
  }

  /** Pick a folder to work in: the system dialog here, the folder browser on a remote target. */
  async openFolder() {
    if (this.target.kind !== "local") {
      this.browsing = true
      return this.emit()
    }
    const dir = await this.t.pickFolder()
    if (dir) this.newSession(dir)
  }

  setBrowsing(open: boolean) {
    this.browsing = open
    this.emit()
  }

  restart() {
    void this.start({ cwd: this.started?.cwd, resume: this.state?.session.id })
  }

  /** Stop the engine for good (before an update): it saves its session and says
   * "exit", which must not close the window, so its events are dropped first. */
  stopEngine(): Promise<void> {
    this.gen++
    return this.t.stop()
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
