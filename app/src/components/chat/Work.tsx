import { useEffect, useState } from "react"
import { Brain, Check, ChevronDown, ChevronRight, FilePen, FileText, Globe, ListTree, Loader2, Search, SquareTerminal, Wrench, X, type LucideIcon } from "lucide-react"
import type { Item, ToolItem, ToolKind } from "@/lib/chat"
import { cn } from "@/lib/utils"
import { useRelative } from "./cwd"
import { DiffCard, DiffStat } from "./DiffView"

const ICON: Record<ToolKind, LucideIcon> = {
  read: FileText,
  edit: FilePen,
  write: FilePen,
  bash: SquareTerminal,
  search: Search,
  web: Globe,
  task: ListTree,
  other: Wrench,
}

const VERB: Record<ToolKind, string> = {
  read: "Read",
  edit: "Edit",
  write: "Write",
  bash: "Run",
  search: "Search",
  web: "Fetch",
  task: "Task",
  other: "Tool",
}

/** A run of reasoning and tool calls: one summary line, expanded while the turn runs. Edits
 *  keep their diffs visible even when the run is folded. */
export function WorkGroup({ items, live }: { items: Item[]; live: boolean }) {
  const [open, setOpen] = useState(live)
  // Open while the agent works on it, folded once the turn moves on.
  useEffect(() => setOpen(live), [live])
  const tools = items.filter((i): i is ToolItem => i.kind === "tool")
  const running = tools.some((t) => t.status === "running")
  const failed = tools.filter((t) => t.status === "error").length
  const names = [...new Set(tools.map((t) => VERB[t.tool.kind]))]
  const diffs = tools.filter((t) => t.diff)
  const summary = tools.length === 0 ? "Thought" : `${tools.length} step${tools.length === 1 ? "" : "s"}`

  return (
    <div className="flex flex-col gap-1.5">
      <button className="rounded-md group flex h-6 items-center gap-1.5 self-start text-[12.5px] text-muted-foreground hover:text-foreground" onClick={() => setOpen(!open)}>
        {open ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
        <span className={cn(running && "shimmer")}>{summary}</span>
        {names.length ? <span className="text-faint">· {names.join(", ")}</span> : null}
        {failed ? <span className="text-red">· {failed} failed</span> : null}
      </button>
      {open ? (
        <div className="ml-[7px] flex flex-col gap-0.5 border-l pl-4">
          {items.map((it) => (it.kind === "reasoning" ? <Reasoning key={it.id} text={it.text} /> : it.kind === "tool" ? <ToolRow key={it.id} item={it} /> : null))}
        </div>
      ) : (
        diffs.map((t) => <DiffCard key={t.id} diff={t.diff!} defaultOpen={false} />)
      )}
    </div>
  )
}

function Reasoning({ text }: { text: string }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="py-0.5">
      <button className="flex min-h-6 w-full items-start gap-2 text-left text-[12.5px] text-faint hover:text-muted-foreground" onClick={() => setOpen(!open)}>
        <Brain className="mt-[3px] size-3.5 shrink-0" />
        <span className={cn("italic", !open && "line-clamp-1")}>{text}</span>
      </button>
    </div>
  )
}

export function ToolRow({ item }: { item: ToolItem }) {
  const [open, setOpen] = useState(false)
  const rel = useRelative()
  const Icon = ICON[item.tool.kind]
  const hasOutput = !!item.output.trim()
  return (
    <div className="py-0.5">
      <button
        className="flex min-h-6 w-full items-center gap-2 text-left text-[12.5px] hover:text-foreground disabled:cursor-default"
        onClick={() => setOpen(!open)}
        disabled={!hasOutput && !item.diff}
      >
        <Icon className="size-3.5 shrink-0 text-muted-foreground" />
        <span className="shrink-0 text-muted-foreground">{VERB[item.tool.kind]}</span>
        <span className={cn("min-w-0 truncate font-mono text-[12px]", item.tool.kind === "bash" ? "text-code" : "text-foreground")}>{rel(item.tool.title)}</span>
        <span className="ml-auto flex shrink-0 items-center gap-2 pl-2">
          {item.diff ? <DiffStat added={item.diff.added} removed={item.diff.removed} /> : null}
          {item.status === "running" ? (
            <Loader2 className="size-3.5 animate-spin text-muted-foreground" />
          ) : item.status === "ok" ? (
            <Check className="size-3.5 text-green" />
          ) : (
            <X className="size-3.5 text-red" />
          )}
        </span>
      </button>
      {item.diff ? (
        <div className="mt-1 mb-1.5">
          <DiffCard diff={item.diff} defaultOpen />
        </div>
      ) : open && hasOutput ? (
        <pre className={cn("rounded-md mt-1 mb-1.5 max-h-64 overflow-auto border bg-card px-3 py-2 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap select-text", item.status === "error" ? "text-red" : "text-muted-foreground")}>
          {item.output}
        </pre>
      ) : null}
    </div>
  )
}
