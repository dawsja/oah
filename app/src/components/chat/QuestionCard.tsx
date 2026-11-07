import { useEffect, useRef, useState } from "react"
import { Check, ChevronLeft, MessageCircleQuestion } from "lucide-react"
import type { Question, QuestionRequest } from "@/lib/chat"
import { cn } from "@/lib/utils"

/** The agent's questions, docked above the composer, one at a time. Number keys pick an option
 *  (a single pick moves on by itself), Enter goes to the next question or submits, Backspace or
 *  ← goes back, Esc dismisses them all. Unanswered questions can be skipped; Submit sends what
 *  was answered. */
export function QuestionCard({ request, onAnswer }: { request: QuestionRequest; onAnswer: (answers: Record<string, string[]>) => void }) {
  const [step, setStep] = useState(0)
  const [picked, setPicked] = useState<Record<string, string[]>>({})
  const [typed, setTyped] = useState<Record<string, string>>({})
  const advance = useRef<number | null>(null)
  useEffect(() => {
    setStep(0)
    setPicked({})
    setTyped({})
  }, [request.requestId])
  useEffect(() => () => void (advance.current && clearTimeout(advance.current)), [])

  const qs = request.questions
  const last = step >= qs.length - 1
  const q = qs[Math.min(step, qs.length - 1)]
  const answerOf = (x: Question) => [...(picked[x.id] ?? []), ...(typed[x.id]?.trim() ? [typed[x.id].trim()] : [])]
  const answers = Object.fromEntries(qs.map((x) => [x.id, answerOf(x)]))
  const answered = answerOf(q).length > 0
  const anyAnswered = qs.some((x) => answers[x.id].length > 0)

  const skip = () => onAnswer(Object.fromEntries(qs.map((x) => [x.id, []])))
  const next = (a = answers) => {
    if (!qs[step] || a[qs[step].id].length === 0) return
    if (step < qs.length - 1) setStep(step + 1)
    else onAnswer(a)
  }
  const pick = (label: string) => {
    if (q.multi) {
      setPicked((p) => {
        const cur = p[q.id] ?? []
        return { ...p, [q.id]: cur.includes(label) ? cur.filter((l) => l !== label) : [...cur, label] }
      })
      return
    }
    const nextPicked = { ...picked, [q.id]: [label] }
    setPicked(nextPicked)
    setTyped((t) => ({ ...t, [q.id]: "" }))
    // A single pick answers the question: move on after a beat, so the pick shows.
    if (advance.current) clearTimeout(advance.current)
    const a = { ...answers, [q.id]: [label] }
    advance.current = window.setTimeout(() => (last ? onAnswer(a) : setStep((s) => s + 1)), 160)
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // The typed-answer field keeps its keys; an empty composer doesn't.
      const inField = e.target instanceof HTMLInputElement
      const composerTyping = e.target instanceof HTMLTextAreaElement && e.target.value.trim() !== ""
      if (composerTyping || e.ctrlKey || e.altKey || e.metaKey) return
      if (e.key === "Escape") skip()
      else if (e.key === "Enter" && !(e.target instanceof HTMLInputElement)) next()
      else if (!inField && /^[1-9]$/.test(e.key) && q.options[Number(e.key) - 1]) pick(q.options[Number(e.key) - 1].label)
      else if (!inField && (e.key === "Backspace" || e.key === "ArrowLeft") && step > 0) setStep(step - 1)
      else if (!inField && e.key === "ArrowRight" && answered && !last) setStep(step + 1)
      else return
      e.preventDefault()
    }
    // Capture: the card answers before the composer sees the key (Esc there would stop the turn).
    window.addEventListener("keydown", onKey, true)
    return () => window.removeEventListener("keydown", onKey, true)
  })

  if (!q) return null
  return (
    <div className="rounded-lg border border-input bg-card">
      <div className="flex items-center gap-2 px-3.5 pt-3 text-[13px] font-semibold">
        <MessageCircleQuestion className="size-4 text-muted-foreground" />
        {q.header ? <span className="rounded-sm border px-1 text-[10.5px] font-normal text-muted-foreground uppercase">{q.header}</span> : null}
        <span className="min-w-0 flex-1">{q.question}</span>
        {qs.length > 1 ? (
          <span className="flex shrink-0 items-center gap-1.5 text-[11px] font-normal text-muted-foreground" aria-label={`Question ${step + 1} of ${qs.length}`}>
            {qs.map((x, i) => (
              <button
                key={x.id}
                aria-label={`Question ${i + 1}`}
                disabled={i > step && !qs.slice(0, i).every((y) => answers[y.id].length)}
                onClick={() => setStep(i)}
                className={cn("size-1.5 rounded-full", i === step ? "bg-primary" : answers[x.id].length ? "bg-foreground/40" : "bg-input")}
              />
            ))}
            <span className="ml-1 font-mono">
              {step + 1}/{qs.length}
            </span>
          </span>
        ) : null}
      </div>
      <div className="max-h-[45vh] overflow-y-auto px-3.5 pt-2.5">
        {q.multi ? <div className="mb-1.5 text-[11px] text-faint">Pick any</div> : null}
        {q.options.length ? (
          <div className="flex flex-col gap-1">
            {q.options.map((o, i) => {
              const on = (picked[q.id] ?? []).includes(o.label)
              return (
                <button
                  key={o.label}
                  className={cn("flex items-start gap-2.5 rounded-md border px-2.5 py-2 text-left hover:bg-accent", on ? "border-primary bg-primary/10" : "border-input")}
                  onClick={() => pick(o.label)}
                >
                  {i < 9 ? (
                    <kbd className={cn("mt-px grid size-4 shrink-0 place-items-center rounded-sm border font-mono text-[10px]", on ? "border-primary bg-primary text-primary-foreground" : "border-input text-muted-foreground")}>
                      {on && q.multi ? <Check className="size-2.5" /> : i + 1}
                    </kbd>
                  ) : null}
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
            key={q.id}
            autoFocus={!q.options.length}
            type={q.secret ? "password" : "text"}
            value={typed[q.id] ?? ""}
            onChange={(e) => {
              const v = e.target.value
              setTyped((t) => ({ ...t, [q.id]: v }))
              // A typed answer replaces a single pick.
              if (!q.multi && v.trim()) setPicked((p) => ({ ...p, [q.id]: [] }))
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault()
                next()
              }
            }}
            placeholder={q.options.length ? "Or type an answer" : "Your answer"}
            className="mt-1.5 h-8 w-full rounded-md border border-input bg-background px-2.5 text-[12.5px] outline-none placeholder:text-faint focus:border-ring/70"
          />
        ) : null}
      </div>
      <div className="flex items-center gap-1.5 p-3">
        {step > 0 ? (
          <button className="flex h-7 items-center gap-1 rounded-md px-2 text-[12.5px] text-muted-foreground hover:bg-accent hover:text-foreground" onClick={() => setStep(step - 1)}>
            <ChevronLeft className="size-3.5" /> Back
          </button>
        ) : null}
        <button
          className={cn(
            "flex h-7 items-center gap-2 rounded-md px-3 text-[12.5px] font-medium disabled:bg-muted disabled:text-faint",
            answered || last ? "bg-primary text-primary-foreground hover:bg-primary/90" : "border border-input hover:bg-accent",
          )}
          disabled={last && !anyAnswered}
          onClick={() => (last ? onAnswer(answers) : setStep(step + 1))}
        >
          {last ? "Submit" : answered ? "Next" : "Skip"} {answered ? <kbd className="font-mono text-[10.5px] opacity-60">Enter</kbd> : null}
        </button>
        <span className="text-[11px] text-faint">{q.options.length ? `1–${Math.min(q.options.length, 9)} to pick` : ""}</span>
        <button className="ml-auto flex h-7 items-center gap-2 rounded-md px-3 text-[12.5px] text-muted-foreground hover:bg-accent" onClick={skip}>
          Dismiss <kbd className="font-mono text-[10.5px] text-faint">Esc</kbd>
        </button>
      </div>
    </div>
  )
}
