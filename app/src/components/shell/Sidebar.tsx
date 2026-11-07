import { useEffect, useState } from "react"
import { ChevronDown, ChevronRight, Copy, Folder, FolderOpen, FolderPlus, GitBranch, GitBranchPlus, LoaderCircle, MoreHorizontal, Plus, Server, Settings, SquarePen, Trash2 } from "lucide-react"
import { AgentGlyph } from "@/components/icons"
import { confirmAction, promptText } from "@/components/Prompt"
import { Tip } from "@/components/Tip"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { ago, type ChatMeta } from "@/lib/chat"
import { samePath, worktrees as wt, type Worktree } from "@/lib/git"
import { inTauri, pickFolder } from "@/lib/ipc"
import type { Project } from "@/lib/types"
import { cn } from "@/lib/utils"
import { toast } from "sonner"
import { useApp } from "@/store/app"
import { useChats } from "@/store/chats"
import { useGit } from "@/store/git"

/** Projects, each with its worktrees and their chats; Settings at the bottom. */
export function Sidebar() {
  const settings = useApp((s) => s.settings)
  const view = useApp((s) => s.view)
  const openSettings = useApp((s) => s.openSettings)
  const addProject = useApp((s) => s.addProject)
  const projectId = useApp((s) => s.projectId)
  const projects = settings?.projects ?? []

  const add = async () => {
    const path = await pickFolder("Add a project folder")
    if (path) await addProject({ kind: "local" }, path)
  }

  return (
    <aside className="flex h-full min-w-0 flex-col border-r bg-sidebar text-sidebar-foreground">
      <div className="p-2">
        <button
          className="rounded-md flex h-8 w-full items-center gap-2 border border-input bg-background px-2.5 text-left text-[12.5px] font-medium hover:border-ring/60 hover:bg-accent disabled:opacity-50"
          disabled={!projectId}
          onClick={() => newChat(projectId!, null)}
        >
          <SquarePen className="size-3.5 text-muted-foreground" />
          New chat
          <span className="ml-auto font-mono text-[10.5px] text-faint">Ctrl N</span>
        </button>
      </div>

      <div className="flex h-7 items-center px-3 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        Projects
        <Tip content="Add project" side="right">
          <button className="rounded-md ml-auto grid size-6 place-items-center hover:bg-accent hover:text-foreground" aria-label="Add project" onClick={add}>
            <FolderPlus className="size-3.5" />
          </button>
        </Tip>
      </div>

      <nav className="min-h-0 flex-1 overflow-y-auto pb-2">
        {projects.length === 0 ? (
          <div className="rounded-lg mx-3 mt-1 border border-dashed p-3 text-[12px] leading-relaxed text-muted-foreground">
            No projects yet. Add a folder on this computer or on an SSH host to start chatting.
          </div>
        ) : (
          projects.map((p) => <ProjectGroup key={p.id} project={p} />)
        )}
      </nav>

      <div className="border-t p-1">
        <button
          className={cn("rounded-md flex h-8 w-full items-center gap-2 px-2.5 text-[12.5px] text-muted-foreground hover:bg-accent hover:text-foreground", view === "settings" && "row-active")}
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

/** Opens an empty chat in a project, optionally in one of its worktrees. */
function newChat(projectId: string, cwd: string | null) {
  useApp.getState().selectProject(projectId)
  useApp.setState({ chatId: null, draftCwd: cwd, view: "chat" })
  useGit.getState().open(null)
}

async function newWorktree(project: Project) {
  const branch = await promptText({
    title: "New worktree",
    description: `A new branch from ${project.git.baseBranch || "the default branch"}, checked out in its own folder, so this chat's changes stay apart from your other work.`,
    placeholder: "feature/branch-name",
    confirm: "Create worktree",
  })
  if (!branch) return
  const id = toast.loading(`Creating ${branch}…`)
  try {
    const w = await wt.create(project.id, branch)
    await useGit.getState().loadWorktrees(project.id)
    toast.success(`Worktree ${w.branch} is ready`, { id })
    newChat(project.id, w.path)
  } catch (e) {
    toast.error(String(e), { id })
  }
}

function ProjectGroup({ project }: { project: Project }) {
  const [open, setOpen] = useState(true)
  const projectId = useApp((s) => s.projectId)
  const chatId = useApp((s) => s.chatId)
  const draftCwd = useApp((s) => s.draftCwd)
  const view = useApp((s) => s.view)
  const removeProject = useApp((s) => s.removeProject)
  const openSettings = useApp((s) => s.openSettings)
  const metas = useChats((s) => s.metas)
  const trees = useGit((s) => s.worktrees[project.id])
  const remote = project.location.kind === "ssh"
  const Icon = remote ? Server : Folder

  useEffect(() => {
    useGit.getState().loadWorktrees(project.id)
  }, [project.id])

  const chats = metas.filter((m) => m.projectId === project.id)
  const extra = (trees ?? []).filter((w) => !w.main)
  const inMain = (m: ChatMeta) => !extra.some((w) => samePath(m.cwd, w.path))
  const active = project.id === projectId && view === "chat" && !chatId && !draftCwd

  return (
    <div className="mt-px">
      <div className={cn("mx-1 rounded-md group flex h-8 cursor-pointer items-center gap-1.5 pr-1 pl-2 text-[12.5px] hover:bg-accent", active && "row-active")} onClick={() => (newChat(project.id, null), setOpen(true))}>
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
        {project.location.kind === "ssh" ? <span className="truncate text-[11px] text-faint">{project.location.host}</span> : null}
        <div className="ml-auto flex opacity-0 group-hover:opacity-100 has-[[data-state=open]]:opacity-100">
          <Tip content="New chat" side="top">
            <button className="rounded-md grid size-6 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="New chat" onClick={(e) => (e.stopPropagation(), newChat(project.id, null))}>
              <Plus className="size-3.5" />
            </button>
          </Tip>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button className="rounded-md grid size-6 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Project menu" onClick={(e) => e.stopPropagation()}>
                <MoreHorizontal className="size-3.5" />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" className="w-52">
              <DropdownMenuItem disabled={trees !== undefined && trees.length === 0} onSelect={() => newWorktree(project)}>
                <GitBranchPlus /> New worktree…
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={() => openSettings("projects")}>
                <Settings /> Project settings
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => navigator.clipboard?.writeText(project.path)}>
                <Copy /> Copy path
              </DropdownMenuItem>
              {!remote && inTauri ? (
                <DropdownMenuItem onSelect={() => import("@tauri-apps/plugin-opener").then((m) => m.revealItemInDir(project.path))}>
                  <FolderOpen /> Show in folder
                </DropdownMenuItem>
              ) : null}
              <DropdownMenuSeparator />
              <DropdownMenuItem variant="destructive" onSelect={() => removeProject(project.id)}>
                <Trash2 /> Remove from oah
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
      {open ? (
        extra.length ? (
          <>
            <WorktreeRow project={project} tree={(trees ?? []).find((w) => w.main) ?? null} chats={chats.filter(inMain)} />
            {extra.map((w) => (
              <WorktreeRow key={w.path} project={project} tree={w} chats={chats.filter((m) => samePath(m.cwd, w.path))} />
            ))}
          </>
        ) : chats.length ? (
          chats.map((c) => <ChatRow key={c.id} meta={c} indent={36} />)
        ) : (
          <div className="py-0.5 pr-2 pl-9 text-[12px] text-faint">No chats yet</div>
        )
      ) : null}
    </div>
  )
}

/** A worktree (or the main checkout) and its chats. */
function WorktreeRow({ project, tree, chats }: { project: Project; tree: Worktree | null; chats: ChatMeta[] }) {
  const [open, setOpen] = useState(true)
  const projectId = useApp((s) => s.projectId)
  const chatId = useApp((s) => s.chatId)
  const draftCwd = useApp((s) => s.draftCwd)
  const main = !tree || tree.main
  const cwd = main ? null : tree.path
  const view = useApp((s) => s.view)
  const active = view === "chat" && projectId === project.id && !chatId && (main ? !draftCwd : samePath(draftCwd, cwd))

  const remove = async () => {
    if (!tree) return
    const ok = await confirmAction({
      title: `Remove worktree ${tree.branch ?? ""}?`,
      description: `Deletes the folder ${tree.path}${project.git.deleteBranchOnRemove ? " and its branch" : ""}. Uncommitted changes there are lost.`,
      confirm: "Remove",
      danger: true,
    })
    if (!ok) return
    try {
      await wt.remove(project.id, tree.path, true)
      for (const c of chats) await useChats.getState().remove(c.id)
      await useGit.getState().loadWorktrees(project.id)
      if (samePath(useApp.getState().draftCwd, tree.path)) useApp.setState({ draftCwd: null })
    } catch (e) {
      toast.error(String(e))
    }
  }

  return (
    <>
      <div className={cn("mx-1 rounded-md group/wt flex h-7 cursor-pointer items-center gap-1.5 pr-1 pl-6 text-[12.5px] hover:bg-accent", active && "row-active")} onClick={() => newChat(project.id, cwd)}>
        <button
          className="grid size-4 place-items-center text-faint"
          onClick={(e) => {
            e.stopPropagation()
            setOpen(!open)
          }}
        >
          {open ? <ChevronDown className="size-3" /> : <ChevronRight className="size-3" />}
        </button>
        <GitBranch className={cn("size-3.5 shrink-0", main ? "text-muted-foreground" : "text-gold")} />
        <span className="truncate font-mono text-[12px]">{tree?.branch ?? "main checkout"}</span>
        {main ? <span className="rounded-sm border px-1 text-[10px] text-faint">main</span> : null}
        <div className="ml-auto flex opacity-0 group-hover/wt:opacity-100 has-[[data-state=open]]:opacity-100">
          <Tip content="New chat here" side="top">
            <button className="rounded-md grid size-6 place-items-center text-muted-foreground hover:bg-background hover:text-foreground" aria-label="New chat here" onClick={(e) => (e.stopPropagation(), newChat(project.id, cwd))}>
              <Plus className="size-3.5" />
            </button>
          </Tip>
          {!main ? (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <button className="rounded-md grid size-6 place-items-center text-muted-foreground hover:bg-background hover:text-foreground" aria-label="Worktree menu" onClick={(e) => e.stopPropagation()}>
                  <MoreHorizontal className="size-3.5" />
                </button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" className="w-48">
                <DropdownMenuItem onSelect={() => navigator.clipboard?.writeText(tree!.path)}>
                  <Copy /> Copy path
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem variant="destructive" onSelect={remove}>
                  <Trash2 /> Remove worktree
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          ) : null}
        </div>
      </div>
      {open ? chats.length ? chats.map((c) => <ChatRow key={c.id} meta={c} indent={52} />) : <div className="py-0.5 pr-2 pl-[52px] text-[12px] text-faint">No chats yet</div> : null}
    </>
  )
}

function ChatRow({ meta, indent }: { meta: ChatMeta; indent: number }) {
  const chatId = useApp((s) => s.chatId)
  const view = useApp((s) => s.view)
  const t = useChats((s) => s.transcripts[meta.id])
  const active = chatId === meta.id && view === "chat"
  return (
    <div
      className={cn("mx-1 rounded-md group/chat flex h-7 cursor-pointer items-center gap-2 pr-1 text-[12.5px] text-muted-foreground hover:bg-accent hover:text-foreground", active && "row-active")}
      style={{ paddingLeft: indent }}
      onClick={() => {
        useApp.getState().selectProject(meta.projectId)
        useChats.getState().open(meta.id)
      }}
    >
      {t?.pending ? (
        <span className="rounded-full size-1.5 shrink-0 bg-primary" />
      ) : t?.running ? (
        <LoaderCircle className="size-3 shrink-0 animate-spin text-gold" />
      ) : (
        <AgentGlyph agent={meta.agent} className="size-3 shrink-0 opacity-70" />
      )}
      <span className="truncate">{meta.title || "New chat"}</span>
      <span className="ml-auto shrink-0 font-mono text-[10.5px] text-faint group-hover/chat:hidden">{ago(meta.updatedAt)}</span>
      <button
        className="rounded-md hidden size-5 shrink-0 place-items-center text-faint group-hover/chat:grid hover:text-red"
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
