// The details panel: context use, tasks, files changed (with undo), MCP servers
// and the goal — what the TUI shows in its status bar, task box and /commands.
import { Minimize2, Target, Undo2 } from "lucide-react"
import { useState, type ReactNode } from "react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Progress } from "@/components/ui/progress"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Separator } from "@/components/ui/separator"
import { useClient } from "@/lib/client"
import { fmtCost, fmtMs, fmtTokens } from "@/lib/format"
import { cn } from "@/lib/utils"
import { useTick } from "./hooks"
import { TodoList } from "./Todos"

function Section({ title, aside, children }: { title: string; aside?: ReactNode; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2.5">
      <h3 className="flex items-center justify-between text-[11px] font-semibold tracking-wider text-muted-foreground uppercase">
        <span>{title}</span>
        {aside}
      </h3>
      {children}
    </section>
  )
}

const Empty = ({ children }: { children: ReactNode }) => <p className="text-xs text-faint">{children}</p>

export function Inspector() {
  const c = useClient()
  const s = c.state!
  const [goal, setGoal] = useState("")
  const window = s.model.contextWindow
  const used = s.session.contextTokens
  const pct = window ? Math.min(999, Math.round((used / window) * 100)) : 0
  const meter = pct >= 85 ? "text-red" : pct >= 60 ? "text-warn" : "text-green"
  const meterBar = pct >= 85 ? "*:bg-red" : pct >= 60 ? "*:bg-warn" : "*:bg-green"
  const todos = s.session.todos
  const done = todos.filter((t) => t.status === "completed").length
  const files = [...new Set(s.session.checkpoints.flatMap((cp) => cp.files))]
  const g = s.session.goal
  useTick(1000, !!g?.running)

  return (
    <aside aria-label="Session details" className="hidden w-[300px] shrink-0 border-l border-sidebar-border bg-rail lg:block">
      <ScrollArea className="h-full">
        <div className="flex flex-col gap-5 p-4">
          <Section title="Context" aside={<span className={cn("font-mono", meter)}>{pct}%</span>}>
            <Progress value={Math.min(100, Math.max(pct ? 2 : 0, pct))} className={cn("h-1.5 bg-muted", meterBar)} aria-label="Context used" />
            <div className="flex justify-between gap-3 font-mono text-xs text-muted-foreground">
              <span>{fmtTokens(used)} / {fmtTokens(window)}</span>
              <span className={cn("truncate", !s.model.ready && "text-warn")} title={s.model.providerLabel}>{s.model.providerLabel}</span>
            </div>
            {s.session.cost || s.cacheHit !== null || s.session.compactions ? (
              <div className="flex flex-wrap gap-1.5">
                {s.session.cost ? <Badge variant="secondary" className="font-mono">{fmtCost(s.session.cost)}</Badge> : null}
                {s.cacheHit !== null ? <Badge variant="secondary" className="font-mono">{s.cacheHit}% cache</Badge> : null}
                {s.session.compactions ? <Badge variant="secondary">compacted {s.session.compactions}×</Badge> : null}
              </div>
            ) : null}
            <Button variant="outline" size="sm" className="self-start bg-transparent" disabled={!!s.busy} onClick={() => c.submit("/compact")}>
              <Minimize2 />
              Compact now
            </Button>
          </Section>

          <Separator />

          <Section title={`Tasks${todos.length ? ` ${done}/${todos.length}` : ""}`}>
            {todos.length ? <TodoList /> : <Empty>oah lists its tasks here as it works.</Empty>}
          </Section>

          <Separator />

          <Section title="Changed files" aside={files.length ? <span className="font-mono">{files.length}</span> : null}>
            {files.length ? (
              <ul className="flex flex-col gap-1">
                {files.slice(0, 30).map((f) => (
                  <li key={f} className="truncate font-mono text-xs [direction:rtl] text-left" title={f}>{f}</li>
                ))}
              </ul>
            ) : (
              <Empty>No changes yet.</Empty>
            )}
            {files.length > 30 ? <Empty>and {files.length - 30} more</Empty> : null}
            <Button variant="outline" size="sm" className="self-start bg-transparent" disabled={!s.session.checkpoints.length || !!s.busy} onClick={() => c.submit("/undo")}>
              <Undo2 />
              Undo to checkpoint
              {s.session.checkpoints.length ? <Badge variant="secondary" className="ml-1 h-4 px-1.5">{s.session.checkpoints.length}</Badge> : null}
            </Button>
          </Section>

          <Separator />

          <Section title="MCP" aside={<Button variant="link" size="sm" className="h-auto p-0 text-xs normal-case" onClick={() => c.submit("/mcp")}>Manage</Button>}>
            {s.mcp.length ? (
              <ul className="flex flex-col gap-2">
                {s.mcp.map((m) => (
                  <li key={m.name} className="flex items-center gap-2 text-[13px]" title={`${m.status} · ${m.source}`}>
                    <span className={cn("size-1.5 shrink-0 rounded-full", m.state === "ready" ? "bg-green" : m.state === "needs-auth" ? "bg-warn" : m.state === "failed" ? "bg-red" : "animate-pulse bg-faint")} />
                    <span className="flex-1 truncate">{m.name}</span>
                    {m.state === "needs-auth" ? (
                      <Button variant="link" size="sm" className="h-auto p-0 text-xs" onClick={() => c.send({ op: "mcpSignIn", name: m.name })}>Sign in</Button>
                    ) : (
                      <span className="text-xs text-muted-foreground">{m.state === "ready" ? `${m.tools} tools` : m.state === "failed" ? "failed" : "connecting"}</span>
                    )}
                  </li>
                ))}
              </ul>
            ) : (
              <Empty>No MCP servers. Ask oah to add one, or /import.</Empty>
            )}
            {s.shells ? <Badge variant="outline" className="self-start border-warn/40 text-warn">{s.shells} background shell{s.shells === 1 ? "" : "s"}</Badge> : null}
          </Section>

          <Separator />

          <Section title="Goal" aside={g && !g.done ? <Button variant="link" size="sm" className="h-auto p-0 text-xs normal-case" onClick={() => c.submit("/goal clear")}>Clear</Button> : null}>
            {g && !g.done ? (
              <div className="flex flex-col gap-1.5 rounded-lg border border-warn/30 bg-card p-3 text-[13px]">
                <span className="select-text">{g.text}</span>
                <span className="font-mono text-xs text-warn">
                  turn {g.turns} · {fmtMs(c.goalElapsed())} {g.running ? "working" : "so far"}
                </span>
              </div>
            ) : (
              <form
                className="flex flex-col gap-1.5"
                onSubmit={(e) => {
                  e.preventDefault()
                  if (!goal.trim()) return
                  c.submit(`/goal ${goal.trim()}`)
                  setGoal("")
                }}
              >
                <div className="relative">
                  <Target className="absolute top-2 left-2.5 size-4 text-muted-foreground" />
                  <Input aria-label="Goal" placeholder="Keep working until…" value={goal} onChange={(e) => setGoal(e.target.value)} className="h-8 pl-8 text-xs" />
                </div>
                <span className="text-xs text-faint">{g?.done ? "Last goal completed. " : ""}oah works until it's done.</span>
              </form>
            )}
          </Section>
        </div>
      </ScrollArea>
    </aside>
  )
}
