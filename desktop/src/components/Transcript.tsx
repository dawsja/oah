// The conversation, answer-first like the TUI: your message, oah's thinking and
// tool calls folded into a steps card (edits keep their diffs), then the answer.
import { ChevronRight } from "lucide-react"
import { memo, useState } from "react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { useClient } from "@/lib/client"
import { blocksOf, fmtMs, groupTurns, stepsSummary, type Step, type Turn } from "@/lib/format"
import type { Entry } from "@/lib/protocol"
import { cn } from "@/lib/utils"
import { useTick } from "./hooks"
import { Mark, SPINNER } from "./icons"
import { Markdown } from "./Markdown"

type ToolEntry = Extract<Entry, { kind: "tool" }>

export function DiffLines({ lines, max, className }: { lines: string[]; max?: number; className?: string }) {
  if (!lines.length) return null
  const shown = max ? lines.slice(0, max) : lines
  return (
    <div className={cn("diff max-h-[420px] overflow-auto rounded-md border border-[#1d1d20] bg-background/40", className)}>
      {shown.map((l, i) => (
        <div key={i} className={l.startsWith("+") ? "a" : l.startsWith("-") ? "d" : l.startsWith("…") ? "e" : undefined}>
          {l.replace(/\t/g, "    ") || " "}
        </div>
      ))}
      {max && lines.length > max ? <div className="e">… {lines.length - max} more lines</div> : null}
    </div>
  )
}

function UserText({ text }: { text: string }) {
  return (
    <div className="max-w-[85%] self-end rounded-2xl rounded-br-sm border bg-user px-4 py-2.5 whitespace-pre-wrap select-text [overflow-wrap:anywhere]">
      {text.split(/(\[Image #\d+\])/).map((part, k) =>
        /^\[Image #\d+\]$/.test(part) ? (
          <Badge key={k} variant="outline" className="mx-0.5 border-primary/40 text-primary">{part}</Badge>
        ) : (
          part
        ),
      )}
    </div>
  )
}

const statusIcon = (e: ToolEntry, tick: number) => (e.status === "running" ? SPINNER[tick % SPINNER.length] : e.status === "ok" ? "✓" : e.status === "denied" ? "⊘" : "✗")
const statusColor = (e: ToolEntry) => (e.status === "ok" ? "text-green" : e.status === "running" ? "text-primary" : "text-red")

function StepRow({ e, tick }: { e: Entry; tick: number }) {
  const [open, setOpen] = useState(false)
  if (e.kind === "reasoning") {
    const secs = e.ms !== undefined ? fmtMs(e.ms) : null
    return (
      <Collapsible open={open} onOpenChange={setOpen}>
        <CollapsibleTrigger className="flex items-center gap-2.5 text-[13px] text-muted-foreground hover:text-foreground">
          <span className="w-3.5 text-center font-mono">{e.ms === undefined ? SPINNER[tick % SPINNER.length] : "∴"}</span>
          {e.ms === undefined ? "Thinking…" : `Thought${secs && secs !== "0s" ? ` for ${secs}` : ""}`}
        </CollapsibleTrigger>
        {e.text.trim() ? (
          <CollapsibleContent>
            <p className="my-1 ml-6 border-l-2 border-border pl-3 text-[13px] whitespace-pre-wrap text-muted-foreground select-text">{e.text.trim()}</p>
          </CollapsibleContent>
        ) : null}
      </Collapsible>
    )
  }
  if (e.kind !== "tool") return null
  const isEdit = /^(Edit|Write|Patch)\b/.test(e.title)
  const hasDetail = !isEdit && e.detail.length > 0
  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <div className="flex min-w-0 items-baseline gap-2.5 text-[13px]">
        <span className={cn("w-3.5 shrink-0 text-center font-mono", statusColor(e))}>{statusIcon(e, tick)}</span>
        <span className="font-mono text-xs [overflow-wrap:anywhere] select-text">{e.title}</span>
        {e.summary ? <span className="text-xs text-muted-foreground [overflow-wrap:anywhere]">{e.summary}</span> : null}
        {e.ms !== undefined && e.ms >= 1000 ? <span className="font-mono text-xs text-faint">{fmtMs(e.ms)}</span> : null}
        {hasDetail ? (
          <CollapsibleTrigger asChild>
            <Button variant="ghost" size="sm" className="ml-auto h-6 px-2 text-xs text-muted-foreground">{open ? "Hide" : "Output"}</Button>
          </CollapsibleTrigger>
        ) : null}
      </div>
      {hasDetail ? (
        <CollapsibleContent>
          <DiffLines lines={e.detail} max={400} className="mt-1.5 mb-1 ml-6" />
        </CollapsibleContent>
      ) : null}
    </Collapsible>
  )
}

/** One card for a run of thinking + tool calls. */
function Steps({ items, running }: { items: Step[]; running: boolean }) {
  const [open, setOpen] = useState(false)
  const tick = useTick(90, running || open)
  const entries = items.map((x) => x.e)
  const { tools, verbs, ms, failures, current, edits } = stepsSummary(entries)
  // Pure thinking in progress: the activity line already says so.
  if (running && !tools.length && !open) return null
  const lastFail = failures.at(-1)
  return (
    <Collapsible open={open} onOpenChange={setOpen} className="overflow-hidden rounded-lg border bg-card">
      <CollapsibleTrigger className="flex w-full min-w-0 items-center gap-2.5 px-3.5 py-2.5 text-left text-[13px] hover:bg-accent/50">
        {running ? <span className="w-3 font-mono text-primary">{SPINNER[tick % SPINNER.length]}</span> : <ChevronRight className={cn("size-3.5 shrink-0 text-muted-foreground transition-transform", open && "rotate-90")} />}
        <span className="shrink-0 font-semibold">{tools.length ? `${tools.length} step${tools.length === 1 ? "" : "s"}` : running ? "Thinking" : "Thought"}</span>
        {verbs.length ? <span className="truncate text-muted-foreground">{verbs.join(" · ")}</span> : null}
        {failures.length ? <Badge variant="outline" className="shrink-0 border-orange/40 text-orange">{failures.length} failed</Badge> : null}
        {current ? <span className="shimmer min-w-0 truncate">{current.title}</span> : null}
        {ms >= 1000 ? <span className="ml-auto shrink-0 font-mono text-xs text-muted-foreground">{fmtMs(ms)}</span> : null}
      </CollapsibleTrigger>
      <CollapsibleContent>
        <div className="flex flex-col gap-1.5 border-t px-3.5 pt-2 pb-2.5">
          {entries.map((e, k) => (
            <StepRow key={k} e={e} tick={tick} />
          ))}
        </div>
      </CollapsibleContent>
      {!open && !running && lastFail ? (
        <p className="px-3.5 pb-2.5 text-[13px] text-red [overflow-wrap:anywhere]">
          ✗ {lastFail.title}
          {lastFail.summary ? ` — ${lastFail.summary}` : ""}
        </p>
      ) : null}
      {edits.length ? (
        <div className="flex flex-col gap-2.5 px-3.5 pb-3">
          {edits.map((x, k) => {
            const add = x.detail.filter((l) => l.startsWith("+")).length
            const del = x.detail.filter((l) => l.startsWith("-")).length
            return (
              <div key={k}>
                <div className="mb-1 flex items-center gap-2 font-mono text-xs text-muted-foreground">
                  <span className="flex-1 truncate">{x.title}</span>
                  {add ? <span className="text-green">+{add}</span> : null}
                  {del ? <span className="text-red">−{del}</span> : null}
                </div>
                <DiffLines lines={x.detail} max={200} />
              </div>
            )
          })}
        </div>
      ) : null}
    </Collapsible>
  )
}

function TurnView({ turn, live, liveText }: { turn: Turn; live: boolean; liveText: number | null }) {
  const blocks = blocksOf(turn)
  return (
    <div className="flex flex-col gap-3.5">
      {turn.user ? <UserText text={turn.user.text} /> : null}
      {blocks.length ? (
        <div className="flex gap-3.5">
          <div className="mt-0.5 grid size-7 shrink-0 place-items-center rounded-lg border bg-popover">
            <Mark size={15} />
          </div>
          <div className="flex min-w-0 flex-1 flex-col gap-3.5">
            {blocks.map((b, k) =>
              b.kind === "steps" ? (
                <Steps key={`s${b.key}`} items={b.items} running={live && k === blocks.length - 1} />
              ) : b.e.kind === "assistant" ? (
                <Markdown key={b.i} text={b.e.text} className={liveText === b.i ? "streaming" : undefined} />
              ) : b.e.kind === "error" ? (
                <p key={b.i} className="text-[13px] text-red select-text [overflow-wrap:anywhere]">✗ {b.e.text}</p>
              ) : null,
            )}
          </div>
        </div>
      ) : null}
    </div>
  )
}

/** A finished turn only re-renders when one of its entries changed. */
const FinishedTurn = memo(
  ({ turn }: { turn: Turn; sig: string }) => <TurnView turn={turn} live={false} liveText={null} />,
  (a, b) => a.sig === b.sig,
)

function Note({ e }: { e: Entry }) {
  if (e.kind === "error")
    return (
      <div className="flex gap-2.5 text-[13px]">
        <span className="text-red">✗</span>
        <span className="text-red select-text [overflow-wrap:anywhere]">{e.text}</span>
      </div>
    )
  if (e.kind !== "info") return null
  return (
    <div className="flex gap-2.5 text-[13px]">
      <span className="shrink-0 text-cyan">※</span>
      <Markdown text={e.text} className="dim min-w-0" />
    </div>
  )
}

export function Transcript() {
  const c = useClient()
  const busy = !!c.state?.busy
  const groups = groupTurns(c.transcript)
  return (
    <>
      {groups.map((g, k) =>
        g.kind === "note" ? (
          <Note key={`n${g.i}`} e={g.entry} />
        ) : busy && k === groups.length - 1 ? (
          <TurnView key={`t${g.userIndex ?? g.steps[0]?.i}`} turn={g} live liveText={c.state?.liveText ?? null} />
        ) : (
          <FinishedTurn key={`t${g.userIndex ?? g.steps[0]?.i}`} turn={g} sig={entryVersion(g)} />
        ),
      )}
    </>
  )
}

/** A turn's signature from its entries' identities: the client replaces an entry
 * object whenever the engine reports a change to it. */
const tokens = new WeakMap<object, number>()
let tokenSeq = 0
function entryVersion(t: Turn): string {
  const tok = (o: object) => tokens.get(o) ?? (tokens.set(o, ++tokenSeq), tokenSeq)
  return `${t.user ? tok(t.user) : 0}:${t.steps.map((s) => tok(s.e)).join(".")}`
}
