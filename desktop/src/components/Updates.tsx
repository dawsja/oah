// App update status: a title-bar button once an update is ready (or downloading),
// a toast when it's ready, and the controls in Settings.
import { ArrowUpCircle, Loader2, RefreshCw } from "lucide-react"
import { useEffect, useRef } from "react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { useClient } from "@/lib/client"
import { relativeTime } from "@/lib/format"
import type { UpdateState } from "@/lib/updates"
import { Tip } from "./Tip"

const pct = (s: Extract<UpdateState, { kind: "downloading" }>) => (s.total ? Math.min(100, Math.round((s.got / s.total) * 100)) : null)

/** In the title bar: progress while downloading, then "Restart to update". */
export function UpdateButton() {
  const c = useClient()
  const s = c.updater.state
  const told = useRef("")
  // Say so once when an update is ready.
  useEffect(() => {
    if (s.kind !== "ready" || told.current === s.version) return
    told.current = s.version
    toast(`oah ${s.version} is ready`, {
      description: s.installed ? "It starts the next time you open oah." : "Restart to install it.",
      action: { label: "Restart", onClick: () => void c.updater.restart() },
      duration: 10_000,
    })
  }, [s])
  if (s.kind === "downloading") {
    const p = pct(s)
    return (
      <Tip content={`Downloading oah ${s.version}`} side="bottom"><span className="flex items-center gap-1.5 px-1 text-xs text-muted-foreground">
        <Loader2 className="size-3.5 animate-spin" />
        Updating{p !== null ? ` ${p}%` : "…"}
      </span></Tip>
    )
  }
  if (s.kind === "installing")
    return (
      <span className="flex items-center gap-1.5 px-1 text-xs text-muted-foreground">
        <Loader2 className="size-3.5 animate-spin" />
        Installing {s.version}…
      </span>
    )
  if (s.kind !== "ready") return null
  return (
    <Tip content={<span className="whitespace-pre-line">{`oah ${s.version}${s.notes ? `\n\n${s.notes}` : ""}`}</span>} side="bottom"><Button size="sm" variant="outline" className="h-7 border-primary/40 text-xs text-primary hover:text-primary" onClick={() => void c.updater.restart()}>
      <ArrowUpCircle />
      {s.installed ? "Restart to finish update" : "Restart to update"}
    </Button></Tip>
  )
}

function status(s: UpdateState): string {
  switch (s.kind) {
    case "off":
      return "Updates are off in development builds."
    case "idle":
      return "Checks for updates in the background."
    case "checking":
      return "Checking for updates…"
    case "current":
      return `Up to date · checked ${relativeTime(s.checkedAt)}`
    case "downloading": {
      const p = pct(s)
      return `Downloading oah ${s.version}${p !== null ? ` · ${p}%` : "…"}`
    }
    case "ready":
      return s.installed ? `oah ${s.version} is installed — restart to use it` : `oah ${s.version} is ready to install`
    case "installing":
      return `Installing oah ${s.version}…`
    case "error":
      return `Couldn't update: ${s.message}`
  }
}

/** Settings → Updates: the app's own updates (the engine inside updates with it). */
export function UpdateSettings() {
  const c = useClient()
  const s = c.updater.state
  const busy = s.kind === "checking" || s.kind === "downloading" || s.kind === "installing"
  return (
    <div className="flex flex-col gap-3 rounded-lg border p-3">
      <div className="flex flex-wrap items-center gap-3">
        <div className="flex min-w-0 flex-1 flex-col">
          <span className="text-sm">Desktop app · {c.info?.version}</span>
          <span className={s.kind === "error" ? "text-xs text-red" : "text-xs text-muted-foreground"}>{status(s)}</span>
        </div>
        {s.kind === "ready" ? (
          <Button size="sm" onClick={() => void c.updater.restart()}>
            <ArrowUpCircle />
            {s.installed ? "Restart now" : "Restart to update"}
          </Button>
        ) : (
          <Button size="sm" variant="outline" disabled={!c.updater.enabled || busy} onClick={() => void c.updater.check(true)}>
            {busy ? <Loader2 className="animate-spin" /> : <RefreshCw />}
            Check for updates
          </Button>
        )}
      </div>
      <div className="flex items-center justify-between gap-4 border-t pt-3">
        <Label htmlFor="autoupdate" className="flex flex-col items-start gap-0.5 font-normal">
          <span className="font-medium">Update automatically</span>
          <span className="text-sm text-muted-foreground">Download new versions in the background{c.info?.bundle === "app" || c.info?.bundle === "appimage" ? " and install them" : " and install when you restart"}.</span>
        </Label>
        <Switch id="autoupdate" checked={c.prefs.autoUpdate !== false} disabled={!c.updater.enabled} onCheckedChange={(v) => c.setPref("autoUpdate", v)} />
      </div>
    </div>
  )
}
