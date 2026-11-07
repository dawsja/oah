import { useState } from "react"
import { ChevronDown, ChevronRight, Folder, FolderPlus, LoaderCircle, MoreHorizontal, Plus, Server, Settings, SquarePen, Trash2 } from "lucide-react"
import { AgentGlyph } from "@/components/icons"
import { Tip } from "@/components/Tip"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { pickFolder } from "@/lib/ipc"
import type { Project } from "@/lib/types"
import { cn } from "@/lib/utils"
import { ago, type ChatMeta } from "@/lib/chat"
import { useApp } from "@/store/app"
import { useChats } from "@/store/chats"

/** Projects, each with its workspaces and chats; Settings at the bottom. */
export function Sidebar() {
  const settings = useApp((s) => s.settings)
  const projectId = useApp((s) => s.projectId)
  const view = useApp((s) => s.view)
  const chatId = useApp((s) => s.chatId)
  const openSettings = useApp((s) => s.openSettings)
  const addProject = useApp((s) => s.addProject)
  const projects = settings?.projects ?? []

  const add = async () => {
    const path = await pickFolder("Add a project folder")
    if (path) await addProject({ kind: "local" }, path)
  }

  return (
    <aside className="flex h-full min-w-0 flex-col border-r bg-sidebar text-sidebar-foreground">
      <div className="p-2">
        <button
          className="flex h-8 w-full items-center gap-2 border border-input bg-background px-2.5 text-left text-[12.5px] font-medium hover:border-ring/60 hover:bg-accent disabled:opacity-50"
          disabled={!projectId}
          onClick={() => useApp.setState({ chatId: null, view: "chat" })}
        >
          <SquarePen className="size-3.5 text-muted-foreground" />
          New chat
          <span className="ml-auto font-mono text-[10.5px] text-faint">Ctrl N</span>
        </button>
      </div>

      <div className="flex h-7 items-center px-3 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        Projects
        <Tip content="Add project" side="right">
          <button className="ml-auto grid size-6 place-items-center hover:bg-accent hover:text-foreground" aria-label="Add project" onClick={add}>
            <FolderPlus className="size-3.5" />
          </button>
        </Tip>
      </div>

      <nav className="min-h-0 flex-1 overflow-y-auto pb-2">
        {projects.length === 0 ? (
          <div className="mx-3 mt-1 border border-dashed p-3 text-[12px] leading-relaxed text-muted-foreground">
            No projects yet. Add a folder on this computer or on an SSH host to start chatting.
          </div>
        ) : (
          projects.map((p) => <ProjectGroup key={p.id} project={p} active={p.id === projectId && view === "chat" && !chatId} />)
        )}
      </nav>

      <div className="border-t p-1">
        <button
          className={cn("flex h-8 w-full items-center gap-2 px-2.5 text-[12.5px] text-muted-foreground hover:bg-accent hover:text-foreground", view === "settings" && "row-active")}
          onClick={() => openSettings()}
        >
          <Settings className="size-3.5" />
          Settings
          <span className="ml-auto font-mono text-[10.5px] text-faint">Ctrl ,</span>
        </button>
      </div>
    </aside>
  )
}

function ChatRow({ meta }: { meta: ChatMeta }) {
  const chatId = useApp((s) => s.chatId)
  const view = useApp((s) => s.view)
  const t = useChats((s) => s.transcripts[meta.id])
  const active = chatId === meta.id && view === "chat"
  return (
    <div
      className={cn("group/chat flex h-7 cursor-pointer items-center gap-2 pr-1 pl-9 text-[12.5px] text-muted-foreground hover:bg-accent hover:text-foreground", active && "row-active")}
      onClick={() => {
        useApp.getState().selectProject(meta.projectId)
        useChats.getState().open(meta.id)
      }}
    >
      {t?.pending ? (
        <span className="size-1.5 shrink-0 bg-primary" />
      ) : t?.running ? (
        <LoaderCircle className="size-3 shrink-0 animate-spin text-gold" />
      ) : (
        <AgentGlyph agent={meta.agent} className="size-3 shrink-0 opacity-70" />
      )}
      <span className="truncate">{meta.title || "New chat"}</span>
      <span className="ml-auto shrink-0 font-mono text-[10.5px] text-faint group-hover/chat:hidden">{ago(meta.updatedAt)}</span>
      <button
        className="hidden size-5 shrink-0 place-items-center text-faint group-hover/chat:grid hover:text-red"
        aria-label="Delete chat"
        onClick={(e) => {
          e.stopPropagation()
          useChats.getState().remove(meta.id)
        }}
      >
        <Trash2 className="size-3" />
      </button>
    </div>
  )
}

function ProjectGroup({ project, active }: { project: Project; active: boolean }) {
  const [open, setOpen] = useState(true)
  const selectProject = useApp((s) => s.selectProject)
  const removeProject = useApp((s) => s.removeProject)
  const openSettings = useApp((s) => s.openSettings)
  const chatId = useApp((s) => s.chatId)
  const remote = project.location.kind === "ssh"
  const Icon = remote ? Server : Folder
  const metas = useChats((s) => s.metas)
  const chats = metas.filter((m) => m.projectId === project.id)
  const newChat = () => {
    selectProject(project.id)
    useApp.setState({ chatId: null })
  }

  return (
    <div className="mt-px">
      <div
        className={cn("group flex h-8 cursor-pointer items-center gap-1.5 pr-1 pl-2 text-[12.5px] hover:bg-accent", active && "row-active")}
        onClick={() => {
          newChat()
          setOpen(true)
        }}
      >
        <button
          className="grid size-4 place-items-center text-muted-foreground"
          aria-label={open ? "Collapse" : "Expand"}
          onClick={(e) => {
            e.stopPropagation()
            setOpen(!open)
          }}
        >
          {open ? <ChevronDown className="size-3" /> : <ChevronRight className="size-3" />}
        </button>
        <Icon className={cn("size-3.5 shrink-0", remote ? "text-cyan" : "text-muted-foreground")} />
        <span className="truncate font-medium">{project.name}</span>
        {remote && project.location.kind === "ssh" ? <span className="truncate text-[11px] text-faint">{project.location.host}</span> : null}
        <div className="ml-auto flex opacity-0 group-hover:opacity-100 has-[[data-state=open]]:opacity-100">
          <Tip content="New chat" side="top">
            <button className="grid size-6 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="New chat" onClick={(e) => (e.stopPropagation(), newChat())}>
              <Plus className="size-3.5" />
            </button>
          </Tip>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button className="grid size-6 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Project menu" onClick={(e) => e.stopPropagation()}>
                <MoreHorizontal className="size-3.5" />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" className="w-48">
              <DropdownMenuItem onSelect={() => openSettings("projects")}>Project settings</DropdownMenuItem>
              <DropdownMenuItem onSelect={() => navigator.clipboard?.writeText(project.path)}>Copy path</DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem variant="destructive" onSelect={() => removeProject(project.id)}>
                <Trash2 /> Remove from oah
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
      {open ? (
        chats.length ? (
          chats.map((c) => <ChatRow key={c.id} meta={c} />)
        ) : (
          <div className="py-0.5 pr-2 pl-9 text-[12px] text-faint">No chats yet</div>
        )
      ) : null}
    </div>
  )
}
