import { useEffect, useState } from "react"
import { ChevronDown, ChevronRight, Columns2, Copy, FileText, FolderOpen, FolderPlus, GitBranch, GitBranchPlus, ListChecks, LoaderCircle, MessageCircleQuestion, MoreHorizontal, Moon, Pencil, Pin, PinOff, Plus, Search, Settings, ShieldQuestion, SquarePen, SquareTerminal, Trash2 } from "lucide-react"
import { useShallow } from "zustand/react/shallow"
import { AgentGlyph } from "@/components/icons"
import { confirmAction, promptText } from "@/components/Prompt"
import { Tip } from "@/components/Tip"
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from "@/components/ui/context-menu"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { ago, runningTasks, type ChatMeta, type Waiting } from "@/lib/chat"
import { samePath, worktrees as wt, type Worktree } from "@/lib/git"
import { LocationIcon } from "@/components/LocationIcon"
import { OpenProjectMenu } from "@/components/OpenProject"
import { isRemote, locationLabel } from "@/lib/types"
import { useLocationAwake, useLocationStopped, watchWsl } from "@/store/wsl"
import { copyText } from "@/lib/clipboard"
import type { Project } from "@/lib/types"
import { cn } from "@/lib/utils"
import { toast } from "sonner"
import { useApp, usePane } from "@/store/app"
import { copyChatMarkdown, needsYou, useChats, waitingOf } from "@/store/chats"
import { openChatSearch } from "./ChatSearch"
import { UpdateButton } from "./Updates"
import { useGit } from "@/store/git"

/** Projects, each with its worktrees and their chats; Settings at the bottom. */
/** A clickable row reached and used by keyboard too: Tab to it, Enter or Space to open it
 *  (Shift+F10 or the menu key opens a chat's menu). */
function rowKeys(activate: () => void) {
  return {
    role: "button",
    tabIndex: 0,
    onKeyDown: (e: React.KeyboardEvent) => {
      if (e.target !== e.currentTarget || (e.key !== "Enter" && e.key !== " ")) return
      e.preventDefault()
      activate()
    },
  } as const
}

/** The focus outline rows show (the theme's ring is too faint to find a row by). */
const ROW_FOCUS = "outline-none focus-visible:ring-2 focus-visible:ring-primary/70"

export function Sidebar() {
  useEffect(() => watchWsl(), [])
  const settings = useApp((s) => s.settings)
  const view = useApp((s) => s.view)
  const openSettings = useApp((s) => s.openSettings)
  const projectId = usePane().projectId
  const projects = settings?.projects ?? []


  return (
    <aside className="flex h-full min-w-0 flex-col bg-sidebar text-sidebar-foreground">
      <div className="flex gap-1 p-2">
        <button
          className="rounded-md flex h-8 min-w-0 flex-1 items-center gap-2 border border-input bg-background px-2.5 text-left text-[12.5px] font-medium hover:border-ring/60 hover:bg-accent disabled:opacity-50"
          disabled={!projectId}
          onClick={(e) => newChat(projectId!, null, toSide(e))}
        >
          <SquarePen className="size-3.5 shrink-0 text-muted-foreground" />
          <span className="truncate">New chat</span>
          <span className="ml-auto shrink-0 font-mono text-[10.5px] text-faint">Ctrl N</span>
        </button>
        <Tip content="Search chats (Ctrl K)">
          <button
            className="rounded-md grid size-8 shrink-0 place-items-center border border-input bg-background text-muted-foreground hover:border-ring/60 hover:bg-accent hover:text-foreground"
            aria-label="Search chats"
            onClick={() => openChatSearch()}
          >
            <Search className="size-3.5" />
          </button>
        </Tip>
      </div>

      <Inbox />

      <div className="flex h-7 items-center px-3 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        Projects
        <OpenProjectMenu>
          <button className="rounded-md ml-auto grid size-6 place-items-center hover:bg-accent hover:text-foreground" aria-label="Add project">
            <FolderPlus className="size-3.5" />
          </button>
        </OpenProjectMenu>
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

      <div className="flex flex-col gap-1 border-t p-1">
        <UpdateButton />
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

/** Opens an empty chat in a project, optionally in one of its worktrees: in the active pane,
 *  or beside it (`side`: Ctrl/⌘ was held). */
function newChat(projectId: string, cwd: string | null, side = false) {
  const target = { projectId, chatId: null, draftCwd: cwd, viewer: null }
  if (side) useApp.getState().splitPane(target)
  else useApp.getState().setPane(target)
  useApp.setState({ view: "chat" })
}

/** Ctrl or ⌘ held on a click: the row opens beside the active pane. */
const toSide = (e: React.MouseEvent | React.KeyboardEvent) => e.ctrlKey || e.metaKey

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
  const { projectId, chatId, draftCwd } = usePane()
  const view = useApp((s) => s.view)
  const removeProject = useApp((s) => s.removeProject)
  const openSettings = useApp((s) => s.openSettings)
  const metas = useChats((s) => s.metas)
  const trees = useGit((s) => s.worktrees[project.id])
  const remote = isRemote(project.location)
  // A stopped WSL distro isn't started just to list worktrees; opening the project starts it.
  const awake = useLocationAwake(project.location)
  const stopped = useLocationStopped(project.location)
  const selected = project.id === projectId

  useEffect(() => {
    if (awake) useGit.getState().loadWorktrees(project.id)
  }, [project.id, awake])

  // Pinned chats first; otherwise the store's order (newest activity first).
  const chats = metas.filter((m) => m.projectId === project.id).sort((a, b) => Number(b.pinned) - Number(a.pinned))
  const extra = (trees ?? []).filter((w) => !w.main)
  const inMain = (m: ChatMeta) => !extra.some((w) => samePath(m.cwd, w.path))
  // With worktrees listed, the main checkout's row shows the selection instead.
  const active = project.id === projectId && view === "chat" && !chatId && !draftCwd && !extra.length

  return (
    <div className="mt-px">
      <div
        className={cn("mx-1 rounded-md group flex h-8 cursor-pointer items-center gap-1.5 pr-1 pl-2 text-[12.5px] hover:bg-accent", ROW_FOCUS, active && "row-active")}
        aria-label={`New chat in ${project.name}`}
        {...rowKeys(() => (newChat(project.id, null), setOpen(true)))}
        onClick={(e) => (newChat(project.id, null, toSide(e)), setOpen(true))}
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
        <LocationIcon location={project.location} />
        <span className="truncate font-medium">{project.name}</span>
        {remote ? <span className="truncate text-[11px] text-faint">{locationLabel(project.location)}</span> : null}
        {stopped && !selected ? <span className="rounded-full shrink-0 border px-1.5 text-[10px] text-faint">stopped</span> : null}
        <div className="ml-auto flex opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 has-[[data-state=open]]:opacity-100">
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
            {/* React bubbles portal clicks to the row, whose onClick would start a new chat. */}
            <DropdownMenuContent align="start" className="w-52" onClick={(e) => e.stopPropagation()}>
              <DropdownMenuItem disabled={trees !== undefined && trees.length === 0} onSelect={() => newWorktree(project)}>
                <GitBranchPlus /> New worktree…
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={() => openSettings("projects")}>
                <Settings /> Project settings
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => copyText(project.path)}>
                <Copy /> Copy path
              </DropdownMenuItem>
              {!remote ? (
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
  const { projectId, chatId, draftCwd } = usePane()
  const main = !tree || tree.main
  const cwd = main ? null : tree.path
  const view = useApp((s) => s.view)
  const active = view === "chat" && projectId === project.id && !chatId && (main ? !draftCwd : samePath(draftCwd, cwd))

  const remove = async () => {
    if (!tree) return
    const ok = await confirmAction({
      title: `Remove worktree ${tree.branch ?? ""}?`,
      description: `Deletes the folder ${tree.path}${project.git.deleteBranchOnRemove ? " and its branch (commits not on another branch are lost)" : ""}. Uncommitted changes there are lost${chats.length ? `, and its ${chats.length === 1 ? "chat is" : `${chats.length} chats are`} deleted` : ""}.`,
      confirm: "Remove",
      danger: true,
    })
    if (!ok) return
    try {
      // Its chats' agents first: on Windows a folder an agent runs in can't be deleted.
      for (const c of chats) await useChats.getState().remove(c.id)
      await wt.remove(project.id, tree.path, true)
      await useGit.getState().loadWorktrees(project.id)
      // A pane about to start a chat there starts it in the project instead.
      for (const p of useApp.getState().panes) if (samePath(p.draftCwd, tree.path)) useApp.getState().setPane({ draftCwd: null }, p.id)
    } catch (e) {
      toast.error(String(e))
    }
  }

  return (
    <>
      <div
        className={cn("mx-1 rounded-md group/wt flex h-7 cursor-pointer items-center gap-1.5 pr-1 pl-6 text-[12.5px] hover:bg-accent", ROW_FOCUS, active && "row-active")}
        aria-label={`New chat in ${tree?.branch ?? "the main checkout"}`}
        {...rowKeys(() => newChat(project.id, cwd))}
        onClick={(e) => newChat(project.id, cwd, toSide(e))}
      >
        <button
          className="grid size-4 place-items-center text-faint"
          aria-label={open ? "Collapse" : "Expand"}
          onClick={(e) => {
            e.stopPropagation()
            setOpen(!open)
          }}
        >
          {open ? <ChevronDown className="size-3" /> : <ChevronRight className="size-3" />}
        </button>
        <GitBranch className={cn("size-3.5 shrink-0", "text-muted-foreground")} />
        <span className="truncate font-mono text-[12px]">{tree?.branch ?? "main checkout"}</span>
        {main ? <span className="rounded-full border px-1.5 text-[10px] text-faint">main</span> : null}
        <div className="ml-auto flex opacity-0 group-hover/wt:opacity-100 group-focus-within/wt:opacity-100 has-[[data-state=open]]:opacity-100">
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
              <DropdownMenuContent align="start" className="w-48" onClick={(e) => e.stopPropagation()}>
                <DropdownMenuItem onSelect={() => copyText(tree!.path)}>
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
  const chatId = usePane().chatId
  const view = useApp((s) => s.view)
  // On screen in the pane beside the active one.
  const beside = useApp((s) => s.view === "chat" && s.panes.some((p) => p.id !== s.activePane && p.chatId === meta.id))
  // Open in its agent's own CLI: it takes no messages until that ends.
  const inCli = useChats((s) => s.cli.includes(meta.id))
  const t = useChats((s) => s.transcripts[meta.id])
  const waits = useChats((s) => waitingOf(s, meta.id))
  // This window's own view of it when it has the chat open, else what the app says.
  const busyElsewhere = useChats((s) => s.busy.includes(meta.id))
  const running = t ? !!t.running : busyElsewhere
  // Background work between turns: what the open transcript says, else what the app says.
  const background = useChats((s) => (s.transcripts[meta.id] ? runningTasks(s.transcripts[meta.id]).some((k) => k.tracked) : s.background.includes(meta.id)))
  const active = chatId === meta.id && view === "chat"
  const rename = () =>
    void promptText({ title: "Rename chat", value: meta.title, confirm: "Rename" }).then((title) => {
      if (title) void useChats.getState().rename(meta.id, title)
    })
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div
          className={cn(
            "mx-1 rounded-md flex h-7 cursor-pointer items-center gap-2 pr-2 text-[12.5px] text-muted-foreground hover:bg-accent hover:text-foreground data-[state=open]:bg-accent data-[state=open]:text-foreground",
            ROW_FOCUS,
            active && "row-active",
            beside && "bg-sidebar-accent/60 text-foreground"
          )}
          style={{ paddingLeft: indent }}
          aria-current={active ? "page" : undefined}
          {...rowKeys(() => void useChats.getState().open(meta.id))}
          onClick={(e) => void useChats.getState().open(meta.id, toSide(e) ? "side" : undefined)}
        >
          {waits ? (
            <span className="rounded-full size-1.5 shrink-0 bg-primary" aria-label={`Waiting for your ${WAITING_LABEL[waits].toLowerCase()}`} />
          ) : meta.sleeping ? (
            <Moon className="size-3 shrink-0 text-faint" aria-label="Sleeping; send a message to resume" />
          ) : inCli ? (
            <Tip content="Open in the agent's CLI">
              <SquareTerminal className="size-3 shrink-0 text-muted-foreground" aria-label="Open in the agent's CLI" />
            </Tip>
          ) : running ? (
            <LoaderCircle className="size-3 shrink-0 animate-spin text-muted-foreground" />
          ) : background ? (
            <Tip content="Background work running">
              <LoaderCircle className="size-3 shrink-0 animate-spin text-faint [animation-duration:2.5s]" aria-label="Background work running" />
            </Tip>
          ) : (
            <AgentGlyph agent={meta.agent} className="size-3 shrink-0 opacity-70" />
          )}
          <span className="truncate">{meta.title || "New chat"}</span>
          {meta.pinned ? <Pin className="ml-auto size-3 shrink-0 text-faint" aria-label="Pinned" /> : null}
          <span className={cn("shrink-0 font-mono text-[10.5px] text-faint", !meta.pinned && "ml-auto")}>{ago(meta.updatedAt)}</span>
        </div>
      </ContextMenuTrigger>
      {/* React bubbles portal clicks to the row, whose onClick would open the chat. */}
      <ContextMenuContent className="w-48" onClick={(e) => e.stopPropagation()}>
        <ContextMenuItem onSelect={() => void useChats.getState().open(meta.id, "side")}>
          <Columns2 /> Open to the side
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem onSelect={() => useChats.getState().pin(meta.id, !meta.pinned)}>
          {meta.pinned ? <PinOff /> : <Pin />} {meta.pinned ? "Unpin" : "Pin"}
        </ContextMenuItem>
        <ContextMenuItem disabled={meta.sleeping || running || background || inCli || !!waits || !meta.agentSessionId} onSelect={() => useChats.getState().sleep(meta.id)}>
          <Moon /> {meta.sleeping ? "Sleeping" : "Sleep"}
        </ContextMenuItem>
        <ContextMenuItem onSelect={rename}>
          <Pencil /> Rename…
        </ContextMenuItem>
        <ContextMenuItem onSelect={() => void copyChatMarkdown(meta.id)}>
          <FileText /> Copy as Markdown
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem variant="destructive" onSelect={() => void useChats.getState().confirmRemove(meta.id)}>
          <Trash2 /> Delete chat
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
}

const WAITING_LABEL: Record<Waiting, string> = { approval: "Approval", plan: "Plan approval", question: "Answer" }
const WAITING_ICON = { approval: ShieldQuestion, plan: ListChecks, question: MessageCircleQuestion } satisfies Record<Waiting, unknown>

/** Chats waiting on the user, wherever they were started (here, a phone, MCP): each opens on
 *  its approval or question. */
function Inbox() {
  const ids = useChats(useShallow(needsYou))
  const metas = useChats((s) => s.metas)
  const projects = useApp((s) => s.settings?.projects)
  if (!ids.length) return null
  return (
    <div className="mx-2 mb-2 rounded-lg border border-primary/30 bg-primary/5">
      <div className="flex h-7 items-center gap-1.5 px-2.5 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        <span className="rounded-full size-1.5 bg-primary" />
        Needs you
        <span className="ml-auto font-mono normal-case text-faint">{ids.length}</span>
      </div>
      <ul className="max-h-48 overflow-y-auto pb-1">
        {ids.map((id) => {
          const meta = metas.find((m) => m.id === id)
          return meta ? <InboxRow key={id} meta={meta} project={projects?.find((p) => p.id === meta.projectId)?.name} /> : null
        })}
      </ul>
    </div>
  )
}

function InboxRow({ meta, project }: { meta: ChatMeta; project: string | undefined }) {
  const what = useChats((s) => waitingOf(s, meta.id))
  if (!what) return null
  const Icon = WAITING_ICON[what]
  const answer = () => void useChats.getState().answerChat(meta.id)
  return (
    <li className="group/in mx-1 flex h-8 cursor-pointer items-center gap-2 rounded-md pr-1 pl-1.5 text-[12.5px] hover:bg-accent" onClick={answer}>
      <Icon className="size-3.5 shrink-0 text-primary" aria-label={WAITING_LABEL[what]} />
      <div className="min-w-0 flex-1 leading-tight">
        <div className="truncate">{meta.title || "New chat"}</div>
        <div className="truncate text-[10.5px] text-faint">
          {WAITING_LABEL[what]}
          {project ? ` · ${project}` : ""}
        </div>
      </div>
      <button
        className="rounded-md h-6 shrink-0 bg-primary px-2 text-[11.5px] font-medium text-primary-foreground hover:bg-primary/90"
        onClick={(e) => {
          e.stopPropagation()
          answer()
        }}
      >
        Answer
      </button>
    </li>
  )
}
