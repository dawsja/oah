import { useEffect } from "react"
import { RefreshCw } from "lucide-react"
import { AgentGlyph } from "@/components/icons"
import { Button } from "@/components/ui/button"
import { formatTokens } from "@/lib/chat"
import { LIMIT_AGENTS, LIMIT_LABEL } from "@/lib/types"
import { useChats } from "@/store/chats"
import { LOCAL_USAGE, usageLocationLabel, useUsageAt } from "@/store/usage"
import { useLocationAwake } from "@/store/wsl"
import { useApp, useProject } from "@/store/app"
import { Group, Section } from "./layout"

function resets(ms: number | null): string | undefined {
  if (!ms) return undefined
  const d = new Date(ms)
  const sameDay = d.toDateString() === new Date().toDateString()
  return sameDay ? `at ${d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })}` : d.toLocaleDateString(undefined, { weekday: "short", hour: "numeric", minute: "2-digit" })
}

/** One plan limit: a square bar, how much is used, and when it resets. */
export function LimitBar({ label, used, resets }: { label: string; used: number | null; resets?: string }) {
  const pct = used === null ? 0 : Math.min(100, Math.max(0, used))
  const tone = pct >= 90 ? "bg-red" : pct >= 70 ? "bg-warn" : "bg-primary"
  return (
    <div className="px-4 py-3">
      <div className="mb-1.5 flex items-baseline text-[12.5px]">
        <span className="font-medium">{label}</span>
        <span className="ml-auto font-mono text-[11.5px] text-muted-foreground">{used === null ? "—" : `${Math.round(pct)}% used`}</span>
      </div>
      <div className="flex h-2 gap-px">
        {Array.from({ length: 20 }, (_, i) => (
          <span key={i} className={`flex-1 ${i < Math.round(pct / 5) ? tone : "bg-muted"}`} />
        ))}
      </div>
      {resets ? <div className="mt-1 text-[11px] text-faint">Resets {resets}</div> : null}
    </div>
  )
}

export function UsageSection() {
  const projects = useApp((s) => s.settings!.projects)
  const metas = useChats((s) => s.metas)
  const location = useProject()?.location ?? LOCAL_USAGE
  const awake = useLocationAwake(location)
  const { limits, loading, load: fetchLimits } = useUsageAt(location)
  const load = () => void fetchLimits(true)
  useEffect(() => {
    void fetchLimits(true)
  }, [fetchLimits, awake])

  return (
    <Section
      title="Usage"
      description={`Plan limits for ${usageLocationLabel(location)}. OpenCode Go covers the Go subscription only.`}
      actions={
        <Button variant="outline" size="sm" onClick={load} disabled={loading}>
          <RefreshCw className={loading ? "animate-spin" : undefined} /> Refresh
        </Button>
      }
    >
      {LIMIT_AGENTS.map((a) => {
        const l = limits[a]
        return (
          <Group
            key={a}
            title={
              <span className="flex items-center gap-1.5">
                <AgentGlyph agent={a} className="size-3" /> {LIMIT_LABEL[a]}
              </span>
            }
            aside={l === undefined ? <span className="text-[11px] text-faint">Loading…</span> : null}
          >
            {l?.error ? <div className="px-4 py-3 text-[12px] text-muted-foreground">{l.error}</div> : null}
            {l && !l.error && l.windows.length === 0 ? <div className="px-4 py-3 text-[12px] text-faint">No plan limits reported.</div> : null}
            {l === undefined ? <LimitBar label="…" used={null} /> : l?.windows.map((w) => <LimitBar key={w.label} label={w.label} used={w.usedPercent} resets={resets(w.resetsAt)} />)}
            {l && !l.error
              ? l.extra.map((x) => (
                  <div key={x.label} className="border-t px-4 py-3 text-[12.5px]">
                    <div className="flex items-baseline">
                      <span className="font-medium">{x.label}</span>
                      <span className="ml-auto font-mono text-[11.5px] text-muted-foreground">{x.value ?? "Unavailable"}</span>
                    </div>
                    {x.note ? <div className="mt-0.5 text-[11px] text-faint">{x.note}</div> : null}
                  </div>
                ))
              : null}
          </Group>
        )
      })}
      <Group title="By project">
        <div className="grid grid-cols-[1fr_repeat(3,7rem)] border-b bg-muted/50 px-4 py-2 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
          <span>Project</span>
          <span className="text-right">Chats</span>
          <span className="text-right">Tokens</span>
          <span className="text-right">Cost</span>
        </div>
        {projects.length === 0 ? (
          <div className="px-4 py-3 text-[12px] text-faint">No projects yet.</div>
        ) : (
          projects.map((p) => {
            const mine = metas.filter((m) => m.projectId === p.id)
            const tokens = mine.reduce((n, m) => n + m.tokens, 0)
            const cost = mine.reduce((n, m) => n + m.costUsd, 0)
            return (
              <div key={p.id} className="grid grid-cols-[1fr_repeat(3,7rem)] px-4 py-2 font-mono text-[12px]">
                <span className="truncate font-sans">{p.name}</span>
                <span className="text-right text-muted-foreground">{mine.length}</span>
                <span className="text-right text-muted-foreground">{formatTokens(tokens)}</span>
                <span className="text-right text-muted-foreground">${cost.toFixed(2)}</span>
              </div>
            )
          })
        )}
      </Group>
      <p className="-mt-2 text-[11.5px] text-faint">Cost is what Claude Code reports per turn; Codex doesn't report cost.</p>
    </Section>
  )
}
