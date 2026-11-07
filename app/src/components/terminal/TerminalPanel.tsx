import "@xterm/xterm/css/xterm.css"
import { useEffect, useLayoutEffect, useRef } from "react"
import { ChevronDown, Plus, SquareTerminal, X } from "lucide-react"
import { Tip } from "@/components/Tip"
import { Resizer } from "@/components/shell/Resizer"
import { baseName, locationKey, shellLabel } from "@/lib/terminal"
import { isRemote, locationLabel, type Location, type Project } from "@/lib/types"
import { cn } from "@/lib/utils"
import { useApp, useProject } from "@/store/app"
import { currentScope } from "@/store/git"
import { attachTerminal, detachTerminal, fitTab, focusTerminal, learnLocalShell, useTerminals, type TermTab } from "@/store/terminals"

/** The shell a terminal in `project` runs (or ran last), when known. */
export function useProjectShell(project: Project | null): string | null {
  const active = useTerminals((s) => (project ? s.tabs.find((t) => t.id === s.active[project.id]) : undefined))
  const known = useTerminals((s) => (project ? s.shells[locationKey(project.location)] : undefined))
  useEffect(() => {
    if (project?.location.kind === "local" && !known) learnLocalShell(project.id)
  }, [project?.id, project?.location.kind, known])
  return active?.shell ?? known ?? null
}

/** Where a shell runs, in words: "bash in Ubuntu", "zsh on devbox", "PowerShell". */
export function shellWhere(shell: string | null, location: Location): string {
  const name = shellLabel(shell)
  return location.kind === "wsl" ? `${name} in ${location.distro}` : location.kind === "ssh" ? `${name} on ${location.host}` : name
}

/** The title bar's terminal button: shows and hides the panel. */
export function TerminalButton() {
  const project = useProject()
  const view = useApp((s) => s.view)
  const open = useApp((s) => s.layout.terminalOpen)
  const has = useTerminals((s) => (project ? s.tabs.some((t) => t.projectId === project.id) : false))
  const shell = useProjectShell(project)
  if (!project || view !== "chat") return null
  const shown = open && has
  const tip = shown ? "Hide terminal (Ctrl+`)" : `${has ? "Show terminal" : "Open a terminal"}: ${shellWhere(shell, project.location)} (Ctrl+\`)`
  return (
    <Tip content={tip} side="bottom">
      <button
        className={cn("rounded-md grid size-7 place-items-center hover:bg-accent", shown ? "bg-accent text-foreground" : "text-muted-foreground")}
        aria-label={shown ? "Hide terminal" : "Open terminal"}
        aria-pressed={shown}
        onClick={() => useTerminals.getState().toggle()}
      >
        <SquareTerminal className="size-3.5" />
      </button>
    </Tip>
  )
}

/** The terminals under the chat: tabs for the project on screen, each a shell where the project
 *  is. Every terminal stays mounted (hidden when it's not on screen), so a shell keeps running
 *  and keeps its scrollback while the panel is hidden or another project is open. */
export function TerminalPanel() {
  const project = useProject()
  const view = useApp((s) => s.view)
  const open = useApp((s) => s.layout.terminalOpen)
  const height = useApp((s) => s.layout.terminalHeight)
  const setLayout = useApp((s) => s.setLayout)
  const tabs = useTerminals((s) => s.tabs)
  const activeId = useTerminals((s) => (project ? s.active[project.id] : undefined))
  const projects = useApp((s) => s.settings?.projects)
  const mine = project ? tabs.filter((t) => t.projectId === project.id) : []
  const visible = view === "chat" && open && mine.length > 0

  // A removed project's terminals end with it.
  useEffect(() => {
    if (!projects) return
    for (const t of useTerminals.getState().tabs) if (!projects.some((p) => p.id === t.projectId)) useTerminals.getState().close(t.id)
  }, [projects])

  if (!tabs.length) return null
  return (
    <div className={cn("flex shrink-0 flex-col", !visible && "hidden")} style={{ height: visible ? height : undefined }}>
      <Resizer side="top" width={height} min={120} max={Math.max(160, window.innerHeight - 220)} onWidth={(h) => setLayout({ terminalHeight: h })} />
      <div className="flex min-h-0 flex-1 flex-col border-t bg-sidebar">
        <div className="flex h-8 shrink-0 items-stretch border-b" role="tablist" aria-label="Terminals">
          <div className="flex min-w-0 items-stretch overflow-x-auto">
            {mine.map((t) => (
              <TabButton key={t.id} tab={t} project={project!} active={t.id === activeId} />
            ))}
          </div>
          <Tip content="New terminal" side="top">
            <button
              className="grid w-8 shrink-0 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground"
              aria-label="New terminal"
              onClick={() => {
                const scope = currentScope()
                if (scope) useTerminals.getState().add(scope.projectId, scope.cwd)
              }}
            >
              <Plus className="size-3.5" />
            </button>
          </Tip>
          <div className="flex-1" />
          <Tip content="Hide terminal (Ctrl+`)" side="top">
            <button className="grid w-8 shrink-0 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Hide terminal" onClick={() => setLayout({ terminalOpen: false })}>
              <ChevronDown className="size-3.5" />
            </button>
          </Tip>
        </div>
        <div className="relative min-h-0 flex-1">
          {tabs.map((t) => (
            <TermHost key={t.id} id={t.id} shown={visible && t.id === activeId} />
          ))}
        </div>
      </div>
    </div>
  )
}

function TabButton({ tab, project, active }: { tab: TermTab; project: Project; active: boolean }) {
  const where = tab.cwd && tab.cwd !== project.path ? baseName(tab.cwd) : isRemote(project.location) ? locationLabel(project.location) : null
  const tip = `${shellWhere(tab.shell, project.location)}, in ${tab.cwd ?? project.path}${tab.status === "exited" ? " (ended)" : ""}`
  return (
    <Tip content={tip} side="top">
      <div
        role="tab"
        aria-selected={active}
        tabIndex={0}
        className={cn(
          "group relative flex max-w-56 shrink-0 cursor-default items-center gap-1.5 border-r pr-1 pl-2.5 text-[12px] text-muted-foreground hover:bg-accent hover:text-foreground",
          active && "bg-background text-foreground after:absolute after:inset-x-0 after:bottom-0 after:h-0.5 after:bg-primary",
        )}
        onClick={() => useTerminals.getState().select(project.id, tab.id)}
        onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && useTerminals.getState().select(project.id, tab.id)}
        onAuxClick={(e) => e.button === 1 && useTerminals.getState().close(tab.id)}
      >
        <SquareTerminal className={cn("size-3.5 shrink-0", tab.status === "starting" && "animate-pulse", tab.status === "exited" && "opacity-50")} />
        <span className={cn("truncate", tab.status === "exited" && "text-faint")}>{shellLabel(tab.shell)}</span>
        {where ? <span className="truncate font-mono text-[11px] text-faint">{where}</span> : null}
        <button
          className={cn("rounded-md ml-0.5 grid size-5 shrink-0 place-items-center text-faint hover:bg-accent hover:text-foreground", !active && "opacity-0 group-hover:opacity-100 focus-visible:opacity-100")}
          aria-label="Close terminal"
          onClick={(e) => {
            e.stopPropagation()
            useTerminals.getState().close(tab.id)
          }}
        >
          <X className="size-3" />
        </button>
      </div>
    </Tip>
  )
}

/** Where one terminal draws. It's created the first time it's on screen, then kept. */
function TermHost({ id, shown }: { id: string; shown: boolean }) {
  const ref = useRef<HTMLDivElement>(null)

  useLayoutEffect(() => {
    const host = ref.current
    if (!host || !shown) return
    attachTerminal(id, host)
    fitTab(id)
    focusTerminal(id)
    const ro = new ResizeObserver(() => fitTab(id))
    ro.observe(host)
    return () => {
      ro.disconnect()
      detachTerminal(id, host)
    }
  }, [id, shown])

  return <div ref={ref} className={cn("absolute inset-0 py-1 pl-2", !shown && "hidden")} />
}
