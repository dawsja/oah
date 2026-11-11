// Small pieces the phone screens share: the top bar and the link status.

import type { ReactNode } from "react"
import { ChevronLeft, Loader2 } from "lucide-react"
import { cn } from "@/lib/utils"
import { native, needsLocalNetwork } from "./rpc"
import { back } from "./route"
import { useMobile } from "./store"

/** A screen's top bar: back (when `onBack` or `backable`), title and subtitle, then actions. */
export function TopBar({ title, subtitle, backable, actions, className }: { title: ReactNode; subtitle?: ReactNode; backable?: boolean; actions?: ReactNode; className?: string }) {
  return (
    <header className={cn("flex h-14 shrink-0 items-center gap-1 border-b bg-background px-1.5", className)}>
      {backable ? (
        <button className="rounded-md grid size-11 shrink-0 place-items-center text-muted-foreground active:bg-accent" aria-label="Back" onClick={back}>
          <ChevronLeft className="size-5" />
        </button>
      ) : (
        <span className="w-2.5" />
      )}
      <div className="min-w-0 flex-1">
        <div className="truncate text-[15px] font-semibold">{title}</div>
        {subtitle ? <div className="truncate text-[11.5px] text-muted-foreground">{subtitle}</div> : null}
      </div>
      {actions ? <div className="flex shrink-0 items-center">{actions}</div> : null}
    </header>
  )
}

/** A square icon button sized for a thumb. */
export function IconButton({ label, onClick, children, className, disabled }: { label: string; onClick?: () => void; children: ReactNode; className?: string; disabled?: boolean }) {
  return (
    <button className={cn("rounded-md grid size-11 place-items-center text-muted-foreground active:bg-accent disabled:opacity-40", className)} aria-label={label} onClick={onClick} disabled={disabled}>
      {children}
    </button>
  )
}

/** The link to the desktop, when it isn't up: connecting, offline (with why), or removed. */
export function LinkBanner() {
  const link = useMobile((s) => s.link)
  const error = useMobile((s) => s.linkError)
  const name = useMobile((s) => s.desktopName)
  if (link === "connected" || link === "idle") return null
  const text =
    link === "connecting"
      ? `Connecting to ${name || "the desktop"}…`
      : link === "revoked"
        ? "This phone was removed on the desktop. Pair it again."
        : `Can't reach ${name || "the desktop"}${error ? `: ${error}` : ""}`
  const blocked = link === "offline" && needsLocalNetwork(error)
  return (
    <div className={cn("flex shrink-0 items-center gap-2 border-b px-4 py-1.5 text-[12.5px]", link === "connecting" ? "text-muted-foreground" : "bg-red/5 text-red")} role="status">
      {link === "connecting" ? <Loader2 className="size-3.5 shrink-0 animate-spin" /> : null}
      <span className="line-clamp-3 min-w-0 flex-1 py-1 leading-snug">{blocked ? `${text}. If oah isn't allowed on the local network, turn it on in Settings.` : text}</span>
      {blocked ? (
        <button className="rounded-md h-10 shrink-0 border border-red/40 px-3 font-medium active:bg-red/10" onClick={() => void native.openSettings()}>
          Settings
        </button>
      ) : null}
      {link === "offline" ? (
        <button className="rounded-md h-10 shrink-0 border border-red/40 px-3 font-medium active:bg-red/10" onClick={() => void native.wake()}>
          Retry
        </button>
      ) : null}
    </div>
  )
}

/** "now", "5m", "3h", "2d", or the date. */
export function ago(ms: number, now = Date.now()): string {
  const s = Math.max(0, (now - ms) / 1000)
  if (s < 60) return "now"
  if (s < 3600) return `${Math.floor(s / 60)}m`
  if (s < 86400) return `${Math.floor(s / 3600)}h`
  if (s < 7 * 86400) return `${Math.floor(s / 86400)}d`
  return new Date(ms).toLocaleDateString()
}
