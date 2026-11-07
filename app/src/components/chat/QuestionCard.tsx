import { useEffect, useState } from "react"
import { Check, MessageCircleQuestion } from "lucide-react"
import type { QuestionRequest } from "@/lib/chat"
import { cn } from "@/lib/utils"

/** The agent's questions, docked above the composer: pick options (or type an answer), then
 *  Submit. Enter submits when every question has an answer; Esc skips. */
export function QuestionCard({ request, onAnswer }: { request: QuestionRequest; onAnswer: (answers: Record<string, string[]>) => void }) {
  const [picked, setPicked] = useState<Record<string, string[]>>({})
  const [typed, setTyped] = useState<Record<string, string>>({})
  useEffect(() => {
    setPicked({})
    setTyped({})
  }, [request.requestId])

  const answers = Object.fromEntries(request.questions.map((q) => [q.id, [...(picked[q.id] ?? []), ...(typed[q.id]?.trim() ? [typed[q.id].trim()] : [])]]))
  const complete = request.questions.every((q) => answers[q.id].length > 0)

  const toggle = (qid: string, label: string, multi: boolean) =>
    setPicked((p) => {
      const cur = p[qid] ?? []
      if (!multi) return { ...p, [qid]: cur[0] === label ? [] : [label] }
      return { ...p, [qid]: cur.includes(label) ? cur.filter((l) => l !== label) : [...cur, label] }
    })

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLTextAreaElement && e.target.value.trim()) return
      if (e.key === "Escape") {
        e.preventDefault()
        onAnswer(Object.fromEntries(request.questions.map((q) => [q.id, []])))
      } else if (e.key === "Enter" && complete && !(e.target instanceof HTMLInputElement)) {
        e.preventDefault()
        onAnswer(answers)
      }
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  })

  return (
    <div className="rounded-lg border border-primary/60 bg-card">
      <div className="flex items-center gap-2 px-3.5 pt-3 text-[13px] font-semibold">
        <MessageCircleQuestion className="size-4 text-gold" />
        {request.questions.length > 1 ? `${request.questions.length} questions` : "A question"}
      </div>
      <div className="flex max-h-[45vh] flex-col gap-3 overflow-y-auto px-3.5 pt-2.5">
        {request.questions.map((q) => (
          <div key={q.id}>
            <div className="mb-1.5 flex items-baseline gap-2">
              {q.header ? <span className="rounded-sm border border-gold/40 px-1 text-[10.5px] text-gold uppercase">{q.header}</span> : null}
              <span className="text-[13px]">{q.question}</span>
              {q.multi ? <span className="text-[11px] text-faint">pick any</span> : null}
            </div>
            {q.options.length ? (
              <div className="grid grid-cols-1 gap-1 sm:grid-cols-2">
                {q.options.map((o) => {
                  const on = (picked[q.id] ?? []).includes(o.label)
                  return (
                    <button
                      key={o.label}
                      className={cn("flex items-start gap-2 rounded-md border px-2.5 py-2 text-left hover:bg-accent", on ? "border-primary bg-primary/10" : "border-input")}
                      onClick={() => toggle(q.id, o.label, q.multi)}
                    >
                      <span className={cn("mt-0.5 grid size-3.5 shrink-0 place-items-center border", q.multi ? "rounded-sm" : "rounded-full", on ? "border-primary bg-primary text-primary-foreground" : "border-input")}>
                        {on ? <Check className="size-2.5" /> : null}
                      </span>
                      <span className="min-w-0">
                        <span className="block text-[12.5px] font-medium">{o.label}</span>
                        {o.description ? <span className="block text-[11.5px] text-muted-foreground">{o.description}</span> : null}
                      </span>
                    </button>
                  )
                })}
              </div>
            ) : null}
            {q.other || !q.options.length ? (
              <input
                type={q.secret ? "password" : "text"}
                value={typed[q.id] ?? ""}
                onChange={(e) => setTyped((t) => ({ ...t, [q.id]: e.target.value }))}
                onKeyDown={(e) => e.key === "Enter" && complete && onAnswer(answers)}
                placeholder={q.options.length ? "Or type an answer" : "Your answer"}
                className="mt-1.5 h-8 w-full rounded-md border border-input bg-background px-2.5 text-[12.5px] outline-none placeholder:text-faint focus:border-ring/70"
              />
            ) : null}
          </div>
        ))}
      </div>
      <div className="flex items-center gap-1.5 p-3">
        <button className="flex h-7 items-center gap-2 rounded-md bg-primary px-3 text-[12.5px] font-medium text-primary-foreground hover:bg-primary/90 disabled:bg-muted disabled:text-faint" disabled={!complete} onClick={() => onAnswer(answers)}>
          Submit <kbd className="font-mono text-[10.5px] opacity-60">Enter</kbd>
        </button>
        <button className="ml-auto flex h-7 items-center gap-2 rounded-md px-3 text-[12.5px] text-muted-foreground hover:bg-accent" onClick={() => onAnswer(Object.fromEntries(request.questions.map((q) => [q.id, []])))}>
          Skip <kbd className="font-mono text-[10.5px] text-faint">Esc</kbd>
        </button>
      </div>
    </div>
  )
}
