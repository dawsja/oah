import { useEffect } from "react"
import { ChevronRight, Gauge, RefreshCw } from "lucide-react"
import { AgentGlyph } from "@/components/icons"
import { Tip } from "@/components/Tip"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { LIMIT_AGENTS, LIMIT_LABEL, type Limits } from "@/lib/types"
import { cn } from "@/lib/utils"
import { useApp } from "@/store/app"
import { nextReset, resetsIn, shortLabel, useUsage } from "@/store/usage"

/** The title bar's usage icon and a popover with each agent's windows. */
export function UsageButton() {
  const { limits, loading, load } = useUsage()
  useEffect(() => {
    void load()
    const t = setInterval(() => document.visibilityState === "visible" && void load(), 5 * 60_000)
    return () => clearInterval(t)
  }, [load])
  const top = Math.max(0, ...Object.values(limits).flatMap((l) => l?.windows.map((w) => w.usedPercent) ?? []))
  const tone = top >= 90 ? "text-red" : top >= 70 ? "text-warn" : "text-muted-foreground"

  return (
    <Popover onOpenChange={(o) => o && void load()}>
      <Tip content="Usage" side="bottom">
        <PopoverTrigger asChild>
          <button className="rounded-md flex h-7 items-center px-2 text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Usage">
            <Gauge className={cn("size-3.5", tone)} />
          </button>
        </PopoverTrigger>
      </Tip>
      <PopoverContent align="end" className="w-80 p-0">
        <div className="flex items-center px-3 pt-2.5 pb-1.5 text-[12.5px] font-semibold">
          Usage
          <button className="rounded-md ml-auto grid size-6 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Refresh usage" onClick={() => void load(true)}>
            <RefreshCw className={cn("size-3.5", loading && "animate-spin")} />
          </button>
        </div>
        {LIMIT_AGENTS.map((a) => (
          <AgentRow key={a} agent={a} limits={limits[a]} />
        ))}
        <button
          className="flex w-full items-center border-t px-3 py-2.5 text-left text-[12.5px] hover:bg-accent"
          onClick={() => useApp.getState().openSettings("usage")}
        >
          Usage details
          <ChevronRight className="ml-auto size-3.5 text-muted-foreground" />
        </button>
      </PopoverContent>
    </Popover>
  )
}

function AgentRow({ agent, limits }: { agent: (typeof LIMIT_AGENTS)[number]; limits: Limits | undefined }) {
  const reset = resetsIn(nextReset(limits))
  return (
    <div className="border-t px-3 py-2.5">
      <div className="flex items-center gap-2 text-[12.5px]">
        <AgentGlyph agent={agent} className="size-4" />
        <span className="font-medium">{LIMIT_LABEL[agent]}</span>
        {reset ? <span className="text-[11px] text-faint">Resets {reset}</span> : null}
      </div>
      {limits === undefined ? (
        <div className="mt-1.5 h-3 w-40 animate-pulse rounded-sm bg-muted" />
      ) : limits.error ? (
        <div className="mt-1 text-[11.5px] leading-snug text-faint">{limits.error}</div>
      ) : (
        <div className="mt-1.5 flex flex-wrap gap-x-3 gap-y-1">
          {limits.windows.map((w) => (
            <span key={w.label} className="flex items-center gap-1.5 text-[11px]" title={w.label}>
              <span className="text-faint">{shortLabel(w)}</span>
              <span className="flex h-1.5 w-10 bg-muted">
                <span className={cn("h-full", w.usedPercent >= 90 ? "bg-red" : w.usedPercent >= 70 ? "bg-warn" : "bg-primary")} style={{ width: `${Math.min(100, w.usedPercent)}%` }} />
              </span>
              <span className="font-mono text-muted-foreground">{Math.round(w.usedPercent)}%</span>
            </span>
          ))}
        </div>
      )}
    </div>
  )
}
