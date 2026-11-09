import { useEffect, useRef, useState } from "react"
import { Check, ChevronLeft, MessageCircleQuestion } from "lucide-react"
import type { Question, QuestionRequest } from "@/lib/chat"
import { CARD_ARM_MS, cardKeyScope } from "@/lib/card-keys"
import { cn } from "@/lib/utils"
import { useTouch } from "./touch"

/** The agent's questions, docked above the composer, one at a time. Number keys pick an option
 *  (a single pick moves on to the next question by itself; on the last one it waits for Submit),
 *  Enter goes to the next question or submits, Backspace or
 *  ← goes back, Esc dismisses them all. Unanswered questions can be skipped; Submit sends what
 *  was answered. */
export function QuestionCard({ request, onAnswer }: { request: QuestionRequest; onAnswer: (answers: Record<string, string[]>) => void }) {
  const [step, setStep] = useState(0)
  const touch = useTouch()
  const [picked, setPicked] = useState<Record<string, string[]>>({})
  const [typed, setTyped] = useState<Record<string, string>>({})
  const advance = useRef<number | null>(null)
  const card = useRef<HTMLDivElement>(null)
  const shownAt = useRef(0)
  useEffect(() => {
    setStep(0)
    setPicked({})
    setTyped({})
    shownAt.current = performance.now()
  }, [request.requestId])
  useEffect(() => () => void (advance.current && clearTimeout(advance.current)), [])

  const qs = request.questions
  const last = step >= qs.length - 1
  const q = qs[Math.min(step, qs.length - 1)] as Question | undefined
  const answerOf = (x: Question) => [...(picked[x.id] ?? []), ...(typed[x.id]?.trim() ? [typed[x.id].trim()] : [])]
  const answers = Object.fromEntries(qs.map((x) => [x.id, answerOf(x)]))
  const answered = q ? answerOf(q).length > 0 : false
  const anyAnswered = qs.some((x) => answers[x.id].length > 0)

  /** Every way of moving between questions goes through here, so a pending auto-advance
   *  can't move a second time, and the step never runs past the last question. */
  const go = (to: number) => {
    if (advance.current) clearTimeout(advance.current)
    advance.current = null
    setStep(Math.max(0, Math.min(to, qs.length - 1)))
  }
  const skip = () => onAnswer(Object.fromEntries(qs.map((x) => [x.id, []])))
  const next = (a = answers) => {
    if (!qs[step] || a[qs[step].id].length === 0) return
    if (step < qs.length - 1) go(step + 1)
    else onAnswer(a)
  }
  const pick = (label: string) => {
    if (!q) return
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
    // A single pick answers the question: move on after a beat, so the pick shows. The last
    // question never sends by itself; Submit (or Enter) does, so the answers can be checked.
    if (advance.current) clearTimeout(advance.current)
    if (!last) advance.current = window.setTimeout(() => go(step + 1), 160)
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Only when nothing else has the key; in the typed-answer field, only Esc.
      const scope = cardKeyScope(e, card.current)
      if (!scope || !q || performance.now() - shownAt.current < CARD_ARM_MS) return
      if (e.key === "Escape") skip()
      else if (scope === "field") return
      else if (e.key === "Enter") next()
      else if (/^[1-9]$/.test(e.key) && q.options[Number(e.key) - 1]) pick(q.options[Number(e.key) - 1].label)
      else if ((e.key === "Backspace" || e.key === "ArrowLeft") && step > 0) go(step - 1)
      else if (e.key === "ArrowRight" && answered && !last) go(step + 1)
      else return
      e.preventDefault()
    }
    // Capture: the card answers before the composer sees the key (Esc there would stop the turn).
    window.addEventListener("keydown", onKey, true)
    return () => window.removeEventListener("keydown", onKey, true)
  })

  if (!q) return null
  return (
    <div ref={card} className="flex min-h-0 max-h-[45vh] flex-col overflow-hidden rounded-2xl border border-input bg-card">
      <div className="min-h-0 overflow-y-auto overscroll-contain [overflow-wrap:anywhere]" role="region" aria-label="Question details" tabIndex={0}>
        {touch ? (
          // A phone: what it's about and how far along on one line, the question under it.
          <div className="px-3.5 pt-3">
            <div className="flex items-center gap-2 text-[12px] text-muted-foreground">
              <MessageCircleQuestion className="size-4" />
              {q.header ? <span className="rounded-full border px-1.5 py-px uppercase">{q.header}</span> : null}
              {qs.length > 1 ? <span className="ml-auto font-mono">{step + 1}/{qs.length}</span> : null}
            </div>
            <div className="mt-1.5 text-[15px] leading-snug font-semibold">{q.question}</div>
          </div>
        ) : (
        <div className="flex items-center gap-2 px-3.5 pt-3 text-[13px] font-semibold">
          <MessageCircleQuestion className="size-4 text-muted-foreground" />
          {q.header ? <span className="rounded-full border px-1.5 text-[10.5px] font-normal text-muted-foreground uppercase">{q.header}</span> : null}
          <span className="min-w-0 flex-1">{q.question}</span>
          {qs.length > 1 ? (
            <span className="flex shrink-0 items-center gap-1.5 text-[11px] font-normal text-muted-foreground" aria-label={`Question ${step + 1} of ${qs.length}`}>
              {qs.map((x, i) => (
                <button
                  key={x.id}
                  aria-label={`Question ${i + 1}`}
                  disabled={i > step && !qs.slice(0, i).every((y) => answers[y.id].length)}
                  onClick={() => go(i)}
                  className={cn("size-1.5 rounded-full", i === step ? "bg-primary" : answers[x.id].length ? "bg-foreground/40" : "bg-input")}
                />
              ))}
              <span className="ml-1 font-mono">
                {step + 1}/{qs.length}
              </span>
            </span>
          ) : null}
        </div>
        )}
        <div className={cn("overflow-y-auto px-3.5 pt-2.5", touch ? "max-h-[38vh]" : "max-h-[45vh]")}>
          {q.multi ? <div className="mb-1.5 text-[11px] text-faint">Pick any</div> : null}
          {q.options.length ? (
            <div className={cn("flex flex-col", touch ? "gap-2" : "gap-1")}>
              {q.options.map((o, i) => {
                const on = (picked[q.id] ?? []).includes(o.label)
                return (
                  <button
                    key={o.label}
                    className={cn("flex items-start gap-2.5 rounded-md border text-left hover:bg-accent", touch ? "min-h-12 px-3 py-2.5" : "px-2.5 py-2", on ? "border-primary bg-primary/10" : "border-input")}
                    onClick={() => pick(o.label)}
                  >
                    {i < 9 && !touch ? (
                      <kbd className={cn("mt-px grid size-4 shrink-0 place-items-center rounded-xs border font-mono text-[10px]", on ? "border-primary bg-primary text-primary-foreground" : "border-input text-muted-foreground")}>
                        {on && q.multi ? <Check className="size-2.5" /> : i + 1}
                      </kbd>
                    ) : null}
                    <span className="min-w-0">
                      <span className={cn("block font-medium", touch ? "text-[14px]" : "text-[12.5px]")}>{o.label}</span>
                      {o.description ? <span className={cn("block text-muted-foreground", touch ? "mt-0.5 text-[13px]" : "text-[11.5px]")}>{o.description}</span> : null}
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
              className={cn("w-full rounded-md border border-input bg-background outline-none placeholder:text-faint focus:border-ring/70", touch ? "mt-2 h-11 px-3 text-[15px]" : "mt-1.5 h-8 px-2.5 text-[12.5px]")}
            />
          ) : null}
        </div>
      </div>
      <div className={cn("flex shrink-0 flex-wrap items-center p-3", touch ? "gap-2" : "gap-1.5")}>
        {step > 0 ? (
          <button className={cn("flex items-center gap-1 rounded-md px-2 text-muted-foreground hover:bg-accent hover:text-foreground", touch ? "h-11 text-[14px]" : "h-7 text-[12.5px]")} onClick={() => go(step - 1)}>
            <ChevronLeft className="size-3.5" /> Back
          </button>
        ) : null}
        <button
          className={cn(
            "flex items-center gap-2 rounded-md px-3 font-medium disabled:bg-muted disabled:text-faint",
            touch ? "h-11 min-w-24 justify-center px-4 text-[14px]" : "h-7 text-[12.5px]",
            answered || last ? "bg-primary text-primary-foreground hover:bg-primary/90" : "border border-input hover:bg-accent",
          )}
          disabled={last && !anyAnswered}
          onClick={() => (last ? onAnswer(answers) : go(step + 1))}
        >
          {last ? "Submit" : answered ? "Next" : "Skip"} {answered && !touch ? <kbd className="font-mono text-[10.5px] opacity-60">Enter</kbd> : null}
        </button>
        {touch ? null : <span className="text-[11px] text-faint">{q.options.length ? `1–${Math.min(q.options.length, 9)} to pick` : ""}</span>}
        <button className={cn("ml-auto flex items-center gap-2 rounded-md px-3 text-muted-foreground hover:bg-accent", touch ? "h-11 text-[14px]" : "h-7 text-[12.5px]")} onClick={skip}>
          Dismiss {touch ? null : <kbd className="font-mono text-[10.5px] text-faint">Esc</kbd>}
        </button>
      </div>
    </div>
  )
}
