// The task list, drawn like the TUI's (src/ui/App.tsx TodoPanel): done ☑ in
// green with the text struck through, the current task spinning while oah works
// (◐ when paused), the rest ☐.
import { Badge } from "@/components/ui/badge"
import { useClient } from "@/lib/client"
import { fmtMs } from "@/lib/format"
import { cn } from "@/lib/utils"
import { useTick } from "./hooks"
import { SPINNER } from "./icons"

export function TodoList() {
  const s = useClient().state!
  const todos = s.session.todos
  const working = !!s.busy && todos.some((x) => x.status === "in_progress")
  const tick = useTick(80, working)
  return (
    <ul className="flex flex-col gap-1.5">
      {todos.map((t, i) => (
        <li key={i} className={cn("flex gap-2.5 text-[13px] leading-snug", t.status === "completed" ? "text-faint" : t.status === "in_progress" ? "font-semibold text-warn" : "text-muted-foreground")}>
          <span className={cn("w-3.5 shrink-0 text-center font-mono", t.status === "completed" && "text-green")}>
            {t.status === "completed" ? "☑" : t.status === "in_progress" ? (working ? SPINNER[tick % SPINNER.length] : "◐") : "☐"}
          </span>
          <span className={cn(t.status === "completed" && "line-through")}>{t.text}</span>
        </li>
      ))}
    </ul>
  )
}

/** `◎ goal · 12m04s` — the goal's working time, ticking while it runs. */
export function GoalClock({ className }: { className?: string }) {
  const c = useClient()
  const g = c.state?.session.goal
  useTick(1000, !!g?.running)
  if (!g || g.done) return null
  return (
    <Badge variant="outline" className={cn("border-warn/40 font-mono text-warn", className)} title={g.text}>
      ◎ goal · {fmtMs(c.goalElapsed())}
    </Badge>
  )
}
