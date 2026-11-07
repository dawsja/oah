import { useEffect, useLayoutEffect, useRef, useState } from "react"
import { ArrowDown, Check, CircleAlert, Copy, Image as ImageIcon, Info } from "lucide-react"
import { blocks, formatDuration, formatTokens, type Item, type Transcript as T } from "@/lib/chat"
import { cn } from "@/lib/utils"
import { Markdown } from "./Markdown"
import { WorkGroup } from "./Work"

/** The conversation: user prompts on the right, the agent's work and answers on the left. */
export function Transcript({ transcript, className }: { transcript: T; className?: string }) {
  const scroller = useRef<HTMLDivElement>(null)
  const [stuck, setStuck] = useState(true)
  const list = blocks(transcript.items)
  const lastWork = list.findLastIndex((b) => b.kind === "work")

  // Follow new content while the view is at the bottom.
  useLayoutEffect(() => {
    const el = scroller.current
    if (el && stuck) el.scrollTop = el.scrollHeight
  })
  useEffect(() => {
    const el = scroller.current
    if (!el) return
    const onScroll = () => setStuck(el.scrollHeight - el.scrollTop - el.clientHeight < 48)
    el.addEventListener("scroll", onScroll, { passive: true })
    return () => el.removeEventListener("scroll", onScroll)
  }, [])

  return (
    <div className={cn("relative min-h-0", className)}>
      <div ref={scroller} className="h-full overflow-y-auto">
        <div className="mx-auto flex w-full max-w-[46rem] flex-col gap-4 px-6 pt-6 pb-10">
          {list.map((b, i) =>
            b.kind === "work" ? (
              <WorkGroup key={b.id} items={b.items} live={transcript.running && i === lastWork && i >= list.length - 2} />
            ) : (
              <Row key={`${b.item.kind}-${b.item.id}`} item={b.item} />
            ),
          )}
          {transcript.running && !transcript.pending && list[list.length - 1]?.kind !== "work" ? <span className="shimmer self-start text-[12.5px]">Working…</span> : null}
        </div>
      </div>
      {!stuck ? (
        <button
          className="absolute bottom-3 left-1/2 flex h-7 -translate-x-1/2 items-center gap-1.5 border bg-popover px-3 text-[12px] text-muted-foreground shadow-lg hover:text-foreground"
          onClick={() => {
            const el = scroller.current
            if (el) el.scrollTop = el.scrollHeight
            setStuck(true)
          }}
        >
          <ArrowDown className="size-3.5" /> Latest
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
          <div className="max-w-[85%] border bg-user px-3.5 py-2.5 text-[length:var(--oah-chat-size,14px)] leading-relaxed whitespace-pre-wrap select-text">
            {item.text}
            {item.images.length ? (
              <div className="mt-1.5 flex items-center gap-1 text-[11.5px] text-muted-foreground">
                <ImageIcon className="size-3" /> {item.images.length} image{item.images.length > 1 ? "s" : ""}
              </div>
            ) : null}
          </div>
        </div>
      )
    case "assistant":
      return <AssistantText text={item.text} />
    case "approval":
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
        <div className={cn("flex items-start gap-2 border px-3 py-2 text-[12.5px]", item.level === "error" ? "border-red/40 bg-del/40 text-red" : item.level === "warn" ? "border-warn/40 text-warn" : "text-muted-foreground")}>
          {item.level === "info" ? <Info className="mt-0.5 size-3.5 shrink-0" /> : <CircleAlert className="mt-0.5 size-3.5 shrink-0" />}
          <span className="select-text">{item.text}</span>
        </div>
      )
    default:
      return null
  }
}

function AssistantText({ text }: { text: string }) {
  const [copied, setCopied] = useState(false)
  return (
    <div className="group/msg relative">
      <Markdown text={text} />
      <button
        className="mt-1 flex h-6 items-center gap-1 text-[11.5px] text-faint opacity-0 group-hover/msg:opacity-100 hover:text-foreground"
        onClick={() => {
          navigator.clipboard?.writeText(text)
          setCopied(true)
          setTimeout(() => setCopied(false), 1200)
        }}
      >
        {copied ? <Check className="size-3" /> : <Copy className="size-3" />} {copied ? "Copied" : "Copy"}
      </button>
    </div>
  )
}
