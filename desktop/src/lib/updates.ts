// Background updates for the desktop app (the engine inside it updates with it).
// The app checks https://dl.oah.dev/desktop/latest.json, where each build finds
// its own signed update under `{os}-{arch}-{installer}` (tauri-plugin-updater).
//
// AppImage installs update in place silently — the new version starts next
// launch, or now with "Restart to finish". Windows (NSIS) and
// deb/rpm installs download silently, then install when you choose "Restart to
// update" (the Windows installer must close the app; deb/rpm ask for a password).
import type { Update } from "@tauri-apps/plugin-updater"

export type UpdateState =
  | { kind: "off" } // development builds and the browser
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "current"; checkedAt: number }
  | { kind: "downloading"; version: string; got: number; total: number | null }
  | { kind: "ready"; version: string; notes: string | null; installed: boolean }
  | { kind: "installing"; version: string }
  | { kind: "error"; message: string }

const EVERY = 4 * 60 * 60 * 1000
/** Installs the updater can replace in place without closing the app. */
const SILENT = new Set(["appimage"])

export class Updater {
  state: UpdateState = { kind: "off" }
  private update: Update | null = null
  /** The update's bytes are downloaded (ready for install()). */
  private fetched = false
  private lastCheck = 0
  private timer: ReturnType<typeof setInterval> | null = null
  private bundle = ""

  constructor(
    private changed: () => void,
    /** Stop the engine (it saves the session) before the app is replaced. */
    private stopEngine: () => Promise<void>,
    private auto: () => boolean,
  ) {}

  private set(s: UpdateState) {
    this.state = s
    this.changed()
  }

  /** Start checking in the background (only in an installed app). */
  start(bundle: string | null) {
    if (!bundle || !("__TAURI_INTERNALS__" in window)) return
    this.bundle = bundle
    this.set({ kind: "idle" })
    setTimeout(() => void this.check(), 15_000)
    this.timer = setInterval(() => void this.check(), EVERY)
    window.addEventListener("focus", () => Date.now() - this.lastCheck > EVERY && void this.check())
  }

  get enabled() {
    return this.state.kind !== "off"
  }

  /** Look for a newer version; with automatic updates on, fetch (and where possible install) it. */
  async check(manual = false) {
    const k = this.state.kind
    if (k === "off" || k === "checking" || k === "downloading" || k === "installing" || k === "ready") return
    this.lastCheck = Date.now()
    this.set({ kind: "checking" })
    try {
      const { check } = await import("@tauri-apps/plugin-updater")
      const u = await check({ timeout: 30_000 })
      if (!u) return this.set({ kind: "current", checkedAt: Date.now() })
      this.update = u
      if (this.auto() || manual) await this.fetch()
      else this.set({ kind: "ready", version: u.version, notes: u.body ?? null, installed: false })
    } catch (e: any) {
      this.set({ kind: "error", message: String(e?.message ?? e) })
    }
  }

  /** Download the update (and install it right away where that's silent). */
  private async fetch() {
    const u = this.update!
    let got = 0
    let total: number | null = null
    this.set({ kind: "downloading", version: u.version, got, total })
    await u.download((ev) => {
      if (ev.event === "Started") total = ev.data.contentLength ?? null
      else if (ev.event === "Progress") got += ev.data.chunkLength
      this.set({ kind: "downloading", version: u.version, got, total })
    })
    this.fetched = true
    if (SILENT.has(this.bundle)) {
      await u.install()
      this.set({ kind: "ready", version: u.version, notes: u.body ?? null, installed: true })
    } else this.set({ kind: "ready", version: u.version, notes: u.body ?? null, installed: false })
  }

  /** Apply the update and start the new version. */
  async restart() {
    const u = this.update
    if (this.state.kind !== "ready" || !u) return
    const { version, installed } = this.state
    this.set({ kind: "installing", version })
    try {
      await this.stopEngine()
      if (!installed) {
        // Automatic updates off: it hasn't been downloaded yet.
        if (!this.fetched) await u.download()
        await u.install() // Windows: the installer takes over and the app exits here
      }
      const { relaunch } = await import("@tauri-apps/plugin-process")
      await relaunch()
    } catch (e: any) {
      this.set({ kind: "error", message: String(e?.message ?? e) })
    }
  }

  stop() {
    if (this.timer) clearInterval(this.timer)
  }
}
