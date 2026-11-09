import { useEffect, useMemo, useRef, useState, type MouseEvent, type PointerEvent } from "react"
import { Loader2, MessageSquarePlus, MonitorSmartphone, Moon, Pencil, Pin, PinOff, Plus, Settings, Trash2 } from "lucide-react"
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import { LocationIcon } from "@/components/LocationIcon"
import { AGENT_LABEL, type Project } from "@/lib/types"
import type { ChatMeta } from "@/lib/chat"
import { cn } from "@/lib/utils"
import { go } from "./route"
import { useMobile } from "./store"
import { IconButton, LinkBanner, TopBar, ago } from "./ui"

/** The folder a chat works in, when it's a worktree rather than the project itself. */
function worktreeOf(meta: ChatMeta, project: Project | undefined): string | null {
  if (!project || !meta.cwd || meta.cwd.replace(/[\\/]+$/, "") === project.path.replace(/[\\/]+$/, "")) return null
  return meta.cwd.split(/[\\/]/).filter(Boolean).pop() ?? null
}

const WAITING_LABEL = { approval: "Needs approval", question: "Question", plan: "Plan to approve" } as const

/** Press and hold: `onLong` after a moment, unless the finger moves (a scroll) or lifts first.
 *  The tap that ends a long press doesn't also count as a click. */
function useLongPress(onLong: () => void, ms = 450) {
  const timer = useRef(0)
  const from = useRef<{ x: number; y: number } | null>(null)
  const fired = useRef(false)
  const cancel = () => {
    clearTimeout(timer.current)
    from.current = null
  }
  useEffect(() => cancel, [])
  return {
    onPointerDown: (e: PointerEvent) => {
      fired.current = false
      from.current = { x: e.clientX, y: e.clientY }
      timer.current = window.setTimeout(() => {
        fired.current = true
        from.current = null
        navigator.vibrate?.(10)
        onLong()
      }, ms)
    },
    onPointerMove: (e: PointerEvent) => {
      if (from.current && Math.hypot(e.clientX - from.current.x, e.clientY - from.current.y) > 10) cancel()
    },
    onPointerUp: cancel,
    onPointerCancel: cancel,
    onPointerLeave: cancel,
    // Android's own long-press menu (and text selection) would get in the way.
    onContextMenu: (e: MouseEvent) => e.preventDefault(),
    onClickCapture: (e: MouseEvent) => {
      if (!fired.current) return
      fired.current = false
      e.preventDefault()
      e.stopPropagation()
    },
  }
}

function ChatRow({ meta, project, onMenu }: { meta: ChatMeta; project: Project | undefined; onMenu: (meta: ChatMeta) => void }) {
  const busy = useMobile((s) => s.busy.includes(meta.id))
  const waiting = useMobile((s) => s.waiting[meta.id])
  const worktree = worktreeOf(meta, project)
  const press = useLongPress(() => onMenu(meta))
  return (
    <button
      className="flex min-h-16 w-full items-center gap-3 px-4 py-3 text-left select-none [-webkit-touch-callout:none] active:bg-accent"
      onClick={() => go(`/chat/${encodeURIComponent(meta.id)}`)}
      {...press}
    >
      <div className="min-w-0 flex-1">
        <div className={cn("line-clamp-2 text-[14.5px] leading-snug", !meta.title && "text-muted-foreground")}>{meta.title || "New chat"}</div>
        <div className="mt-0.5 flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
          {meta.pinned ? <Pin className="size-3 shrink-0" aria-label="Pinned" /> : null}
          {meta.sleeping ? <Moon className="size-3 shrink-0" aria-label="Sleeping" /> : null}
          <span>{AGENT_LABEL[meta.agent]}</span>
          {worktree ? <span className="rounded-sm min-w-0 truncate bg-muted px-1.5 font-mono text-[11.5px]">{worktree}</span> : null}
        </div>
      </div>
      {waiting ? (
        <span className="rounded-full shrink-0 bg-primary px-2.5 py-1 text-[11.5px] font-medium text-primary-foreground">{WAITING_LABEL[waiting]}</span>
      ) : busy ? (
        <Loader2 className="size-4 shrink-0 animate-spin text-primary" aria-label="Working" />
      ) : (
        <span className="shrink-0 text-[12px] text-faint">{ago(meta.updatedAt)}</span>
      )}
    </button>
  )
}

/** What holding a chat opens: the desktop's right-click menu (pin, sleep, rename, delete). */
function ChatMenu({ meta, onClose }: { meta: ChatMeta | null; onClose: () => void }) {
  const busy = useMobile((s) => (meta ? s.busy.includes(meta.id) : false))
  const waiting = useMobile((s) => (meta ? !!s.waiting[meta.id] : false))
  const connected = useMobile((s) => s.link === "connected")
  const [renaming, setRenaming] = useState(false)
  const [title, setTitle] = useState("")
  // Deleting asks with a second tap (Android's WebView may not show confirm dialogs).
  const [armed, setArmed] = useState(false)
  useEffect(() => {
    setRenaming(false)
    setArmed(false)
    setTitle(meta?.title ?? "")
  }, [meta?.id])
  useEffect(() => {
    if (!armed) return
    const t = setTimeout(() => setArmed(false), 3000)
    return () => clearTimeout(t)
  }, [armed])

  const st = useMobile.getState()
  const sleepBlocked = !meta ? null : meta.sleeping ? "Already asleep" : busy ? "Working" : waiting ? "Waiting for you" : !meta.agentSessionId ? "Nothing to resume yet" : null
  const item = "flex h-12 w-full items-center gap-3 px-4 text-left text-[14.5px] active:bg-accent disabled:opacity-40"
  const save = () => {
    if (meta && title.trim()) void st.rename(meta.id, title.trim())
    onClose()
  }

  return (
    <Sheet open={!!meta} onOpenChange={(o) => !o && onClose()}>
      <SheetContent side="bottom" onOpenAutoFocus={(e) => e.preventDefault()} className="gap-0 rounded-t-2xl pb-[max(0.75rem,env(safe-area-inset-bottom))]">
        <SheetHeader className="pb-2">
          <SheetTitle className="line-clamp-2 pr-8 leading-snug">{meta?.title || "New chat"}</SheetTitle>
          <SheetDescription>{meta ? AGENT_LABEL[meta.agent] : ""}</SheetDescription>
        </SheetHeader>
        {!meta ? null : renaming ? (
          <form
            className="flex gap-2 px-4 pb-2"
            onSubmit={(e) => {
              e.preventDefault()
              save()
            }}
          >
            <input
              autoFocus
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              aria-label="Chat name"
              className="rounded-md h-11 min-w-0 flex-1 border border-input bg-transparent px-3 text-[15px] outline-none focus:border-ring"
            />
            <button type="submit" disabled={!title.trim()} className="rounded-md h-11 shrink-0 bg-primary px-4 text-[14px] font-medium text-primary-foreground disabled:opacity-50">
              Save
            </button>
          </form>
        ) : (
          <div className="flex flex-col">
            <button className={item} disabled={!connected} onClick={() => (void st.pin(meta.id, !meta.pinned), onClose())}>
              {meta.pinned ? <PinOff className="size-5 text-muted-foreground" /> : <Pin className="size-5 text-muted-foreground" />}
              {meta.pinned ? "Unpin" : "Pin"}
            </button>
            <button className={item} disabled={!connected || !!sleepBlocked} onClick={() => (void st.sleep(meta.id), onClose())}>
              <Moon className="size-5 text-muted-foreground" />
              <span className="flex-1">Sleep</span>
              {sleepBlocked ? <span className="text-[12px] text-muted-foreground">{sleepBlocked}</span> : null}
            </button>
            <button className={item} disabled={!connected} onClick={() => setRenaming(true)}>
              <Pencil className="size-5 text-muted-foreground" /> Rename…
            </button>
            <div className="mx-4 my-1 border-t" />
            <button
              className={cn(item, "text-red", armed && "bg-red/10")}
              disabled={!connected}
              onClick={() => {
                if (!armed) return setArmed(true)
                void st.remove(meta.id)
                onClose()
              }}
            >
              <Trash2 className="size-5" /> {armed ? "Tap again to delete" : "Delete chat"}
            </button>
          </div>
        )}
      </SheetContent>
    </Sheet>
  )
}

export function ChatsScreen() {
  const name = useMobile((s) => s.desktopName)
  const link = useMobile((s) => s.link)
  const projects = useMobile((s) => s.projects)
  const metas = useMobile((s) => s.metas)
  const waiting = useMobile((s) => s.waiting)
  const loadedOnce = useMobile((s) => s.projects.length > 0 || s.metas.length > 0)
  const [menu, setMenu] = useState<ChatMeta | null>(null)

  // Projects with something waiting for you first, then recent ones; in each, the chats that
  // wait for you first, then pinned ones, then newest first.
  const groups = useMemo(() => {
    const by = new Map<string, ChatMeta[]>()
    for (const m of metas) by.set(m.projectId, [...(by.get(m.projectId) ?? []), m])
    for (const list of by.values()) list.sort((a, b) => Number(!!waiting[b.id]) - Number(!!waiting[a.id]) || Number(b.pinned) - Number(a.pinned))
    const rank = (p: Project) => (by.get(p.id)?.some((m) => waiting[m.id]) ? 1e15 : 0) + (by.get(p.id)?.[0]?.updatedAt ?? 0)
    return [...projects].sort((a, b) => rank(b) - rank(a)).map((p) => ({ project: p, chats: by.get(p.id) ?? [] }))
  }, [projects, metas, waiting])

  return (
    <div className="safe flex h-full flex-col bg-background">
      <TopBar
        title={name || "oah"}
        subtitle={
          <span className={cn(link === "connected" ? "text-green" : link === "offline" || link === "revoked" ? "text-red" : "text-muted-foreground")}>
            {link === "connected" ? "Connected" : link === "connecting" ? "Connecting…" : link === "revoked" ? "Removed on the desktop" : "Offline · showing the last list"}
          </span>
        }
        actions={
          <>
            <IconButton label="Desktops" onClick={() => go("/desktops")}>
              <MonitorSmartphone className="size-5" />
            </IconButton>
            <IconButton label="Settings" onClick={() => go("/settings")}>
              <Settings className="size-5" />
            </IconButton>
          </>
        }
      />
      <LinkBanner />
      <div className="min-h-0 flex-1 overflow-y-auto pb-6">
        {!loadedOnce ? (
          <div className="flex h-48 flex-col items-center justify-center gap-2 px-8 text-center text-[13px] text-muted-foreground">
            <Loader2 className="size-5 animate-spin" />
            {link === "connected" ? "Loading…" : "Waiting for the desktop. It has to be on, with oah open, on the same Wi-Fi or VPN as this phone."}
          </div>
        ) : groups.length === 0 ? (
          <div className="px-6 py-16 text-center text-[13px] text-muted-foreground">No projects on this desktop yet. Open a folder in oah there first.</div>
        ) : (
          groups.map(({ project, chats }) => (
            <section key={project.id} className="mt-3">
              <div className="flex items-center gap-2 pr-1.5 pl-4">
                <LocationIcon location={project.location} className="size-3.5 shrink-0 text-faint" />
                <h2 className="min-w-0 flex-1 truncate text-[11.5px] font-medium tracking-wide text-muted-foreground uppercase">{project.name}</h2>
                <IconButton label={`New chat in ${project.name}`} onClick={() => go(`/new/${encodeURIComponent(project.id)}`)} disabled={link !== "connected"}>
                  <Plus className="size-4" />
                </IconButton>
              </div>
              <div className="divide-y border-y bg-card">
                {chats.length ? (
                  chats.slice(0, 30).map((m) => <ChatRow key={m.id} meta={m} project={project} onMenu={setMenu} />)
                ) : (
                  <button className="flex w-full items-center gap-2 px-4 py-3 text-[13px] text-muted-foreground active:bg-accent" onClick={() => go(`/new/${encodeURIComponent(project.id)}`)}>
                    <MessageSquarePlus className="size-4" /> Start a chat
                  </button>
                )}
              </div>
            </section>
          ))
        )}
      </div>
      {/* The newest copy of the chat, so pin and sleep show what they are now. */}
      <ChatMenu meta={menu ? (metas.find((m) => m.id === menu.id) ?? null) : null} onClose={() => setMenu(null)} />
    </div>
  )
}
