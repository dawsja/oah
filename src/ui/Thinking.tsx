// Animated "thinking" label: a braille spinner plus a highlight that sweeps
// across the letters.
import { useEffect, useState } from "react"
import { mix, SPINNER, t } from "./theme"

export function useTick(ms: number, active = true): number {
  const [n, setN] = useState(0)
  useEffect(() => {
    if (!active) return
    const id = setInterval(() => setN((x) => x + 1), ms)
    return () => clearInterval(id)
  }, [ms, active])
  return n
}

export function Shimmer({ text, tick, base = t.dim, glow = t.text, width = 4 }: { text: string; tick: number; base?: string; glow?: string; width?: number }) {
  const chars = [...text]
  const span = chars.length + width * 2
  const pos = (tick % span) - width
  return (
    <>
      {chars.map((c, i) => {
        const d = Math.abs(i - pos)
        const k = Math.max(0, 1 - d / width)
        return (
          <span key={i} fg={mix(base, glow, k)}>
            {c}
          </span>
        )
      })}
    </>
  )
}

/** One-line activity indicator: ⠹ Thinking 4s · esc to interrupt */
export function Thinking({ label, started, color = t.accent, hint = "esc to interrupt" }: { label: string; started: number; color?: string; hint?: string }) {
  const tick = useTick(80)
  const secs = Math.floor((Date.now() - started) / 1000)
  const el = secs >= 60 ? `${Math.floor(secs / 60)}m${String(secs % 60).padStart(2, "0")}s` : `${secs}s`
  return (
    <text>
      <span fg={color}>{SPINNER[tick % SPINNER.length]} </span>
      <Shimmer text={label} tick={tick} />
      <span fg={t.faint}>
        {"  "}
        {el}
        {hint ? ` · ${hint}` : ""}
      </span>
    </text>
  )
}
