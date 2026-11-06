// How the UI reaches the shell: Tauri IPC in the app, or a WebSocket to
// scripts/dev-bridge.ts when the UI runs in a plain browser (`bun run web`).
import type { AppInfo, ClientOp, ImageData, Listing, Prefs, RemoteCheck, SessionMeta, Started, Target, Targets } from "./protocol"

export type Platform = "windows" | "linux" | "web"

export interface Transport {
  platform: Platform
  start(opts: { cwd?: string; resume?: string; target?: Target }, onLine: (line: string) => void): Promise<Started>
  send(op: ClientOp): Promise<void>
  sessions(target: Target): Promise<SessionMeta[]>
  deleteSession(target: Target, id: string): Promise<void>
  remote: {
    targets(): Promise<Targets>
    check(target: Target): Promise<RemoteCheck>
    install(target: Target): Promise<RemoteCheck>
    ls(target: Target, path: string): Promise<Listing>
  }
  readImage(path: string): Promise<ImageData>
  info(): Promise<AppInfo>
  prefs(): Promise<Prefs>
  setPrefs(patch: Partial<Record<keyof Prefs, unknown>>): Promise<void>
  openFolder(which: "home" | "cwd", path?: string, target?: Target): Promise<void>
  /** Stop the engine (it saves the open session). */
  stop(): Promise<void>
  openUrl(url: string): Promise<void>
  pickFolder(): Promise<string | null>
  pickImages(): Promise<string[]>
  copy(text: string): Promise<void>
  /** Image files dropped on the window. */
  onDrop(fn: (paths: string[]) => void): void
  win: { minimize(): void; toggleMaximize(): void; close(): void }
}

const isTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window

async function tauriTransport(): Promise<Transport> {
  const { invoke, Channel } = await import("@tauri-apps/api/core")
  const { getCurrentWindow } = await import("@tauri-apps/api/window")
  const { getCurrentWebview } = await import("@tauri-apps/api/webview")
  const dialog = await import("@tauri-apps/plugin-dialog")
  const clip = await import("@tauri-apps/plugin-clipboard-manager")
  const opener = await import("@tauri-apps/plugin-opener")
  const ua = navigator.userAgent
  const w = getCurrentWindow()
  return {
    platform: ua.includes("Windows") ? "windows" : "linux",
    start(opts, onLine) {
      const ch = new Channel<string>()
      ch.onmessage = onLine
      return invoke<Started>("engine_start", { cwd: opts.cwd ?? null, resume: opts.resume ?? null, target: opts.target ?? null, onEvent: ch })
    },
    send: (op) => invoke("engine_send", { op: JSON.stringify(op) }),
    sessions: (target) => invoke<SessionMeta[]>("list_sessions", { target }),
    deleteSession: (target, id) => invoke("delete_session", { target, id }),
    remote: {
      targets: () => invoke<Targets>("remote_targets"),
      check: (target) => invoke<RemoteCheck>("remote_check", { target }),
      install: (target) => invoke<RemoteCheck>("remote_install", { target }),
      ls: (target, path) => invoke<Listing>("remote_ls", { target, path }),
    },
    readImage: (path) => invoke<ImageData>("read_image", { path }),
    info: () => invoke<AppInfo>("app_info"),
    prefs: () => invoke<Prefs>("prefs_get"),
    setPrefs: (patch) => invoke("prefs_set", { patch }).then(() => {}),
    openFolder: (which, path, target) => invoke("open_folder", { which, path: path ?? null, target: target ?? null }),
    stop: () => invoke("engine_stop"),
    openUrl: (url) => opener.openUrl(url),
    async pickFolder() {
      const r = await dialog.open({ directory: true, multiple: false, title: "Open a folder in oah" })
      return typeof r === "string" ? r : null
    },
    async pickImages() {
      const r = await dialog.open({ multiple: true, title: "Attach images", filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "gif", "webp"] }] })
      return Array.isArray(r) ? r : r ? [r] : []
    },
    copy: (text) => clip.writeText(text),
    onDrop(fn) {
      void getCurrentWebview().onDragDropEvent((e) => {
        if (e.payload.type === "drop") fn(e.payload.paths)
      })
    },
    win: { minimize: () => void w.minimize(), toggleMaximize: () => void w.toggleMaximize(), close: () => void w.close() },
  }
}

/** Browser development: scripts/dev-bridge.ts runs the engine and answers these calls. */
function webTransport(): Transport {
  const url = (import.meta.env.VITE_BRIDGE as string | undefined) ?? "ws://127.0.0.1:1430"
  let ws: WebSocket | null = null
  let onLine: (l: string) => void = () => {}
  let seq = 0
  const waiting = new Map<number, { resolve: (v: any) => void; reject: (e: Error) => void }>()
  const ready = new Promise<WebSocket>((resolve, reject) => {
    const s = new WebSocket(url)
    s.onopen = () => resolve(s)
    s.onerror = () => reject(new Error(`No dev bridge at ${url} — run: bun run web:bridge`))
    s.onmessage = (m) => {
      const msg = JSON.parse(String(m.data))
      if (msg.line !== undefined) return onLine(msg.line)
      const w = waiting.get(msg.id)
      if (!w) return
      waiting.delete(msg.id)
      msg.error ? w.reject(new Error(msg.error)) : w.resolve(msg.result)
    }
    ws = s
  })
  const call = async <T,>(method: string, params: object = {}): Promise<T> => {
    const s = await ready
    const id = ++seq
    return new Promise<T>((resolve, reject) => {
      waiting.set(id, { resolve, reject })
      s.send(JSON.stringify({ id, method, params }))
    })
  }
  void ws
  return {
    platform: "web",
    start(opts, fn) {
      onLine = fn
      return call<Started>("start", opts)
    },
    send: (op) => call("send", { op }),
    sessions: (target) => call<SessionMeta[]>("sessions", { target }),
    deleteSession: (target, id) => call("deleteSession", { target, id }),
    // Remote machines need the app's shell (Rust); the browser bridge is local only.
    remote: {
      targets: async () => ({ wsl: [], sshConfig: [] }),
      check: () => Promise.reject(new Error("Remote machines need the desktop app")),
      install: () => Promise.reject(new Error("Remote machines need the desktop app")),
      ls: () => Promise.reject(new Error("Remote machines need the desktop app")),
    },
    readImage: () => Promise.reject(new Error("Not available in the browser")),
    info: () => call<AppInfo>("info"),
    prefs: () => call<Prefs>("prefs"),
    setPrefs: (patch) => call("setPrefs", { patch }),
    openFolder: async () => {},
    stop: async () => {},
    openUrl: async (u) => void window.open(u, "_blank", "noopener"),
    pickFolder: async () => window.prompt("Folder to open"),
    pickImages: async () => {
      const p = window.prompt("Image path")
      return p ? [p] : []
    },
    copy: (text) => navigator.clipboard.writeText(text),
    onDrop() {},
    win: { minimize() {}, toggleMaximize() {}, close() {} },
  }
}

export const transport: Promise<Transport> = isTauri() ? tauriTransport() : Promise.resolve(webTransport())
