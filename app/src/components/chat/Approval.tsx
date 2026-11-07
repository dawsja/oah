import { useEffect } from "react"
import { FilePen, ShieldQuestion, SquareTerminal } from "lucide-react"
import type { ApprovalRequest, Decision } from "@/lib/chat"
import { useRelative } from "./cwd"
import { DiffLines } from "./DiffView"

/** The agent's question, docked above the composer: Allow (Enter), Always (A), Deny (Esc). */
export function ApprovalCard({ request, onAnswer }: { request: ApprovalRequest; onAnswer: (d: Decision) => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const typing = e.target instanceof HTMLTextAreaElement && e.target.value.trim() !== ""
      if (typing || e.ctrlKey || e.altKey || e.metaKey) return
      if (e.key === "Enter") onAnswer("allow")
      else if (e.key === "Escape") onAnswer("deny")
      else if ((e.key === "a" || e.key === "A") && request.canRemember && !(e.target instanceof HTMLTextAreaElement)) onAnswer("allowAlways")
      else return
      e.preventDefault()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [request, onAnswer])

  const rel = useRelative()
  const Icon = request.kind === "command" ? SquareTerminal : request.kind === "edit" ? FilePen : ShieldQuestion
  return (
    <div className="rounded-lg border border-primary/60 bg-card">
      <div className="flex items-start gap-2.5 px-3.5 pt-3">
        <Icon className="mt-0.5 size-4 shrink-0 text-gold" />
        <div className="min-w-0 flex-1">
          <div className="text-[13px] font-semibold">{rel(request.title)}</div>
          {request.detail ? (
            <div className={request.kind === "command" ? "rounded-md mt-1.5 border bg-background px-2.5 py-1.5 font-mono text-[12px] break-all text-code" : "mt-0.5 text-[12.5px] text-muted-foreground"}>{rel(request.detail)}</div>
          ) : null}
        </div>
      </div>
      {request.diff ? <DiffLines patch={request.diff.patch} className="rounded-md mx-3.5 mt-2.5 max-h-56 border bg-background py-1" /> : null}
      <div className="flex items-center gap-1.5 p-3">
        <button className="rounded-md flex h-7 items-center gap-2 bg-primary px-3 text-[12.5px] font-medium text-primary-foreground hover:bg-primary/90" onClick={() => onAnswer("allow")}>
          Allow <kbd className="font-mono text-[10.5px] opacity-60">Enter</kbd>
        </button>
        {request.canRemember ? (
          <button className="rounded-md flex h-7 items-center gap-2 border border-input px-3 text-[12.5px] hover:bg-accent" onClick={() => onAnswer("allowAlways")}>
            Always allow in this chat <kbd className="font-mono text-[10.5px] text-faint">A</kbd>
          </button>
        ) : null}
        <button className="rounded-md ml-auto flex h-7 items-center gap-2 px-3 text-[12.5px] text-muted-foreground hover:bg-accent hover:text-red" onClick={() => onAnswer("deny")}>
          Deny <kbd className="font-mono text-[10.5px] text-faint">Esc</kbd>
        </button>
      </div>
    </div>
  )
}
