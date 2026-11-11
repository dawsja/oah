import { X } from "lucide-react"
import { Tip } from "@/components/Tip"
import { cn } from "@/lib/utils"
import { useApp, usePane } from "@/store/app"

/** Closes the pane it's in; nothing while there's only one. */
export function PaneClose({ className }: { className?: string }) {
  const pane = usePane()
  const alone = useApp((s) => s.panes.length < 2)
  if (alone) return null
  return (
    <Tip content="Close this pane (Ctrl+Shift+\)" side="bottom">
      <button
        className={cn("rounded-md grid size-7 shrink-0 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground", className)}
        aria-label="Close pane"
        onClick={() => useApp.getState().closePane(pane.id)}
      >
        <X className="size-4" />
      </button>
    </Tip>
  )
}

/** Puts the cursor where a pane takes typing: its CLI, else its composer. */
export function focusPaneInput(id: string) {
  requestAnimationFrame(() => {
    const el = document.querySelector<HTMLElement>(`[data-pane="${id}"]`)
    const input = el?.querySelector<HTMLElement>(".xterm-helper-textarea") ?? el?.querySelector<HTMLElement>("textarea")
    input?.focus({ preventScroll: true })
  })
}
