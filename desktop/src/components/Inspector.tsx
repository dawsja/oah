// The details panel: context use, tasks (while there are any) and files changed
// with undo. MCP servers and goals live in /mcp and /goal (the goal timer is in
// the title bar).
import { Minimize2, Undo2 } from "lucide-react"
import type { ReactNode } from "react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Progress } from "@/components/ui/progress"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Separator } from "@/components/ui/separator"
import { useClient } from "@/lib/client"
import { fmtCost, fmtTokens } from "@/lib/format"
import { cn } from "@/lib/utils"
import { TodoList } from "./Todos"
import { Tip } from "./Tip"

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
  const window = s.model.contextWindow
  const used = s.session.contextTokens
  const pct = window ? Math.min(999, Math.round((used / window) * 100)) : 0
  const meter = pct >= 85 ? "text-red" : pct >= 60 ? "text-warn" : "text-green"
  const meterBar = pct >= 85 ? "*:bg-red" : pct >= 60 ? "*:bg-warn" : "*:bg-green"
  const todos = s.session.todos
  const done = todos.filter((t) => t.status === "completed").length
  const files = [...new Set(s.session.checkpoints.flatMap((cp) => cp.files))]

  return (
    <aside aria-label="Session details" className="hidden w-[300px] shrink-0 border-l border-sidebar-border bg-rail lg:block">
      <ScrollArea className="h-full">
        <div className="flex flex-col gap-5 p-4">
          <Section title="Context" aside={<span className={cn("font-mono", meter)}>{pct}%</span>}>
            <Progress value={Math.min(100, Math.max(pct ? 2 : 0, pct))} className={cn("h-1.5 bg-muted", meterBar)} aria-label="Context used" />
            <div className="flex justify-between gap-3 font-mono text-xs text-muted-foreground">
              <span>{fmtTokens(used)} / {fmtTokens(window)}</span>
              <Tip content={s.model.providerLabel}><span className={cn("truncate", !s.model.ready && "text-warn")}>{s.model.providerLabel}</span></Tip>
            </div>
            {s.session.cost || s.cacheHit !== null || s.session.compactions || s.shells ? (
              <div className="flex flex-wrap gap-1.5">
                {s.session.cost ? <Badge variant="secondary" className="font-mono">{fmtCost(s.session.cost)}</Badge> : null}
                {s.cacheHit !== null ? <Badge variant="secondary" className="font-mono">{s.cacheHit}% cache</Badge> : null}
                {s.session.compactions ? <Badge variant="secondary">compacted {s.session.compactions}×</Badge> : null}
                {s.shells ? <Badge variant="outline" className="border-warn/40 text-warn">{s.shells} background shell{s.shells === 1 ? "" : "s"}</Badge> : null}
              </div>
            ) : null}
            <Button variant="outline" size="sm" className="self-start bg-transparent" disabled={!!s.busy} onClick={() => c.submit("/compact")}>
              <Minimize2 />
              Compact now
            </Button>
          </Section>

          {todos.length ? (
            <>
              <Separator />
              <Section title={`Tasks ${done}/${todos.length}`}>
                <TodoList />
              </Section>
            </>
          ) : null}

          <Separator />

          <Section title="Changed files" aside={files.length ? <span className="font-mono">{files.length}</span> : null}>
            {files.length ? (
              <ul className="flex flex-col gap-1">
                {files.slice(0, 30).map((f) => (
                  <Tip key={f} content={f} side="left"><li className="truncate font-mono text-xs [direction:rtl] text-left">{f}</li></Tip>
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

        </div>
      </ScrollArea>
    </aside>
  )
}
