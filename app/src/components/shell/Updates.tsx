// The title bar's update button: progress while downloading, then "Restart to update".
import { useEffect, useRef } from "react"
import { ArrowUpCircle, Loader2 } from "lucide-react"
import { toast } from "sonner"
import { Tip } from "@/components/Tip"
import { updater, useUpdates } from "@/lib/updates"

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
  if (s.kind === "downloading") {
    const p = s.total ? Math.min(100, Math.round((s.got / s.total) * 100)) : null
    return (
      <span className="mr-1 flex items-center gap-1.5 px-1 text-[11.5px] text-muted-foreground">
        <Loader2 className="size-3.5 animate-spin" /> Updating{p !== null ? ` ${p}%` : "…"}
      </span>
    )
  }
  if (s.kind === "installing")
    return (
      <span className="mr-1 flex items-center gap-1.5 px-1 text-[11.5px] text-muted-foreground">
        <Loader2 className="size-3.5 animate-spin" /> Installing {s.version}…
      </span>
    )
  if (s.kind !== "ready") return null
  return (
    <Tip content={<span className="whitespace-pre-line">{`oah ${s.version}${s.notes ? `\n\n${s.notes}` : ""}`}</span>} side="bottom">
      <button className="mr-1 flex h-7 items-center gap-1.5 rounded-md bg-primary px-2 text-[11.5px] font-medium text-primary-foreground hover:bg-primary/90" onClick={() => void updater.restart()}>
        <ArrowUpCircle className="size-3.5" />
        {s.installed ? "Restart to finish update" : "Restart to update"}
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
