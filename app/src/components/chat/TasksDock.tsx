import { useEffect, useState } from "react"
import { Bot, Check, ChevronDown, ChevronRight, CircleDashed, CircleSlash, Eye, LoaderCircle, Sparkles, SquareTerminal, X } from "lucide-react"
import { formatDuration, type BackgroundTask } from "@/lib/chat"
import { AGENT_LABEL, type AgentKind } from "@/lib/types"
import { cn } from "@/lib/utils"
import { Tip } from "@/components/Tip"

const KIND_ICON = { command: SquareTerminal, agent: Bot, watch: Eye, other: Sparkles } as const

/** The current time, ticking every second while `on`. */
function useNow(on: boolean): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!on) return
    const t = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(t)
  }, [on])
  return now
}

/** "2 running · 1 done"; tasks the agent doesn't report on are only "started". */
function summary(tasks: BackgroundTask[]): string {
  const running = tasks.filter((t) => t.status === "running" && t.tracked).length
  const started = tasks.filter((t) => t.status === "running" && !t.tracked).length
  const failed = tasks.filter((t) => t.status === "failed").length
  const done = tasks.length - running - started - failed
  return [running && `${running} running`, started && `${started} started`, done && `${done} done`, failed && `${failed} failed`].filter(Boolean).join(" · ")
}

/** Work the agent runs beside the chat (commands left running, subagents, watchers): what's
 *  running and for how long, what ended since you last wrote, and a way to stop each. */
export function TasksDock({ agent, tasks, onStop }: { agent: AgentKind; tasks: BackgroundTask[]; onStop?: (taskId: string) => void }) {
  const [open, setOpen] = useState(false)
  // Ended tasks put away, by id: they show again only if new ones end.
  const [hidden, setHidden] = useState<string[]>([])
  const running = tasks.filter((t) => t.status === "running")
  const now = useNow(running.length > 0)
  const shown = tasks.filter((t) => t.status === "running" || !hidden.includes(t.id))
  if (!shown.length) return null
  const latest = running[running.length - 1] ?? shown[shown.length - 1]
  const failed = shown.some((t) => t.status === "failed")
  return (
    <div className="overflow-hidden rounded-2xl border bg-card text-[12.5px]" role="region" aria-label="Background work">
      <div className="flex h-8 items-center hover:bg-accent">
        <button className="flex h-full min-w-0 flex-1 items-center gap-2 pl-4 text-left" aria-expanded={open} onClick={() => setOpen(!open)}>
          {open ? <ChevronDown className="size-3.5 shrink-0 text-faint" /> : <ChevronRight className="size-3.5 shrink-0 text-faint" />}
          {running.some((t) => t.tracked) ? (
            <LoaderCircle className="size-3.5 shrink-0 animate-spin text-muted-foreground" />
          ) : running.length ? (
            <CircleDashed className="size-3.5 shrink-0 text-muted-foreground" />
          ) : failed ? (
            <X className="size-3.5 shrink-0 text-destructive" />
          ) : (
            <Check className="size-3.5 shrink-0 text-green" />
          )}
          <span className="shrink-0 font-medium">Background</span>
          <span className="shrink-0 font-mono text-[11px] text-faint">{summary(shown)}</span>
          {!open ? <span className="truncate text-muted-foreground">{latest.title}</span> : null}
        </button>
        {!running.length ? (
          <Tip content="Put away">
            <button
              className="rounded-md mr-2 grid size-6 shrink-0 place-items-center text-faint hover:bg-background hover:text-foreground"
              aria-label="Put away finished background work"
              onClick={() => setHidden(tasks.map((t) => t.id))}
            >
              <X className="size-3.5" />
            </button>
          </Tip>
        ) : null}
      </div>
      {open ? (
        <ul className="max-h-60 overflow-y-auto border-t py-1">
          {shown.map((t) => (
            <TaskRow key={t.id} task={t} agent={agent} now={now} onStop={onStop && (() => onStop(t.id))} />
          ))}
        </ul>
      ) : null}
    </div>
  )
}

function TaskRow({ task: t, agent, now, onStop }: { task: BackgroundTask; agent: AgentKind; now: number; onStop?: () => void }) {
  const Icon = KIND_ICON[t.kind] ?? Sparkles
  const live = t.status === "running"
  const elapsed = formatDuration((t.endedAt ?? now) - t.startedAt)
  const when = !live ? `${t.status === "completed" ? "done" : t.status} after ${elapsed}` : t.tracked ? elapsed : `started ${elapsed} ago`
  const detail = live && !t.tracked ? `${AGENT_LABEL[agent]} doesn't say when this ends; ask it to check.` : t.detail
  return (
    <li className="group/task flex items-start gap-2.5 px-4 py-1.5">
      <Icon className={cn("mt-0.5 size-3.5 shrink-0", live ? "text-muted-foreground" : "text-faint")} />
      <div className="min-w-0 flex-1">
        <Tip content={t.title.length > 60 ? t.title : null}>
          <div className={cn("truncate", t.kind === "command" && "font-mono text-[12px]", !live && "text-muted-foreground")}>{t.title}</div>
        </Tip>
        {detail ? <div className="line-clamp-2 text-[11.5px] break-words text-faint">{detail}</div> : null}
      </div>
      <span className="mt-px flex shrink-0 items-center gap-1.5 font-mono text-[11px] text-faint">
        {when}
        {live ? (
          t.tracked ? <LoaderCircle className="size-3 animate-spin" /> : null
        ) : t.status === "completed" ? (
          <Check className="size-3 text-green" />
        ) : t.status === "failed" ? (
          <X className="size-3 text-destructive" />
        ) : (
          <CircleSlash className="size-3" />
        )}
      </span>
      {live && t.stoppable && onStop ? (
        <Tip content="Stop">
          <button className="rounded-md -my-0.5 grid size-5 shrink-0 place-items-center text-faint hover:bg-background hover:text-foreground" aria-label={`Stop ${t.title}`} onClick={onStop}>
            <X className="size-3" />
          </button>
        </Tip>
      ) : null}
    </li>
  )
}
