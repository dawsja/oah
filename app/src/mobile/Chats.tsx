import { useMemo } from "react"
import { Loader2, MessageSquarePlus, MonitorSmartphone, Plus } from "lucide-react"
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

function ChatRow({ meta, project }: { meta: ChatMeta; project: Project | undefined }) {
  const busy = useMobile((s) => s.busy.includes(meta.id))
  const waiting = useMobile((s) => s.waiting[meta.id])
  const worktree = worktreeOf(meta, project)
  return (
    <button className="flex min-h-16 w-full items-center gap-3 px-4 py-3 text-left active:bg-accent" onClick={() => go(`/chat/${encodeURIComponent(meta.id)}`)}>
      <div className="min-w-0 flex-1">
        <div className={cn("line-clamp-2 text-[14.5px] leading-snug", !meta.title && "text-muted-foreground")}>{meta.title || "New chat"}</div>
        <div className="mt-0.5 flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
          <span>{AGENT_LABEL[meta.agent]}</span>
          {worktree ? <span className="rounded-sm min-w-0 truncate bg-muted px-1.5 font-mono text-[11.5px]">{worktree}</span> : null}
        </div>
      </div>
      {waiting ? (
        <span className="rounded-sm shrink-0 bg-primary px-2 py-1 text-[11.5px] font-medium text-primary-foreground">{WAITING_LABEL[waiting]}</span>
      ) : busy ? (
        <Loader2 className="size-4 shrink-0 animate-spin text-primary" aria-label="Working" />
      ) : (
        <span className="shrink-0 text-[12px] text-faint">{ago(meta.updatedAt)}</span>
      )}
    </button>
  )
}

export function ChatsScreen() {
  const name = useMobile((s) => s.desktopName)
  const link = useMobile((s) => s.link)
  const projects = useMobile((s) => s.projects)
  const metas = useMobile((s) => s.metas)
  const waiting = useMobile((s) => s.waiting)
  const loadedOnce = useMobile((s) => s.projects.length > 0 || s.metas.length > 0)

  // Projects with something waiting for you first, then recent ones; in each, the chats that
  // wait for you first, then newest first.
  const groups = useMemo(() => {
    const by = new Map<string, ChatMeta[]>()
    for (const m of metas) by.set(m.projectId, [...(by.get(m.projectId) ?? []), m])
    for (const list of by.values()) list.sort((a, b) => Number(!!waiting[b.id]) - Number(!!waiting[a.id]))
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
          <IconButton label="Desktops" onClick={() => go("/desktops")}>
            <MonitorSmartphone className="size-5" />
          </IconButton>
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
                  chats.slice(0, 30).map((m) => <ChatRow key={m.id} meta={m} project={project} />)
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
    </div>
  )
}
