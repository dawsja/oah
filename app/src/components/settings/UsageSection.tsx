import { RefreshCw } from "lucide-react"
import { AgentGlyph } from "@/components/icons"
import { Button } from "@/components/ui/button"
import { AGENT_LABEL, type AgentKind } from "@/lib/types"
import { useApp } from "@/store/app"
import { Group, Section } from "./layout"

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
      <div className="h-2 bg-muted">
        <div className={`h-full ${tone}`} style={{ width: `${pct}%` }} />
      </div>
      {resets ? <div className="mt-1 text-[11px] text-faint">Resets {resets}</div> : null}
    </div>
  )
}

const LIMITS: Record<AgentKind, string[]> = {
  claude: ["Current session (5 hours)", "This week, all models", "This week, Opus"],
  codex: ["5 hour window", "Weekly window"],
}

export function UsageSection() {
  const projects = useApp((s) => s.settings!.projects)
  return (
    <Section
      title="Usage"
      description="Plan limits from each provider and what oah's chats have used."
      actions={
        <Button variant="outline" size="sm">
          <RefreshCw /> Refresh
        </Button>
      }
    >
      {(["claude", "codex"] as const).map((a) => (
        <Group
          key={a}
          title={
            <span className="flex items-center gap-1.5">
              <AgentGlyph agent={a} className="size-3" /> {AGENT_LABEL[a]}
            </span>
          }
          aside={<span className="text-[11px] text-faint">Not loaded yet</span>}
        >
          {LIMITS[a].map((l) => (
            <LimitBar key={l} label={l} used={null} />
          ))}
        </Group>
      ))}
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
          projects.map((p) => (
            <div key={p.id} className="grid grid-cols-[1fr_repeat(3,7rem)] px-4 py-2 font-mono text-[12px]">
              <span className="truncate font-sans">{p.name}</span>
              <span className="text-right text-muted-foreground">0</span>
              <span className="text-right text-muted-foreground">0</span>
              <span className="text-right text-muted-foreground">$0.00</span>
            </div>
          ))
        )}
      </Group>
    </Section>
  )
}
