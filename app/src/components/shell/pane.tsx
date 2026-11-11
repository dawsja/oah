import { ArrowDownToLine, ArrowRightToLine, Columns2, Rows2, X } from "lucide-react"
import { Tip } from "@/components/Tip"
import { DropdownMenuItem, DropdownMenuShortcut } from "@/components/ui/dropdown-menu"
import { cn } from "@/lib/utils"
import { useApp, usePane, type SplitDir } from "@/store/app"

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

/** A chat menu's way to a second pane: to the right or below with one pane, and with two,
 *  where this one goes (to the right of the other, or under it). */
export function SplitItems() {
  const pane = usePane()
  const two = useApp((s) => s.panes.length > 1)
  const last = useApp((s) => s.panes[s.panes.length - 1]?.id === pane.id)
  const dir = useApp((s) => s.layout.splitDir)
  const split = (d: SplitDir) => {
    useApp.getState().splitPane(undefined, d)
    focusPaneInput(useApp.getState().activePane)
  }
  const item = (label: string, icon: React.ReactNode, onSelect: () => void, shortcut?: string, disabled = false) => (
    <DropdownMenuItem disabled={disabled} onSelect={onSelect}>
      {icon}
      <span className="flex-1 whitespace-nowrap">{label}</span>
      {shortcut ? <DropdownMenuShortcut className="pl-4">{shortcut}</DropdownMenuShortcut> : null}
    </DropdownMenuItem>
  )
  if (!two) {
    return (
      <>
        {item("Split right", <Columns2 />, () => split("right"), "Ctrl+\\")}
        {item("Split down", <Rows2 />, () => split("down"), "Ctrl+Alt+\\")}
      </>
    )
  }
  const place = (d: SplitDir) => useApp.getState().placePane(d, pane.id)
  return (
    <>
      {item("Move to the right", <ArrowRightToLine />, () => place("right"), undefined, last && dir === "right")}
      {item("Move below", <ArrowDownToLine />, () => place("down"), undefined, last && dir === "down")}
    </>
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
