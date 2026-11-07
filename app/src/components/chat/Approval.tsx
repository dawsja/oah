import { useEffect } from "react"
import { FilePen, ListChecks, ShieldQuestion, SquareTerminal } from "lucide-react"
import type { ApprovalRequest, Decision } from "@/lib/chat"
import { useRelative } from "./cwd"
import { DiffLines } from "./DiffView"
import { Markdown } from "./Markdown"
import { inTerminal } from "@/lib/terminal"
import { cn } from "@/lib/utils"

/** The agent's question, docked above the composer: Allow (Enter), Always (A), Deny (Esc). */
export function ApprovalCard({ request, queued = 0, onAnswer }: { request: ApprovalRequest; queued?: number; onAnswer: (d: Decision) => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // A terminal's keys are the shell's (Enter there must not allow anything).
      if (inTerminal(e.target)) return
      const typing = e.target instanceof HTMLTextAreaElement && e.target.value.trim() !== ""
      if (typing || e.ctrlKey || e.altKey || e.metaKey) return
      if (e.key === "Enter") onAnswer("allow")
      else if (e.key === "Escape") onAnswer("deny")
      else if ((e.key === "a" || e.key === "A") && request.canRemember && !(e.target instanceof HTMLTextAreaElement)) onAnswer("allowAlways")
      else return
      e.preventDefault()
    }
    // Capture: the card answers before the composer sees the key (Esc there would stop the turn).
    window.addEventListener("keydown", onKey, true)
    return () => window.removeEventListener("keydown", onKey, true)
  }, [request, onAnswer])

  const rel = useRelative()
  const plan = request.kind === "plan"
  const Icon = request.kind === "command" ? SquareTerminal : request.kind === "edit" ? FilePen : request.kind === "plan" ? ListChecks : ShieldQuestion
  return (
    <div className="rounded-lg border border-input bg-card">
      <div className="flex items-start gap-2.5 px-3.5 pt-3">
        <Icon className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline gap-2 text-[13px] font-semibold">
            {rel(request.title)}
            {queued ? <span className="rounded-sm border px-1 text-[10.5px] font-normal text-muted-foreground">+{queued} more waiting</span> : null}
          </div>
          {request.detail && request.kind === "plan" ? (
            <Markdown text={request.detail} className="rounded-md mt-2 max-h-[45vh] overflow-y-auto border bg-background px-3 py-2" />
          ) : request.detail ? (
            <div className={request.kind === "command" ? "rounded-md mt-1.5 border bg-background px-2.5 py-1.5 font-mono text-[12px] break-all text-code" : "mt-0.5 text-[12.5px] text-muted-foreground"}>{rel(request.detail)}</div>
          ) : null}
        </div>
      </div>
      {request.diff ? <DiffLines patch={request.diff.patch} className="rounded-md mx-3.5 mt-2.5 max-h-56 border bg-background py-1" /> : null}
      <div className="flex items-center gap-1.5 p-3">
        <button className="rounded-md flex h-7 items-center gap-2 bg-primary px-3 text-[12.5px] font-medium text-primary-foreground hover:bg-primary/90" onClick={() => onAnswer("allow")}>
          {plan ? "Approve plan" : "Allow"} <kbd className="font-mono text-[10.5px] opacity-60">Enter</kbd>
        </button>
        {request.canRemember ? (
          <button className="rounded-md flex h-7 items-center gap-2 border border-input px-3 text-[12.5px] hover:bg-accent" onClick={() => onAnswer("allowAlways")}>
            Always allow in this chat <kbd className="font-mono text-[10.5px] text-faint">A</kbd>
          </button>
        ) : null}
        <button className={cn("rounded-md ml-auto flex h-7 items-center gap-2 px-3 text-[12.5px] text-muted-foreground hover:bg-accent ", plan ? "hover:text-foreground" : "hover:text-red")} onClick={() => onAnswer("deny")}>
          {plan ? "Keep planning" : "Deny"} <kbd className="font-mono text-[10.5px] text-faint">Esc</kbd>
        </button>
      </div>
    </div>
  )
}
