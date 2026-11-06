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

export interface Started {
  cwd: string
  cwdShort: string
  engine: string
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
}
