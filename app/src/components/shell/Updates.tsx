// The update control at the bottom of the sidebar: progress while downloading, then "Restart to update".
import { useEffect, useRef } from "react"
import { ArrowUpCircle, Loader2 } from "lucide-react"
import { toast } from "sonner"
import { Tip } from "@/components/Tip"
import { updater, useUpdates } from "@/lib/updates"
import { cn } from "@/lib/utils"

/** The update, at the bottom of the sidebar above Settings: progress while it downloads and
 *  installs, then "Restart to update". Nothing while there's no update. */
export function UpdateButton() {
  const s = useUpdates((u) => u.state)
  const told = useRef("")
  useEffect(() => {
    if (s.kind !== "ready" || told.current === s.version) return
    told.current = s.version
    toast(`oah ${s.version} is ready`, {
      description: s.installed ? "It starts the next time you open oah." : "Restart to install it.",
      action: { label: "Restart", onClick: () => void updater.restart() },
      duration: 10_000,
    })
  }, [s])
  if (s.kind === "downloading" || s.kind === "installing") {
    const p = s.kind === "downloading" && s.total ? Math.min(100, Math.round((s.got / s.total) * 100)) : null
    return (
      <div className="px-2.5 py-1.5 text-[12px] text-muted-foreground">
        <div className="flex items-center gap-2">
          <Loader2 className="size-3.5 animate-spin" />
          {s.kind === "installing" ? `Installing ${s.version}…` : `Updating to ${s.version}`}
          {p !== null ? <span className="ml-auto font-mono text-[10.5px] text-faint">{p}%</span> : null}
        </div>
        {s.kind === "downloading" ? (
          <div className="rounded-full mt-1.5 h-1 overflow-hidden bg-muted">
            <div className={cn("rounded-full h-full bg-primary transition-[width]", p === null && "w-1/3 animate-pulse")} style={p !== null ? { width: `${p}%` } : undefined} />
          </div>
        ) : null}
      </div>
    )
  }
  if (s.kind !== "ready") return null
  // Releases so far say only "oah <version>": no news under the same heading.
  const notes = s.notes?.trim() && s.notes.trim() !== `oah ${s.version}` ? s.notes.trim() : null
  return (
    <Tip content={<span className="whitespace-pre-line">{`oah ${s.version}${notes ? `\n\n${notes}` : ""}`}</span>} side="right">
      <button
        className="rounded-md flex h-8 w-full items-center gap-2 bg-primary px-2.5 text-[12.5px] font-medium text-primary-foreground hover:bg-primary/90"
        onClick={() => void updater.restart()}
      >
        <ArrowUpCircle className="size-3.5" />
        {s.installed ? "Restart to finish update" : "Restart to update"}
        <span className="ml-auto font-mono text-[10.5px] opacity-70">{s.version}</span>
      </button>
    </Tip>
  )
}

/** One line for the Settings footer. */
export function updateStatus(): string {
  const s = useUpdates.getState().state
  switch (s.kind) {
    case "off":
      return "Updates are off in development builds"
    case "checking":
      return "Checking for updates…"
    case "current":
      return "Up to date"
    case "downloading":
      return `Downloading ${s.version}…`
    case "ready":
      return `${s.version} is ready`
    case "installing":
      return `Installing ${s.version}…`
    case "error":
      return `Couldn't update: ${s.message}`
    default:
      return "Checks for updates in the background"
  }
}
