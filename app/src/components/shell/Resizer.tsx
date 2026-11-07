import { useRef } from "react"

/** A 1px divider with a wider invisible grab area; reports the new width while dragged. */
export function Resizer({ width, onWidth, side, min, max }: { width: number; onWidth: (w: number) => void; side: "left" | "right"; min: number; max: number }) {
  const start = useRef<{ x: number; w: number } | null>(null)
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      className="relative z-10 -mx-[3px] w-[7px] shrink-0 cursor-col-resize hover:bg-ring/50 active:bg-ring"
      onPointerDown={(e) => {
        start.current = { x: e.clientX, w: width }
        e.currentTarget.setPointerCapture(e.pointerId)
      }}
      onPointerMove={(e) => {
        if (!start.current) return
        const dx = e.clientX - start.current.x
        const w = start.current.w + (side === "left" ? dx : -dx)
        onWidth(Math.round(Math.min(max, Math.max(min, w))))
      }}
      onPointerUp={() => (start.current = null)}
    />
  )
}
