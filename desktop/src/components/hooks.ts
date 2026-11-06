import { useEffect, useState } from "react"

/** A counter that ticks every `ms` while `on` (spinners, elapsed time). */
export function useTick(ms: number, on: boolean): number {
  const [n, setN] = useState(0)
  useEffect(() => {
    if (!on) return
    const id = setInterval(() => setN((x) => x + 1), ms)
    return () => clearInterval(id)
  }, [ms, on])
  return n
}

export const mod = (e: { ctrlKey: boolean; metaKey: boolean }) => (navigator.userAgent.includes("Mac") ? e.metaKey : e.ctrlKey)
export const modKey = navigator.userAgent.includes("Mac") ? "⌘" : "Ctrl+"
