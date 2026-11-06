// The engine protocol, typed from the engine's own source so the desktop app and
// `oah serve` can't drift apart. Type-only: nothing from ../../src is bundled.
export type { ClientOp, ServerEvent, Snapshot } from "../../../src/serve"
export type { Entry, ToolStatus } from "../../../src/session"
export type { PickItem, PickerKind } from "../../../src/ui/store"
export type { Effort, Mode, Permission, Provider } from "../../../src/config"

/** Protocol major version this UI speaks (`PROTOCOL` in src/serve.ts). */
export const PROTOCOL = 1

/** A saved session, as the sidebar lists it (desktop/core/src/sessions.rs). */
export interface SessionMeta {
  id: string
  title: string
  cwd: string
  cwdShort: string
  created: number
  updated: number
  model: string
  mode: string
  entries: number
  hasGoal: boolean
  /** False when only the engine's summary was read (model/mode/goal unknown). */
  detailed: boolean
}

/** desktop/src-tauri/src/lib.rs `Info`. */
export interface AppInfo {
  version: string
  /** How the app was installed (appimage, deb, rpm, nsis, msi, app); null in development. */
  bundle: string | null
  os: string
  home: string
  homeShort: string
  homeDir: string
  sessionsDir: string
  oahHomeEnv: boolean
  xdgEnv: boolean
}

/** Where the engine runs (desktop/core/src/remote.rs `Target`). */
export type Target = { kind: "local" } | { kind: "wsl"; distro: string } | { kind: "ssh"; host: string }
export const LOCAL: Target = { kind: "local" }
export const targetKey = (t: Target) => (t.kind === "local" ? "local" : t.kind === "wsl" ? `wsl:${t.distro}` : `ssh:${t.host}`)
export const targetLabel = (t: Target) => (t.kind === "local" ? "This computer" : t.kind === "wsl" ? `WSL · ${t.distro}` : t.host)
export const sameTarget = (a: Target, b: Target) => targetKey(a) === targetKey(b)

/** Is oah installed on a remote target, and new enough? */
export interface RemoteCheck {
  home: string
  version: string | null
  ok: boolean
  minVersion: string
}

export interface Listing {
  path: string
  dirs: string[]
}

export interface Targets {
  wsl: string[]
  sshConfig: string[]
}

export interface ImageData {
  name: string
  mediaType: string
  data: string
}

export interface Started {
  cwd: string
  cwdShort: string
  engine: string
  target: Target
}

/** Sent by the shell when the engine process stops. */
export interface EngineExit {
  t: "engine-exit"
  code: number | null
  stderr: string
}

export type Prefs = {
  lastCwd?: string
  inspector?: boolean
  collapsedGroups?: string[]
  /** Download and install app updates in the background (default on). */
  autoUpdate?: boolean
  /** Where the engine runs (this computer, a WSL distro, an SSH host). */
  target?: Target
  /** SSH hosts added by hand (hosts in ~/.ssh/config are listed anyway). */
  sshHosts?: string[]
  /** Last folder per remote target (`targetKey`). */
  lastCwds?: Record<string, string>
  /** Folders closed in the sidebar: `${targetKey}\n${cwd}`. */
  closedFolders?: string[]
}
