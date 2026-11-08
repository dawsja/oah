import { useRef } from "react"
import { cn } from "@/lib/utils"

/** A 1px divider with a wider invisible grab area; reports the new size while dragged. `side`
 *  is where the resized pane is: left or right of the divider, or above it ("top": a pane under
 *  the divider that grows upwards). */
export function Resizer({ width, defaultWidth, onWidth, side, min, max }: { width: number; defaultWidth: number; onWidth: (w: number) => void; side: "left" | "right" | "top"; min: number; max: number }) {
  const start = useRef<{ at: number; w: number } | null>(null)
  const vertical = side === "top"
  return (
    <div
      role="separator"
      title="Drag to resize · Double-click to reset"
      onDoubleClick={() => {
        start.current = null
        onWidth(Math.min(max, Math.max(min, defaultWidth)))
      }}
      aria-orientation={vertical ? "horizontal" : "vertical"}
      className={cn(
        "relative z-10 shrink-0 hover:bg-ring/50 active:bg-ring",
        vertical ? "-my-[3px] h-[7px] w-full cursor-row-resize" : "-mx-[3px] w-[7px] cursor-col-resize",
      )}
      onPointerDown={(e) => {
        start.current = { at: vertical ? e.clientY : e.clientX, w: width }
        e.currentTarget.setPointerCapture(e.pointerId)
      }}
      onPointerMove={(e) => {
        if (!start.current) return
        const d = (vertical ? e.clientY : e.clientX) - start.current.at
        const w = start.current.w + (side === "left" ? d : -d)
        onWidth(Math.round(Math.min(max, Math.max(min, w))))
      }}
      onPointerUp={() => (start.current = null)}
    />
  )
}
