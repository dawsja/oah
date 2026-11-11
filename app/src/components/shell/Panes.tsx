import { Fragment, useRef, type RefObject } from "react"
import { ChatView } from "@/components/chat/ChatView"
import { ErrorBoundary } from "@/components/ErrorBoundary"
import { cn } from "@/lib/utils"
import { PaneContext, useApp, type Pane, type SplitDir } from "@/store/app"

const MIN_SHARE = 0.25

/** The main area: one pane, or two side by side or one above the other, with a divider to
 *  drag. Each shows its own chat, in its own project; a click or a keystroke in one makes it
 *  the active pane. */
export function Panes() {
  const panes = useApp((s) => s.panes)
  const active = useApp((s) => s.activePane)
  const split = useApp((s) => s.layout.split)
  const dir = useApp((s) => s.layout.splitDir)
  const box = useRef<HTMLDivElement>(null)
  const share = Math.min(1 - MIN_SHARE, Math.max(MIN_SHARE, split))
  return (
    <div ref={box} className={cn("flex min-h-0 flex-1", dir === "down" && "flex-col")}>
      {panes.map((p, i) => (
        <Fragment key={p.id}>
          {i > 0 ? <SplitHandle box={box} dir={dir} /> : null}
          <PaneFrame pane={p} active={p.id === active} alone={panes.length === 1} share={panes.length === 1 ? 1 : i === 0 ? share : 1 - share} />
        </Fragment>
      ))}
    </div>
  )
}

function PaneFrame({ pane, active, alone, share }: { pane: Pane; active: boolean; alone: boolean; share: number }) {
  const focus = () => useApp.getState().focusPane(pane.id)
  return (
    <PaneContext.Provider value={pane.id}>
      <section
        data-pane={pane.id}
        aria-label={alone ? undefined : active ? "Active pane" : "Pane"}
        className={cn("relative min-h-0 min-w-0 basis-0 overflow-clip rounded-lg border bg-background", !alone && active && "border-primary/45")}
        style={{ flexGrow: share }}
        onPointerDownCapture={focus}
        onFocusCapture={focus}
      >
        <ErrorBoundary label="this chat">
          <ChatView />
        </ErrorBoundary>
      </section>
    </PaneContext.Provider>
  )
}

/** The divider between two panes: drag to give one more room, double-click for halves. */
function SplitHandle({ box, dir }: { box: RefObject<HTMLDivElement | null>; dir: SplitDir }) {
  const dragging = useRef(false)
  const down = dir === "down"
  const set = (split: number) => useApp.getState().setLayout({ split: Math.round(Math.min(1 - MIN_SHARE, Math.max(MIN_SHARE, split)) * 1000) / 1000 })
  return (
    <div
      role="separator"
      aria-orientation={down ? "horizontal" : "vertical"}
      title="Drag to resize · Double-click to reset"
      className={cn(
        "relative z-10 shrink-0 after:absolute after:rounded-full after:transition-colors hover:after:bg-ring/60 active:after:bg-ring",
        down
          ? "h-2 cursor-row-resize after:inset-x-[40%] after:top-1/2 after:h-[3px] after:-translate-y-1/2"
          : "w-2 cursor-col-resize after:inset-y-[40%] after:left-1/2 after:w-[3px] after:-translate-x-1/2",
      )}
      onDoubleClick={() => {
        dragging.current = false
        set(0.5)
      }}
      onPointerDown={(e) => {
        dragging.current = true
        e.currentTarget.setPointerCapture(e.pointerId)
      }}
      onPointerMove={(e) => {
        const r = box.current?.getBoundingClientRect()
        if (!dragging.current || !r) return
        if (down ? r.height > 0 : r.width > 0) set(down ? (e.clientY - r.top) / r.height : (e.clientX - r.left) / r.width)
      }}
      onPointerUp={() => (dragging.current = false)}
    />
  )
}
