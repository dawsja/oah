// Plan limits for each agent's sign-in on this computer, shared by the title bar's usage popover
// and Settings → Usage. Fetched when first wanted and again after five minutes.

import { create } from "zustand"
import { ipc } from "@/lib/ipc"
import { LIMIT_AGENTS, type AgentKind, type LimitWindow, type Limits } from "@/lib/types"

type UsageStore = {
  limits: Partial<Record<AgentKind, Limits>>
  loading: boolean
  /** Unix ms of the last fetch. */
  at: number | null
  load(force?: boolean): Promise<void>
}

const STALE = 5 * 60_000

export const useUsage = create<UsageStore>((set, get) => ({
  limits: {},
  loading: false,
  at: null,
  async load(force = false) {
    const { loading, at } = get()
    if (loading || (!force && at && Date.now() - at < STALE)) return
    set({ loading: true })
    await Promise.all(
      LIMIT_AGENTS.map((a) =>
        ipc.providerLimits(a).then(
          (l) => set((s) => ({ limits: { ...s.limits, [a]: l } })),
          (e) => set((s) => ({ limits: { ...s.limits, [a]: { windows: [], error: String(e), extra: [] } } })),
        ),
      ),
    )
    set({ loading: false, at: Date.now() })
  },
}))

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
