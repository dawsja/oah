// Which WSL distros are running. Anything that touches a stopped distro starts it (a few
// seconds, and it keeps the WSL VM up), so oah only does that for the project you open;
// other WSL projects wait in the sidebar, marked stopped, until you open them.

import { useEffect } from "react"
import { create } from "zustand"
import { toast } from "sonner"
import { ipc } from "@/lib/ipc"
import type { Location } from "@/lib/types"

type WslStore = {
  /** Distro → running; missing until the first look. */
  running: Record<string, boolean>
  starting: Record<string, boolean>
  /** Looks again (`wsl --list --running`; doesn't start anything). */
  refresh(): Promise<void>
  /** Starts `distro` if it's stopped. */
  start(distro: string): Promise<boolean>
}

export const useWsl = create<WslStore>((set, get) => ({
  running: {},
  starting: {},
  async refresh() {
    const list = await ipc.wslDistros().catch(() => [])
    set({ running: Object.fromEntries(list.map((d) => [d.name, d.running])) })
  },
  async start(distro) {
    if (get().running[distro] || get().starting[distro]) return true
    set((s) => ({ starting: { ...s.starting, [distro]: true } }))
    try {
      await ipc.wslStart(distro)
      set((s) => ({ running: { ...s.running, [distro]: true } }))
      return true
    } catch (e) {
      toast.error(`Couldn't start ${distro}`, { description: String(e) })
      return false
    } finally {
      set((s) => ({ starting: { ...s.starting, [distro]: false } }))
    }
  },
}))

let watching = false
/** Keeps the running list fresh: at start, every minute, and when the window comes back. */
export function watchWsl() {
  if (watching) return
  watching = true
  void useWsl.getState().refresh()
  setInterval(() => document.visibilityState === "visible" && void useWsl.getState().refresh(), 60_000)
  window.addEventListener("focus", () => void useWsl.getState().refresh())
}

/** Whether a project's system can be used without starting anything: always for this computer
 *  and SSH; for WSL only once its distro is known to be running (unknown counts as asleep, or
 *  the first look would start it). */
export function useLocationAwake(location: Location | undefined): boolean {
  return useWsl((s) => location?.kind !== "wsl" || s.running[location.distro] === true)
}

/** A WSL project whose distro is known to be stopped. */
export function useLocationStopped(location: Location | undefined): boolean {
  return useWsl((s) => location?.kind === "wsl" && s.running[location.distro] === false)
}

/** The open project's distro is started on purpose (other WSL projects are left asleep). */
export function useStartOpenDistro(location: Location | undefined) {
  const distro = location?.kind === "wsl" ? location.distro : null
  const stopped = useWsl((s) => (distro ? s.running[distro] === false : false))
  const starting = useWsl((s) => (distro ? !!s.starting[distro] : false))
  useEffect(() => {
    if (distro && stopped) void useWsl.getState().start(distro)
  }, [distro, stopped])
  return { distro, starting }
}
