import { useEffect, useLayoutEffect, useRef, useState } from "react"
import { ArrowDown, Check, ChevronRight, CircleAlert, Copy, Info, ListChecks } from "lucide-react"
import { blocks, formatDuration, formatTokens, type Decision, type Item, type Transcript as T } from "@/lib/chat"
import { copyText } from "@/lib/clipboard"
import { cn } from "@/lib/utils"
import { Markdown } from "./Markdown"
import { MessageImages } from "./ChatImages"
import { WorkGroup } from "./Work"
import { useTouch } from "./touch"

/** Blocks mounted at once; older ones come in by this many. */
const WINDOW = 150

/** The conversation: user prompts on the right, the agent's work and answers on the left. */
export function Transcript({ transcript, className }: { transcript: T; className?: string }) {
  const scroller = useRef<HTMLDivElement>(null)
  const touch = useTouch()
  const [stuck, setStuck] = useState(true)
  const all = blocks(transcript.items)
  // Long chats mount only their latest blocks; older ones load on request.
  const [limit, setLimit] = useState(WINDOW)
  const hidden = Math.max(0, all.length - limit)
  const list = hidden ? all.slice(hidden) : all
  const lastWork = list.findLastIndex((b) => b.kind === "work")

  // Follow new content while the view is at the bottom.
  const stuckRef = useRef(stuck)
  stuckRef.current = stuck
  useLayoutEffect(() => {
    const el = scroller.current
    if (el && stuck) el.scrollTop = el.scrollHeight
  })
  useEffect(() => {
    const el = scroller.current
    if (!el) return
    const onScroll = () => setStuck(el.scrollHeight - el.scrollTop - el.clientHeight < 48)
    el.addEventListener("scroll", onScroll, { passive: true })
    // Sizes change without a render here too: a card docking under the chat (or the phone's
    // keyboard) shrinks the view, and blocks that skip layout off-screen grow as they come in.
    // While at the bottom, stay there.
    const pin = new ResizeObserver(() => {
      if (stuckRef.current) el.scrollTop = el.scrollHeight
    })
    pin.observe(el)
    if (el.firstElementChild) pin.observe(el.firstElementChild)
    return () => {
      el.removeEventListener("scroll", onScroll)
      pin.disconnect()
    }
  }, [])

  return (
    <div className={cn("relative min-h-0", className)}>
      <div ref={scroller} className="h-full overflow-y-auto">
        <div className={cn("mx-auto flex w-full max-w-[46rem] flex-col gap-4", touch ? "px-4 pt-4 pb-6" : "px-6 pt-6 pb-10")}>
          {hidden ? (
            <button
              className="self-center rounded-md border px-3 py-1 text-[12px] text-muted-foreground hover:bg-accent hover:text-foreground"
              onClick={() => {
                const el = scroller.current
                const from = el ? el.scrollHeight - el.scrollTop : 0
                setLimit((n) => n + WINDOW)
                // Keep the reader where they were as older blocks appear above.
                requestAnimationFrame(() => el && (el.scrollTop = el.scrollHeight - from))
              }}
            >
              Show {Math.min(hidden, WINDOW)} earlier
            </button>
          ) : null}
          {list.map((b, i) => (
            // Off-screen blocks skip layout and paint until they scroll into view.
            <div key={b.kind === "work" ? b.id : `${b.item.kind}-${b.item.id}`} className="[contain-intrinsic-size:auto_120px] [content-visibility:auto]">
              {b.kind === "work" ? <WorkGroup items={b.items} live={transcript.running && i === lastWork && i >= list.length - 2} /> : <Row item={b.item} />}
            </div>
          ))}
          {transcript.running && !transcript.approvals.length && !transcript.questions.length && list[list.length - 1]?.kind !== "work" ? <span className="shimmer self-start text-[12.5px]">Working…</span> : null}
        </div>
      </div>
      {!stuck ? (
        <button
          className={cn("rounded-full absolute bottom-3 left-1/2 flex -translate-x-1/2 items-center gap-1.5 border bg-popover px-3 text-muted-foreground shadow-lg hover:text-foreground", touch ? "h-10 px-4 text-[13px]" : "h-7 text-[12px]")}
          onClick={() => {
            const el = scroller.current
            if (el) el.scrollTop = el.scrollHeight
            setStuck(true)
          }}
        >
          <ArrowDown className="size-3.5" /> Jump to latest
        </button>
      ) : null}
    </div>
  )
}

function Row({ item }: { item: Item }) {
  switch (item.kind) {
    case "user":
      return (
        <div className="flex justify-end">
          <div className="rounded-xl max-w-[85%] border bg-user px-3.5 py-2.5 text-[length:var(--oah-chat-size,14px)] leading-relaxed whitespace-pre-wrap select-text">
            {item.text}
            <MessageImages files={item.files} count={item.images.length} />
          </div>
        </div>
      )
    case "assistant":
      return <AssistantText text={item.text} />
    case "approval":
      if (item.decision && item.request.kind === "plan" && item.request.detail) return <PlanCard text={item.request.detail} decision={item.decision} />
      return item.decision ? (
        <div className="flex items-center gap-2 text-[12px] text-faint">
          <span className={cn("size-1.5", item.decision === "deny" ? "bg-red" : "bg-green")} />
          {item.decision === "deny" ? "Denied" : item.decision === "allowAlways" ? "Allowed for this chat" : "Allowed"}: {item.request.title}
        </div>
      ) : null
    case "turnEnd":
      return (
        <div className="flex items-center gap-2 font-mono text-[11px] text-faint">
          {item.interrupted ? <span className="text-warn">Interrupted</span> : null}
          {item.durationMs ? <span>{formatDuration(item.durationMs)}</span> : null}
          {item.usage ? <span>· {formatTokens(item.usage.inputTokens + item.usage.outputTokens + item.usage.cacheReadTokens + item.usage.cacheWriteTokens)} tokens</span> : null}
          {item.costUsd ? <span>· ${item.costUsd.toFixed(2)}</span> : null}
        </div>
      )
    case "notice":
      return (
        <div className={cn("rounded-lg flex items-start gap-2 border px-3 py-2 text-[12.5px]", item.level === "error" ? "border-red/40 bg-del/40 text-red" : item.level === "warn" ? "border-warn/40 text-warn" : "text-muted-foreground")}>
          {item.level === "info" ? <Info className="mt-0.5 size-3.5 shrink-0" /> : <CircleAlert className="mt-0.5 size-3.5 shrink-0" />}
          <span className="select-text">{item.text}</span>
        </div>
      )
    default:
      return null
  }
}

/** A plan the agent asked to start on, kept in the chat once answered. */
function PlanCard({ text, decision }: { text: string; decision: Decision }) {
  const [open, setOpen] = useState(decision !== "deny")
  return (
    <div className="rounded-lg border bg-card">
      <button className="flex min-h-10 w-full items-center gap-2 px-3 py-2 text-left text-[12.5px]" onClick={() => setOpen(!open)}>
        <ListChecks className="size-3.5 text-muted-foreground" />
        <span className="font-medium">Plan</span>
        <span className="text-faint">{decision === "deny" ? "· kept planning" : "· approved"}</span>
        <ChevronRight className={cn("ml-auto size-3.5 text-faint transition-transform", open && "rotate-90")} />
      </button>
      {open ? <Markdown text={text} className="border-t px-3.5 py-2.5" /> : null}
    </div>
  )
}

function AssistantText({ text }: { text: string }) {
  const [copied, setCopied] = useState(false)
  const touch = useTouch()
  return (
    <div className="group/msg relative">
      <Markdown text={text} />
      <button
        className={cn("rounded-md mt-1 flex items-center gap-1 text-faint hover:text-foreground", touch ? "h-10 pr-3 text-[12.5px]" : "h-6 text-[11.5px] opacity-0 group-hover/msg:opacity-100")}
        onClick={() => {
          copyText(text)
          setCopied(true)
          setTimeout(() => setCopied(false), 1200)
        }}
      >
        {copied ? <Check className="size-3" /> : <Copy className="size-3" />} {copied ? "Copied" : "Copy"}
      </button>
    </div>
  )
}
