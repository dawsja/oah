import { ChevronRight, Folder, FolderOpen, Plus, Settings2 } from "lucide-react"
import { useMemo, useState } from "react"
import { Button } from "@/components/ui/button"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { Kbd } from "@/components/ui/kbd"
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarInput,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { useClient } from "@/lib/client"
import { basename, modelSlug, relativeTime } from "@/lib/format"
import type { SessionMeta } from "@/lib/protocol"
import { cn } from "@/lib/utils"
import { modKey } from "./hooks"

const PER_GROUP = 8

/** Saved sessions grouped by project folder, newest first — the same files the CLI's /session lists. */
export function AppSidebar() {
  const c = useClient()
  const [q, setQ] = useState("")
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const s = c.state
  const activeId = s?.session.id
  const collapsed = new Set(c.prefs.collapsedGroups ?? [])

  const groups = useMemo(() => {
    let list: SessionMeta[] = c.sessions.filter((x) => x.entries > 0 || x.id === activeId)
    // The open conversation isn't on disk until it has messages: show it anyway.
    if (s && c.started && !list.some((x) => x.id === s.session.id))
      list = [{ id: s.session.id, title: s.session.title, cwd: c.started.cwd, cwdShort: c.started.cwdShort, created: s.session.created, updated: Date.now(), model: s.settings.model, mode: s.settings.mode, entries: 0, hasGoal: false, detailed: true }, ...list]
    const words = q.toLowerCase().split(/\s+/).filter(Boolean)
    if (words.length) list = list.filter((x) => words.every((w) => `${x.title} ${x.cwd} ${x.id}`.toLowerCase().includes(w)))
    const byCwd = new Map<string, SessionMeta[]>()
    for (const x of list) byCwd.set(x.cwd, [...(byCwd.get(x.cwd) ?? []), x])
    return [...byCwd.entries()].map(([cwd, items]) => ({ cwd, name: basename(cwd) || cwd, short: items[0]!.cwdShort, items }))
  }, [c.sessions, activeId, s?.session.title, c.started, q])

  const setOpen = (cwd: string, open: boolean) => c.setPref("collapsedGroups", open ? [...collapsed].filter((x) => x !== cwd) : [...collapsed, cwd])
  const providers = s?.auth.available.length ?? 0
  const mcpReady = s?.mcp.filter((m) => m.state === "ready").length ?? 0
  const mcpAuth = s?.mcp.some((m) => m.state === "needs-auth")

  return (
    <Sidebar className="top-(--header-height) h-[calc(100svh-var(--header-height))]!">
      <SidebarHeader className="gap-2.5 px-3 pt-3">
        <div className="flex gap-2">
          <Button className="h-9 flex-1 justify-start gap-2 font-semibold" onClick={() => c.newSession()}>
            <Plus />
            New session
            <Kbd className="ml-auto bg-primary-foreground/10 text-primary-foreground/70">{modKey}N</Kbd>
          </Button>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="outline" size="icon" className="size-9 bg-transparent" aria-label="New session in another folder" onClick={() => void c.openFolder()}>
                <FolderOpen />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Open a folder · {modKey}O</TooltipContent>
          </Tooltip>
        </div>
        <SidebarInput placeholder="Search sessions" aria-label="Search sessions" value={q} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.key === "Escape" && setQ("")} className="h-8 bg-card" />
      </SidebarHeader>

      <SidebarContent className="gap-0">
        {groups.length === 0 ? <p className="px-4 py-3 text-[13px] text-muted-foreground">{q ? "No sessions match." : "No saved sessions yet."}</p> : null}
        {groups.map((g) => {
          const open = !collapsed.has(g.cwd) || !!q
          const showAll = expanded.has(g.cwd) || !!q
          const items = showAll ? g.items : g.items.slice(0, PER_GROUP)
          return (
            <Collapsible key={g.cwd} open={open} onOpenChange={(o) => setOpen(g.cwd, o)} className="group/collapsible">
              <SidebarGroup className="py-1">
                <SidebarGroupLabel asChild className="font-mono text-muted-foreground hover:bg-sidebar-accent hover:text-sidebar-foreground">
                  <CollapsibleTrigger title={g.short}>
                    <Folder className="mr-1.5" />
                    <span className="truncate">{g.name}</span>
                    <span className="ml-auto pr-1 font-sans text-faint">{g.items.length}</span>
                    <ChevronRight className="transition-transform group-data-[state=open]/collapsible:rotate-90" />
                  </CollapsibleTrigger>
                </SidebarGroupLabel>
                <CollapsibleContent>
                  <SidebarGroupContent>
                    <SidebarMenu>
                      {items.map((x) => {
                        const active = x.id === activeId
                        const live = active && !!s?.busy
                        return (
                          <SidebarMenuItem key={x.id}>
                            <SidebarMenuButton size="lg" isActive={active} onClick={() => c.openSession(x)} title={`${x.title || "(untitled)"}\n${x.cwdShort} · ${x.id}`} className="h-auto flex-col items-start gap-0.5 py-2">
                              <span className="flex w-full items-center gap-2">
                                <span className="truncate">{x.title || (active ? "New session" : "(untitled)")}</span>
                                {active ? <span className={cn("ml-auto size-1.5 shrink-0 rounded-full bg-primary", live && "animate-pulse")} /> : null}
                              </span>
                              <span className="w-full truncate text-xs font-normal text-muted-foreground">
                                {active && s ? (
                                  <>
                                    {s.model.name} · {s.settings.mode === "plan" ? <span className="text-cyan">plan</span> : "build"}
                                    {s.session.goal && !s.session.goal.done ? " · goal" : ""} ·{" "}
                                  </>
                                ) : x.detailed ? (
                                  <>
                                    {modelSlug(x.model) || "no model"} · {x.mode === "plan" ? <span className="text-cyan">plan</span> : "build"}
                                    {x.hasGoal ? " · goal" : ""} ·{" "}
                                  </>
                                ) : null}
                                {relativeTime(x.updated)}
                              </span>
                            </SidebarMenuButton>
                          </SidebarMenuItem>
                        )
                      })}
                      {!showAll && g.items.length > PER_GROUP ? (
                        <SidebarMenuItem>
                          <SidebarMenuButton size="sm" className="text-muted-foreground" onClick={() => setExpanded(new Set([...expanded, g.cwd]))}>
                            Show {g.items.length - PER_GROUP} more
                          </SidebarMenuButton>
                        </SidebarMenuItem>
                      ) : null}
                    </SidebarMenu>
                  </SidebarGroupContent>
                </CollapsibleContent>
              </SidebarGroup>
            </Collapsible>
          )
        })}
      </SidebarContent>

      <SidebarFooter className="border-t border-sidebar-border">
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton isActive={c.view === "settings"} onClick={() => c.setView(c.view === "settings" ? "chat" : "settings")}>
              <Settings2 />
              <span>Settings</span>
              <Kbd className="ml-auto">{modKey},</Kbd>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
        <div className="flex items-center gap-2 px-2 pb-1 text-xs text-muted-foreground">
          <span className={cn("size-1.5 rounded-full", providers ? "bg-green" : "bg-warn")} />
          <span className="truncate">
            {providers ? `${providers} provider${providers === 1 ? "" : "s"}` : "No provider — /auth"}
            {s?.mcp.length ? ` · ${mcpReady}/${s.mcp.length} MCP` : ""}
            {mcpAuth ? " · sign-in needed" : ""}
          </span>
        </div>
      </SidebarFooter>
    </Sidebar>
  )
}
