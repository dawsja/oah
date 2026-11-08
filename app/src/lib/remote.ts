// Phones controlling this desktop (app/src-tauri/src/remote.rs): on/off, the pairing code and
// the paired devices, kept current from `remote-state` events.

import { create } from "zustand"
import type { PairedDevice } from "./types"

export type RemoteAddress = { ip: string; label: string }
export type RemoteDevice = PairedDevice & { connected: boolean }
export type RemoteState = {
  enabled: boolean
  running: boolean
  port: number
  addresses: RemoteAddress[]
  devices: RemoteDevice[]
  error: string | null
  discoveryAvailable: boolean
  desktopName: string
}
export type PairingInfo = { code: string; expiresAt: number; addresses: string[] }

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core")
  return invoke<T>(cmd, args)
}

export const remote = {
  allowFirewall: () => call<void>("remote_allow_firewall"),
  state: () => call<RemoteState>("remote_state"),
  setEnabled: (enabled: boolean) => call<RemoteState>("remote_set_enabled", { enabled }),
  setPort: (port: number) => call<RemoteState>("remote_set_port", { port }),
  pairStart: () => call<PairingInfo>("remote_pair_start"),
  pairCancel: () => call<void>("remote_pair_cancel"),
  revoke: (deviceId: string) => call<RemoteState>("remote_revoke", { deviceId }),
}

type Store = {
  state: RemoteState | null
  /** The last phone that paired (the pair dialog shows it, then closes). */
  pairingOpen: boolean
  pairRequest: string | null
  paired: string | null
  /** Tries left after a wrong code, while the dialog is open. */
  failedLeft: number | null
  refresh(): Promise<void>
}

export const useRemote = create<Store>((set) => ({
  state: null,
  pairingOpen: false,
  pairRequest: null,
  paired: null,
  failedLeft: null,
  async refresh() {
    const state = await remote.state().catch(() => null)
    if (state) set({ state })
  },
}))

let listening = false
/** Loads the state once and follows `remote-state` (and pairing) events. */
export async function initRemote() {
  if (listening) return
  listening = true
  const { listen } = await import("@tauri-apps/api/event")
  await listen<{ name: string }>("remote-pair-request", (e) => {
    const s = useRemote.getState()
    if (!s.pairingOpen && !s.pairRequest) useRemote.setState({ pairRequest: e.payload.name })
  })
  await listen("remote-state", () => void useRemote.getState().refresh())
  await listen<{ name: string }>("remote-paired", (e) => useRemote.setState({ paired: e.payload.name, failedLeft: null }))
  await listen<{ left: number }>("remote-pair-failed", (e) => useRemote.setState({ failedLeft: e.payload.left }))
  await useRemote.getState().refresh()
}

/** "just now", "5 min ago", "3 h ago", or the date. */
export function seenLabel(ms: number, now = Date.now()): string {
  if (!ms) return "never connected"
  const s = Math.max(0, Math.round((now - ms) / 1000))
  if (s < 60) return "just now"
  if (s < 3600) return `${Math.round(s / 60)} min ago`
  if (s < 86400) return `${Math.round(s / 3600)} h ago`
  return new Date(ms).toLocaleDateString()
}
