import { Fragment, useRef, type RefObject } from "react"
import { ChatView } from "@/components/chat/ChatView"
import { ErrorBoundary } from "@/components/ErrorBoundary"
import { cn } from "@/lib/utils"
import { PaneContext, useApp, type Pane } from "@/store/app"

const MIN_SHARE = 0.25

/** The main area: one pane, or two side by side with a divider to drag. Each shows its own
 *  chat, in its own project; a click or a keystroke in one makes it the active pane. */
export function Panes() {
  const panes = useApp((s) => s.panes)
  const active = useApp((s) => s.activePane)
  const split = useApp((s) => s.layout.split)
  const box = useRef<HTMLDivElement>(null)
  const share = Math.min(1 - MIN_SHARE, Math.max(MIN_SHARE, split))
  return (
    <div ref={box} className="flex min-h-0 flex-1">
      {panes.map((p, i) => (
        <Fragment key={p.id}>
          {i > 0 ? <SplitHandle box={box} /> : null}
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
function SplitHandle({ box }: { box: RefObject<HTMLDivElement | null> }) {
  const dragging = useRef(false)
  const set = (split: number) => useApp.getState().setLayout({ split: Math.round(Math.min(1 - MIN_SHARE, Math.max(MIN_SHARE, split)) * 1000) / 1000 })
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      title="Drag to resize · Double-click to reset"
      className="relative z-10 w-2 shrink-0 cursor-col-resize after:absolute after:inset-y-[40%] after:left-1/2 after:w-[3px] after:-translate-x-1/2 after:rounded-full after:transition-colors hover:after:bg-ring/60 active:after:bg-ring"
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
        if (dragging.current && r && r.width > 0) set((e.clientX - r.left) / r.width)
      }}
      onPointerUp={() => (dragging.current = false)}
    />
  )
}
