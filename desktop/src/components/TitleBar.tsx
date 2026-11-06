import { PanelRight } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Separator } from "@/components/ui/separator"
import { SidebarTrigger } from "@/components/ui/sidebar"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { useClient } from "@/lib/client"
import { targetLabel } from "@/lib/protocol"
import { cn } from "@/lib/utils"
import { modKey } from "./hooks"
import { Mark, WinClose, WinMax, WinMin } from "./icons"
import { TargetIcon } from "./Remote"
import { GoalClock } from "./Todos"
import { UpdateButton } from "./Updates"
import { Tip } from "./Tip"

/** The window's title bar: drag region, session title and window controls. */
export function TitleBar() {
  const c = useClient()
  const t = c.t
  const custom = !!t && t.platform !== "web"
  const s = c.state
  const title = c.view === "settings" ? "Settings" : s?.session.title || (s ? "New session" : "")
  const inspector = c.prefs.inspector !== false
  return (
    <header data-tauri-drag-region className="flex h-(--header-height) shrink-0 items-center gap-2 border-b border-sidebar-border bg-deep pl-3">
      <SidebarTrigger className="text-muted-foreground" />
      <div data-tauri-drag-region className="flex items-center gap-2 pl-1">
        <Mark size={18} label="oah" />
        <span data-tauri-drag-region className="font-mono text-sm font-extrabold text-brand">oah</span>
      </div>
      <Separator orientation="vertical" className="mx-1 h-4!" />
      <span data-tauri-drag-region className="truncate text-[13px] font-medium">{title}</span>
      {c.target.kind !== "local" ? (
        <Tip content={`Working on ${targetLabel(c.target)}`} side="bottom"><span data-tauri-drag-region className="flex shrink-0 items-center gap-1 rounded-md border border-cyan/30 bg-cyan/10 px-1.5 py-0.5 text-[11px] font-medium text-cyan">
          <TargetIcon t={c.target} className="size-3" />
          {targetLabel(c.target)}
        </span></Tip>
      ) : null}
      {c.started && c.view === "chat" ? (
        <Tip content={c.started.cwd} side="bottom"><span data-tauri-drag-region className="truncate font-mono text-xs text-muted-foreground">
          {c.started.cwdShort}
        </span></Tip>
      ) : null}
      <div data-tauri-drag-region className="h-full flex-1" onDoubleClick={() => custom && t.win.toggleMaximize()} />
      <UpdateButton />
      {c.view === "chat" ? <GoalClock className="mr-1" /> : null}
      {c.view === "chat" ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button variant="ghost" size="icon" className={cn("size-7", inspector ? "text-primary" : "text-muted-foreground")} aria-label={inspector ? "Hide details" : "Show details"} onClick={() => c.setPref("inspector", !inspector)}>
              <PanelRight />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Details panel · {modKey}I</TooltipContent>
        </Tooltip>
      ) : null}
      {custom ? (
        <div className="ml-1 flex h-full">
          <button className="grid w-11 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Minimize" onClick={() => t.win.minimize()}><WinMin /></button>
          <button className="grid w-11 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Maximize" onClick={() => t.win.toggleMaximize()}><WinMax /></button>
          <button className="grid w-11 place-items-center text-muted-foreground hover:bg-[#c42b1c] hover:text-white" aria-label="Close" onClick={() => t.win.close()}><WinClose /></button>
        </div>
      ) : (
        <div className="w-2" />
      )}
    </header>
  )
}
