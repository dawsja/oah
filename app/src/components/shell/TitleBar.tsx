import { Columns2, PanelLeft, PanelRight, Smartphone } from "lucide-react"
import { Tip } from "@/components/Tip"
import { Mark, WinClose, WinMax, WinMin } from "@/components/icons"
import { cn } from "@/lib/utils"
import { useRemote } from "@/lib/remote"
import { useApp, useProject } from "@/store/app"
import { UsageButton } from "./UsageButton"
import { TerminalButton } from "@/components/terminal/TerminalPanel"

async function win() {
  const { getCurrentWindow } = await import("@tauri-apps/api/window")
  return getCurrentWindow()
}

/** The frameless window's title bar: drag region, where you are, panel toggles, window controls. */
export function TitleBar() {
  const view = useApp((s) => s.view)
  const layout = useApp((s) => s.layout)
  const setLayout = useApp((s) => s.setLayout)
  const project = useProject()

  return (
    <header data-tauri-drag-region className="flex h-9 shrink-0 items-center bg-sidebar pl-3 text-[12px]">
      <div data-tauri-drag-region className="flex items-center gap-2 pr-3">
        <Mark size={16} label="oah" />
        <span data-tauri-drag-region className="font-mono text-[12px] font-bold tracking-tight text-brand">oah</span>
      </div>
      <Tip content="Toggle sidebar" side="bottom">
        {/* Bright while the sidebar is shown, as the side panel's toggle is. */}
        <button
          className={cn("rounded-md grid size-7 place-items-center hover:bg-accent hover:text-foreground", layout.sidebarWidth > 0 ? "text-foreground" : "text-muted-foreground")}
          aria-label="Toggle sidebar"
          aria-pressed={layout.sidebarWidth > 0}
          onClick={() => setLayout({ sidebarWidth: layout.sidebarWidth > 0 ? 0 : 272 })}
        >
          <PanelLeft className="size-3.5" />
        </button>
      </Tip>
      <div data-tauri-drag-region className="ml-2 flex min-w-0 items-center gap-2 text-muted-foreground">
        {view === "settings" ? (
          <span data-tauri-drag-region className="text-foreground">Settings</span>
        ) : project ? (
          <>
            {/* Where the project is (its system, its folder) is in the sidebar already. */}
            <span data-tauri-drag-region className="truncate font-medium text-foreground">{project.name}</span>
          </>
        ) : null}
      </div>
      <div data-tauri-drag-region className="h-full flex-1" onDoubleClick={() => win().then((w) => w.toggleMaximize())} />
      <PhoneButton />
      <UsageButton />
      <TerminalButton />
      {view === "chat" ? <SplitButton /> : null}
      {view === "chat" ? (
        <Tip content="Toggle side panel" side="bottom">
          <button
            className={cn("rounded-md mr-1 grid size-7 place-items-center hover:bg-accent", layout.rightOpen ? "text-foreground" : "text-muted-foreground")}
            aria-label="Toggle side panel"
            aria-pressed={layout.rightOpen}
            onClick={() => setLayout({ rightOpen: !layout.rightOpen })}
          >
            <PanelRight className="size-3.5" />
          </button>
        </Tip>
      ) : null}
      <div className="flex h-full">
        <button className="grid w-11 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Minimize" onClick={() => win().then((w) => w.minimize())}>
          <WinMin />
        </button>
        <button className="grid w-11 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Maximize" onClick={() => win().then((w) => w.toggleMaximize())}>
          <WinMax />
        </button>
        <button className="grid w-11 place-items-center text-muted-foreground hover:bg-[#c42b1c] hover:text-white" aria-label="Close" onClick={async () => {
            // Agents mid-turn are asked about first, then stopped with what they'd written kept.
            if (await (await import("@/lib/quit")).stopAgentsForQuit("Close")) await (await win()).close()
          }}>
          <WinClose />
        </button>
      </div>
    </header>
  )
}

/** A second chat beside this one, or back to one pane (the active pane closes). */
function SplitButton() {
  const two = useApp((s) => s.panes.length > 1)
  const project = useProject()
  if (!project) return null
  return (
    <Tip content={two ? "Close the active pane (Ctrl+Shift+\\)" : "Split: a second chat beside this one (Ctrl+\\)"} side="bottom">
      <button
        className={cn("rounded-md grid size-7 place-items-center hover:bg-accent", two ? "bg-accent text-foreground" : "text-muted-foreground")}
        aria-label={two ? "Close the active pane" : "Split into two panes"}
        aria-pressed={two}
        onClick={() => (two ? useApp.getState().closePane() : useApp.getState().splitPane())}
      >
        <Columns2 className="size-3.5" />
      </button>
    </Tip>
  )
}

/** Shown while a paired phone is connected; opens Settings → Phone. */
function PhoneButton() {
  const devices = useRemote((s) => s.state?.devices)
  const connected = devices?.filter((d) => d.connected) ?? []
  const open = useApp((s) => s.openSettings)
  if (connected.length === 0) return null
  const names = connected.map((d) => d.name).join(", ")
  return (
    <Tip content={`${names} connected`} side="bottom">
      <button className="rounded-md mr-1 grid size-7 place-items-center text-green hover:bg-accent" aria-label={`${names} connected`} onClick={() => open("phone")}>
        <Smartphone className="size-3.5" />
      </button>
    </Tip>
  )
}
