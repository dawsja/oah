import { useEffect, useState } from "react"
import { Monitor, Plus, Trash2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import { go } from "./route"
import { useMobile } from "./store"
import { IconButton, TopBar } from "./ui"

export function DesktopsScreen() {
  const desktops = useMobile((s) => s.desktops)
  const current = useMobile((s) => s.desktopId)
  const link = useMobile((s) => s.link)
  // Forgetting asks with a second tap (Android's WebView may not show confirm dialogs).
  const [armed, setArmed] = useState<string | null>(null)
  useEffect(() => {
    if (!armed) return
    const t = setTimeout(() => setArmed(null), 3000)
    return () => clearTimeout(t)
  }, [armed])

  const forget = async (id: string) => {
    if (armed !== id) return setArmed(id)
    setArmed(null)
    await useMobile.getState().forget(id)
  }

  return (
    <div className="safe flex h-full flex-col bg-background">
      <TopBar
        title="Desktops"
        backable={!!current}
      />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="divide-y border-b">
          {desktops.map((d) => {
            const on = d.id === current
            return (
              <div key={d.id} className={cn("flex items-center gap-3 pl-4", on && "bg-card")}>
                <button
                  className="flex min-w-0 flex-1 items-center gap-3 py-3.5 text-left"
                  onClick={async () => {
                    await useMobile.getState().connect(d.id)
                    go("/", true)
                  }}
                >
                  <Monitor className={cn("size-5 shrink-0", on && link === "connected" ? "text-green" : "text-muted-foreground")} />
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-[14px] font-medium">{d.name}</div>
                    <div className={cn("truncate text-[11.5px] text-muted-foreground", !d.demo && "font-mono")}>{d.demo ? "A demo inside this app" : (d.lastAddress ?? d.addresses[0])}</div>
                  </div>
                  {on ? (
                    <span className={cn("text-[12px]", link === "connected" ? "text-green" : link === "connecting" ? "text-muted-foreground" : "text-red")}>
                      {link === "connected" ? "Connected" : link === "connecting" ? "Connecting…" : "Offline"}
                    </span>
                  ) : null}
                </button>
                {armed === d.id ? (
                  <button className="rounded-md mr-2 h-10 shrink-0 border border-red bg-red/10 px-3 text-[13px] text-red" onClick={() => void forget(d.id)}>
                    Forget
                  </button>
                ) : (
                  <IconButton label={`Forget ${d.name}`} onClick={() => void forget(d.id)}>
                    <Trash2 className="size-4" />
                  </IconButton>
                )}
              </div>
            )
          })}
        </div>
        <div className="p-4">
          <Button variant="outline" className="h-11 w-full" onClick={() => go("/pair")}>
            <Plus /> Pair a desktop
          </Button>
          <p className="mt-3 text-center text-[12px] text-faint">Tap a desktop to switch to it.</p>
        </div>
      </div>
    </div>
  )
}
