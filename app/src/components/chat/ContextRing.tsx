import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { formatTokens, type ContextFill } from "@/lib/chat"
import { cn } from "@/lib/utils"

/** How full the chat's context window is: a ring by the send button; click for what it holds. */
export function ContextRing({ context }: { context: ContextFill | null }) {
  if (!context || !context.window) return null
  const pct = Math.min(100, (context.used / context.window) * 100)
  const tone = pct >= 90 ? "text-red" : pct >= 70 ? "text-warn" : "text-muted-foreground"
  const r = 6.5
  const c = 2 * Math.PI * r
  const parts = context.parts.length
    ? context.parts
    : [
        { name: "Used", tokens: context.used, kind: "used" },
        { name: "Free space", tokens: Math.max(0, context.window - context.used), kind: "free" },
      ]

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          className="rounded-full grid size-7 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground"
          aria-label={`Context: ${Math.round(pct)}% used`}
          title={`Context: ${formatTokens(context.used)} of ${formatTokens(context.window)} (${Math.round(pct)}%)`}
        >
          <svg viewBox="0 0 16 16" className={cn("size-4 -rotate-90", tone)} aria-hidden="true">
            <circle cx="8" cy="8" r={r} fill="none" stroke="currentColor" strokeOpacity="0.22" strokeWidth="2" />
            <circle cx="8" cy="8" r={r} fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeDasharray={`${(pct / 100) * c} ${c}`} />
          </svg>
        </button>
      </PopoverTrigger>
      <PopoverContent align="end" side="top" className="w-72 p-3">
        <div className="flex items-baseline">
          <span className="text-[13px] font-semibold">Context</span>
          <span className="ml-auto font-mono text-[12px] text-muted-foreground">
            {formatTokens(context.used)}/{formatTokens(context.window)}
          </span>
        </div>
        <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-muted">
          <div className={cn("h-full rounded-full", pct >= 90 ? "bg-red" : pct >= 70 ? "bg-warn" : "bg-foreground/70")} style={{ width: `${pct}%` }} />
        </div>
        <div className="mt-3 flex flex-col gap-1.5">
          {parts.map((p) => (
            <div key={p.name} className="flex items-baseline gap-2 text-[12px]" title={`${p.tokens.toLocaleString()} tokens`}>
              <span className={cn("min-w-0 flex-1 truncate", p.kind === "free" ? "text-muted-foreground" : "text-foreground")}>{p.name}</span>
              <span className="shrink-0 font-mono text-[11px] text-faint">{formatTokens(p.tokens)}</span>
              <span className="w-12 shrink-0 text-right font-mono text-[11.5px] text-muted-foreground">{percent(p.tokens, context.window)}</span>
            </div>
          ))}
        </div>
        {context.parts.some((p) => p.kind === "deferred") ? <p className="mt-2.5 text-[11px] leading-snug text-faint">Deferred tools load only when the agent uses them.</p> : null}
        {!context.parts.length ? <p className="mt-2.5 text-[11px] leading-snug text-faint">This agent reports only the total.</p> : null}
      </PopoverContent>
    </Popover>
  )
}

/** 84.4%, 1%, 0.3%, 0%: one decimal at most. */
function percent(tokens: number, window: number): string {
  return `${Math.round((tokens / window) * 1000) / 10}%`
}
