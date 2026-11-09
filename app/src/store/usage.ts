// Plan limits by location, shared by the title bar and Settings → Usage.

import { useCallback } from "react"
import { create } from "zustand"
import { ipc } from "@/lib/ipc"
import { LIMIT_AGENTS, type AgentKind, type LimitWindow, type Limits, type Location, type Settings } from "@/lib/types"
import { useApp } from "./app"

type UsageEntry = {
  limits: Partial<Record<AgentKind, Limits>>
  /** Per agent, unix ms of its last answer without an error. Only those are reused. */
  okAt: Partial<Record<AgentKind, number>>
  loading: boolean
  /** Unix ms of the last fetch. */
  at: number | null
}

type UsageStore = {
  entries: Record<string, UsageEntry>
  load(location: Location, force?: boolean): Promise<void>
}

export const LOCAL_USAGE: Location = { kind: "local" }
const EMPTY: UsageEntry = { limits: {}, okAt: {}, loading: false, at: null }
export function usageLocationKey(location: Location): string {
  return location.kind === "local" ? "local" : JSON.stringify([location.kind, location.kind === "wsl" ? location.distro : location.host])
}
export function usageLocationLabel(location: Location): string {
  return location.kind === "local" ? "This computer" : location.kind === "wsl" ? `WSL · ${location.distro}` : `SSH · ${location.host}`
}

const STALE = 5 * 60_000

/** An answer is reused for five minutes; a failed one (not signed in, no key, rejected,
 *  offline) is asked again next time, so signing in shows without a manual refresh. */
export const useUsage = create<UsageStore>((set, get) => ({
  entries: {},
  async load(location, force = false) {
    const key = usageLocationKey(location)
    const { loading, okAt } = get().entries[key] ?? EMPTY
    if (loading) return
    const now = Date.now()
    const due = limitAgents(useApp.getState().settings).filter((a) => force || !(now - (okAt[a] ?? -Infinity) < STALE))
    if (!due.length) return
    const update = (edit: (entry: UsageEntry) => UsageEntry) => set((s) => ({ entries: { ...s.entries, [key]: edit(s.entries[key] ?? EMPTY) } }))
    const answer = (a: AgentKind, l: Limits) =>
      update((entry) => {
        const { [a]: _, ...rest } = entry.okAt
        return { ...entry, limits: { ...entry.limits, [a]: l }, okAt: l.error ? rest : { ...rest, [a]: Date.now() } }
      })
    update((entry) => ({ ...entry, loading: true }))
    await Promise.all(
      due.map((a) =>
        ipc.providerLimits(a, location).then(
          (l) => answer(a, l),
          (e) => answer(a, { windows: [], error: String(e), extra: [] }),
        ),
      ),
    )
    update((entry) => ({ ...entry, loading: false, at: Date.now() }))
  },
}))

/** The agents whose limits are worth asking for: those on in Settings → Providers. */
export function limitAgents(settings: Settings | null): AgentKind[] {
  return LIMIT_AGENTS.filter((a) => settings?.providers[a].enabled !== false || (a === "opencode" && settings?.providers.opencode2?.enabled !== false))
}

/** The agents with limits to show at a location: on in Settings, and installed there (an
 *  uninstalled CLI's leftover sign-in isn't worth a row). */
export function shownAgents(settings: Settings | null, limits: Partial<Record<AgentKind, Limits>>): AgentKind[] {
  return limitAgents(settings).filter((a) => !limits[a]?.missing)
}

export function useUsageAt(location: Location) {
  const key = usageLocationKey(location)
  const entry = useUsage((s) => s.entries[key] ?? EMPTY)
  const fetch = useUsage((s) => s.load)
  const load = useCallback((force = false) => fetch(location, force), [fetch, location])
  return { ...entry, load }
}

/** A window's chip label: `5h`, `wk`, `30d`, or the model a weekly limit is for. */
export function shortLabel(w: LimitWindow): string {
  const l = w.label.toLowerCase()
  const model = /this week, (?!all)(.+)$/i.exec(w.label)
  if (model) return model[1]
  if (l.includes("5 hour") || l.includes("session")) return "5h"
  if (l.includes("week")) return "wk"
  if (l.includes("month")) return "30d"
  const days = /(\d+) day/.exec(l)
  if (days) return `${days[1]}d`
  const hours = /(\d+) hour/.exec(l)
  return hours ? `${hours[1]}h` : w.label
}

/** "in 2d 17h", "in 4h 47m", "in 12m". */
export function resetsIn(ms: number | null, now = Date.now()): string | null {
  if (!ms) return null
  const m = Math.max(0, Math.round((ms - now) / 60_000))
  const d = Math.floor(m / 1440)
  const h = Math.floor((m % 1440) / 60)
  if (d) return `in ${d}d ${h}h`
  if (h) return `in ${h}h ${m % 60}m`
  return `in ${m}m`
}

/** The window that resets first: what "Resets in …" on an agent's row means. */
export function nextReset(l: Limits | undefined): number | null {
  const times = (l?.windows ?? []).map((w) => w.resetsAt).filter((t): t is number => !!t)
  return times.length ? Math.min(...times) : null
}
