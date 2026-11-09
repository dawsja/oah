import { useEffect, useRef, useState } from "react"
import { Check, ChevronDown, ChevronRight, Circle, CornerDownRight, FileText, FolderPlus, Image as ImageIcon, LoaderCircle, MoreHorizontal, Pause, Pencil, Server, Trash2, X } from "lucide-react"
import { AgentGlyph, Mark } from "@/components/icons"
import { promptText } from "@/components/Prompt"
import { Button } from "@/components/ui/button"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { agentChoices } from "@/lib/agents"
import { emptyTranscript, type ChatMeta, type ModelOption, type SlashCommand, type PlanItem, type Transcript as T } from "@/lib/chat"
import { copyText } from "@/lib/clipboard"
import { LocationIcon } from "@/components/LocationIcon"
import { addLocalFolder, OpenProjectMenu, useDistros } from "@/components/OpenProject"
import { ipc, type Catalog } from "@/lib/ipc"
import { AGENT_LABEL, isOpencode, locationLabel, type AgentKind, type Project } from "@/lib/types"
import { cn } from "@/lib/utils"
import { useApp, useProject } from "@/store/app"
import { copyChatMarkdown, useChats, type Queued } from "@/store/chats"
import { draftKey } from "@/store/drafts"
import { useGit, useScope } from "@/store/git"
import { useStartOpenDistro } from "@/store/wsl"
import { Viewer } from "./Viewer"
import { ApprovalCard } from "./Approval"
import { QuestionCard } from "./QuestionCard"
import { Composer, editInComposer, STEER_KEY } from "./Composer"
import { Tip } from "@/components/Tip"
import { CwdContext } from "./cwd"
import { Transcript } from "./Transcript"
import { TasksDock } from "./TasksDock"

/** The middle of the window: the open chat, or a new one in the current project. */
export function ChatView() {
  const settings = useApp((s) => s.settings)
  const chatId = useApp((s) => s.chatId)
  const project = useProject()
  const meta = useChats((s) => s.metas.find((m) => m.id === chatId))
  const transcript = useChats((s) => (chatId ? s.transcripts[chatId] : undefined))
  const viewer = useGit((s) => s.viewer)
  const scope = useScope()

  useEffect(() => {
    if (chatId && !transcript) useChats.getState().open(chatId)
  }, [chatId, transcript])

  if (!settings) return null
  if (!project) return <NoProject />
  if (viewer && scope) return <Viewer scope={scope} viewer={viewer} />
  // Keyed, so nothing of one chat (how far it's scrolled, an open dock) carries to the next, and
  // a new chat in another project starts from that project's agent and model.
  if (chatId && meta && meta.projectId === project.id) return <OpenChat key={meta.id} project={project} meta={meta} transcript={transcript ?? emptyTranscript()} />
  return <NewChat key={project.id} project={project} />
}

function useAgentDefaults(project: Project, agent: AgentKind) {
  const provider = useApp((s) => s.settings)!.providers[agent]
  return { model: project.defaultModel || provider.defaultModel || "", permission: provider.permissionMode }
}

/** Catalogs by agent and project, kept a few minutes: listing one starts the agent's CLI (a
 *  whole `claude -p`, a throwaway OpenCode session), so the model menu and every lone `/`
 *  shouldn't. A failed one isn't kept. */
const catalogCache = new Map<string, { at: number; catalog: Promise<Catalog> }>()
const CATALOG_FRESH = 5 * 60_000
/** The commands a running chat of each agent last listed, so a new chat has them at once. */
const seenCommands = new Map<string, SlashCommand[]>()
function useCatalog(agent: AgentKind, projectId: string, live: { models: ModelOption[]; commands: SlashCommand[] } = { models: [], commands: [] }) {
  const key = `${agent}|${projectId}`
  const [fetched, setFetched] = useState<Catalog | undefined>(undefined)
  // An answer for the agent picked before this one is dropped.
  const keyRef = useRef(key)
  keyRef.current = key
  useEffect(() => setFetched(undefined), [key])
  if (live.commands.length) seenCommands.set(key, live.commands)
  const load = () => {
    // A running chat already has its agent's own lists (only OpenCode's are fuller from the catalog).
    if (live.models.length && live.commands.length && !isOpencode(agent)) return
    const hit = catalogCache.get(key)
    if (!hit || Date.now() - hit.at > CATALOG_FRESH) {
      const catalog = ipc.providerCatalog(agent, projectId)
      catalogCache.set(key, { at: Date.now(), catalog })
      catalog.catch(() => catalogCache.delete(key))
    }
    const asked = key
    catalogCache
      .get(key)!
      .catalog.then((c) => asked === keyRef.current && setFetched(c))
      .catch(() => asked === keyRef.current && setFetched({ models: [], commands: [] }))
  }
  const models = isOpencode(agent) && fetched ? fetched.models : live.models.length ? live.models : fetched?.models
  const commands = live.commands.length ? live.commands : (seenCommands.get(key) ?? fetched?.commands ?? [])
  return { models, commands, load }
}

function NewChat({ project }: { project: Project }) {
  const settings = useApp((s) => s.settings)!
  const choices = agentChoices(settings)
  const preferred = project.defaultAgent ?? settings.providers.defaultAgent
  const [agent, setAgent] = useState<AgentKind>(choices.includes(preferred) ? preferred : (choices[0] ?? "claude"))
  const d = useAgentDefaults(project, agent)
  const [permission, setPermission] = useState(d.permission)
  const [model, setModel] = useState<string | null>(null)
  const [effort, setEffort] = useState("")
  const { models, commands, load } = useCatalog(agent, project.id)
  const { distro, starting } = useStartOpenDistro(project.location)
  const draftCwd = useApp((s) => s.draftCwd)
  const worktree = useGit((s) => s.worktrees[project.id]?.find((w) => !w.main && w.path === draftCwd))
  const send = (text: string, images: { mime: string; data: string; name?: string }[] = []) =>
    useChats.getState().send(null, text, { agent, model: model ?? undefined, effort: effort || undefined, permission, cwd: draftCwd ?? undefined, images })
  // On this computer, whether the chosen agent's CLI is there at all: said before the first
  // message fails, with the way to install it.
  const [installed, setInstalled] = useState<boolean | null>(null)
  const binary = settings.providers[agent].binary
  useEffect(() => {
    setInstalled(null)
    if (project.location.kind !== "local") return
    let current = true
    ipc.providerProbe(agent, binary).then(
      (p) => current && setInstalled(p.found),
      () => undefined,
    )
    return () => void (current = false)
  }, [agent, binary, project.location.kind])

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col overflow-clip bg-background">
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center px-6 pb-[12vh]">
        <div className="w-full max-w-[46rem]">
          <div className="mb-6 flex items-end gap-3">
            <Mark size={34} />
            <div>
              <h1 className="text-[22px] leading-tight font-semibold tracking-tight">What should we work on?</h1>
              <p className="mt-1 text-[12.5px] text-muted-foreground">
                {AGENT_LABEL[agent]} in <span className="font-medium text-foreground">{project.name}</span>
                {worktree ? (
                  <span className="rounded-full ml-1 border px-1.5 font-mono text-[11px] text-muted-foreground">⎇ {worktree.branch}</span>
                ) : null}
                <span className="text-faint"> · {locationLabel(project.location)}</span>
                {starting ? (
                  <span className="ml-1.5 inline-flex items-center gap-1 text-muted-foreground">
                    <LoaderCircle className="size-3 animate-spin" /> starting {distro}…
                  </span>
                ) : null}
              </p>
              {installed === false ? (
                <p className="mt-1.5 text-[12.5px] text-warn" role="status">
                  {AGENT_LABEL[agent]} isn't installed on this computer.{" "}
                  <button className="underline underline-offset-2 hover:text-foreground" onClick={() => useApp.getState().openSettings("providers")}>
                    Set it up in Providers
                  </button>
                  {agentChoices(settings).length > 1 ? " or pick another agent below." : "."}
                </p>
              ) : null}
            </div>
          </div>
          <Composer
            key={draftKey(null, project.id, draftCwd)}
            draftKey={draftKey(null, project.id, draftCwd)}
            agent={agent}
            agents={choices}
            onAgent={(a) => {
              setAgent(a)
              setModel(null)
              setEffort("")
              // That agent's own default mode, not the last one's (which may be Full access).
              setPermission(settings.providers[a].permissionMode)
            }}
            model={model ?? d.model}
            effort={effort}
            models={models}
            onModel={(id, e) => {
              setModel(id)
              setEffort(e)
            }}
            onModelsOpen={load}
            commands={commands}
            permission={permission}
            onPermission={setPermission}
            onSend={send}
            autoFocus
          />
          <div className="mt-3 grid grid-cols-3 gap-2 text-[12px]">
            {["Explain how this project is laid out", "Find and fix a failing test", "Review my uncommitted changes"].map((t) => (
              <button key={t} className="rounded-lg border bg-card px-3.5 py-2.5 text-left text-muted-foreground hover:border-ring/60 hover:bg-accent hover:text-foreground" onClick={() => editInComposer(t, [])}>
                {t}
              </button>
            ))}
          </div>
        </div>
      </div>
    </div>
  )
}

function OpenChat({ project, meta, transcript }: { project: Project; meta: ChatMeta; transcript: T }) {
  const agent = meta.agent
  const d = useAgentDefaults(project, agent)
  const { models, commands, load } = useCatalog(agent, project.id, transcript)
  // The mode it last ran with (from here, a phone or MCP) before the agent's default.
  const permission = useChats((s) => s.permission[meta.id]) ?? meta.permission ?? d.permission
  const queued = useChats((s) => s.queued[meta.id]) ?? NONE
  const chats = useChats.getState()
  const dock = useRef<HTMLDivElement>(null)
  const focus = useChats((s) => s.focus === meta.id)
  const card = transcript.questions[0]?.requestId ?? transcript.approvals[0]?.requestId
  const loaded = useChats((s) => !!s.transcripts[meta.id])
  // Opened to answer it (the inbox, a notification): the waiting card takes the cursor.
  useEffect(() => {
    if (!focus) return
    const el = dock.current?.querySelector<HTMLElement>("[data-waiting]")
    if (!el) {
      // Nothing waits any more once the chat loaded: nothing to focus.
      if (loaded) useChats.setState({ focus: null })
      return
    }
    useChats.setState({ focus: null })
    el.scrollIntoView({ block: "nearest" })
    el.querySelector<HTMLElement>("[role=region]")?.focus({ preventScroll: true })
    el.animate([{ boxShadow: "0 0 0 2px var(--primary)" }, { boxShadow: "0 0 0 0 transparent" }], { duration: 1200, easing: "ease-out" })
  }, [focus, card, loaded])

  return (
    <CwdContext.Provider value={transcript.cwd ?? meta.cwd}>
    <div className="flex h-full min-h-0 min-w-0 flex-col overflow-clip bg-background">
      <ChatHeader meta={meta} transcript={transcript} model={transcript.model || meta.model || d.model} />
      <Transcript transcript={transcript} className="flex-1" />
      {meta.sleeping && !transcript.running ? <p className="px-6 py-2 text-center text-[12px] text-muted-foreground">Sleeping · send a message to resume this session.</p> : null}
      <div className="mx-auto flex max-h-[75%] min-h-0 w-full max-w-[46rem] shrink-0 flex-col gap-2 px-6 pb-4">
        <div ref={dock} className="flex min-h-0 flex-col gap-2 overflow-y-auto overscroll-contain empty:hidden">
          {transcript.plan.length ? <PlanDock items={transcript.plan} /> : null}
          {transcript.tasks.length ? <TasksDock agent={meta.agent} tasks={transcript.tasks} onStop={(id) => void chats.stopTask(meta.id, id)} /> : null}
          {transcript.questions[0] ? (
            <div data-waiting className="rounded-2xl flex min-h-0 flex-col">
              <QuestionCard request={transcript.questions[0]} onAnswer={(a) => chats.answerQuestion(meta.id, transcript.questions[0].requestId, a)} />
            </div>
          ) : transcript.approvals[0] ? (
            <div data-waiting className="rounded-2xl flex min-h-0 flex-col">
              <ApprovalCard request={transcript.approvals[0]} queued={transcript.approvals.length - 1} onAnswer={(dec) => chats.answer(meta.id, transcript.approvals[0].requestId, dec)} />
            </div>
          ) : null}
        </div>
        <div className="shrink-0">
          {queued.length ? <QueueDock chatId={meta.id} items={queued} running={transcript.running} /> : null}
          <Composer
            history={transcript.items.filter((item) => item.kind === "user").map((item) => item.text)}
            key={meta.id}
            draftKey={draftKey(meta.id, project.id)}
            agent={agent}
            model={meta.model || transcript.model || d.model}
            effort={meta.effort}
            context={transcript.context}
            models={models}
            onModel={(id, e) => {
              if (id !== meta.model) void chats.setModel(meta.id, id)
              if (e !== meta.effort) void chats.setEffort(meta.id, e)
            }}
            onModelsOpen={load}
            commands={commands}
            onClear={() => chats.clear(meta.id)}
            onCompact={(instructions) => chats.compact(meta.id, instructions, permission)}
            permission={permission}
            onPermission={(m) => chats.setPermission(meta.id, m)}
            running={transcript.running}
            onSend={(text, images, how) => {
              const running = useChats.getState().transcripts[meta.id]?.running
              // A command never goes into a turn that's running: it waits its turn.
              if (running && how === "steer" && !text.startsWith("/")) return chats.steer(meta.id, text, images)
              if (running) return void chats.enqueue(meta.id, text, images)
              return chats.send(meta.id, text, { agent: meta.agent, permission, images })
            }}
            onSteerQueued={queued.length ? () => void chats.sendQueued(meta.id) : undefined}
            onStop={() => chats.interrupt(meta.id)}
            placeholder={transcript.running ? `Queue a follow-up, or ${STEER_KEY}+Enter to send it now` : "Reply, @ to add files, / for commands"}
            lockAgent
            autoFocus
          />
        </div>
      </div>
    </div>
    </CwdContext.Provider>
  )
}

function ChatHeader({ meta, transcript, model }: { meta: ChatMeta; transcript: T; model: string }) {
  return (
    <div className="flex h-10 shrink-0 items-center gap-2 border-b px-4">
      <AgentGlyph agent={meta.agent} className="size-3.5" />
      <span className="truncate text-[13px] font-medium">{meta.title || "New chat"}</span>
      {transcript.running ? <LoaderCircle className="size-3.5 animate-spin text-muted-foreground" /> : null}
      <span className="ml-auto font-mono text-[11px] text-faint">{model}</span>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button className="rounded-md grid size-7 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Chat menu">
            <MoreHorizontal className="size-4" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-48">
          <DropdownMenuItem
            onSelect={() => {
              void promptText({ title: "Rename chat", value: meta.title, confirm: "Rename" }).then((t) => {
                if (t) void useChats.getState().rename(meta.id, t)
              })
            }}
          >
            Rename
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => void copyChatMarkdown(meta.id)}>
            <FileText /> Copy as Markdown
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => copyText(meta.agentSessionId ?? "")} disabled={!meta.agentSessionId}>
            Copy the agent's session id
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem variant="destructive" onSelect={() => void useChats.getState().confirmRemove(meta.id)}>
            <Trash2 /> Delete chat
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  )
}

const NONE: Queued[] = []

/** Messages waiting for the turn to end: each can go now, back to the composer, or away. */
function QueueDock({ chatId, items, running }: { chatId: string; items: Queued[]; running: boolean }) {
  const chats = useChats.getState()
  return (
    <div className="mx-auto w-[calc(100%-3rem)] max-w-[40rem] rounded-t-lg border border-b-0 bg-card text-[12.5px]">
      <div className="flex h-6 items-center gap-1.5 px-3 pt-1 text-[10.5px] text-muted-foreground">
        {running ? null : <Pause className="size-3" />}
        <span className="font-medium text-foreground">{items.length} queued</span>
        <span className="truncate text-faint">{running ? "· sends when the agent finishes" : "· held after the stop"}</span>
        {running ? null : (
          <button className="rounded-md ml-auto px-1.5 text-[11.5px] text-foreground hover:bg-accent" onClick={() => void chats.sendQueued(chatId)}>
            Resume
          </button>
        )}
      </div>
      <ul className="max-h-40 overflow-y-auto pb-1">
        {items.map((q, i) => (
          <li key={q.id} className="group/q flex h-8 items-center gap-2 px-3 hover:bg-accent/50">
            <span className="w-3 shrink-0 text-right font-mono text-[10.5px] text-faint">{i + 1}</span>
            <span className="min-w-0 flex-1 truncate">{q.text.replace(/\s+/g, " ")}</span>
            {q.images.length ? (
              <span className="flex shrink-0 items-center gap-0.5 text-[11px] text-faint">
                <ImageIcon className="size-3" /> {q.images.length}
              </span>
            ) : null}
            <Tip content={running ? `Send now: the agent takes it at its next step (${STEER_KEY}+Enter on an empty box sends the first)` : "Send now"}>
              <button className="rounded-md flex h-6 shrink-0 items-center gap-1 px-1.5 text-[11.5px] text-muted-foreground hover:bg-accent hover:text-foreground" onClick={() => void chats.sendQueued(chatId, q.id)}>
                <CornerDownRight className="size-3" /> Send now
              </button>
            </Tip>
            <Tip content="Edit: back to the box">
              <button
                className="rounded-md grid size-6 shrink-0 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground"
                aria-label="Edit queued message"
                onClick={() => {
                  const item = chats.unqueue(chatId, q.id)
                  if (item) editInComposer(item.text, item.images.map((img) => ({ mime: img.mime, data: img.data, name: img.name ?? "image" })))
                }}
              >
                <Pencil className="size-3" />
              </button>
            </Tip>
            <Tip content="Remove">
              <button className="rounded-md grid size-6 shrink-0 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Remove queued message" onClick={() => chats.unqueue(chatId, q.id)}>
                <X className="size-3.5" />
              </button>
            </Tip>
          </li>
        ))}
      </ul>
    </div>
  )
}

/** The agent's checklist, folded to its current step. */
function PlanDock({ items }: { items: PlanItem[] }) {
  const [open, setOpen] = useState(false)
  const done = items.filter((i) => i.status === "completed").length
  const current = items.find((i) => i.status === "inProgress") ?? items.find((i) => i.status === "pending")
  return (
    <div className="overflow-hidden rounded-2xl border bg-card text-[12.5px]">
      <button className="flex h-8 w-full items-center gap-2 px-4 text-left hover:bg-accent" onClick={() => setOpen(!open)}>
        {open ? <ChevronDown className="size-3.5 text-faint" /> : <ChevronRight className="size-3.5 text-faint" />}
        <span className="font-medium">Plan</span>
        <span className="font-mono text-[11px] text-faint">
          {done}/{items.length}
        </span>
        {!open && current ? <span className="truncate text-muted-foreground">{current.text}</span> : null}
      </button>
      {open ? (
        <ul className="border-t px-3 py-2">
          {items.map((it, i) => (
            <li key={i} className="flex items-center gap-2 py-0.5">
              {it.status === "completed" ? (
                <Check className="size-3.5 text-green" />
              ) : it.status === "inProgress" ? (
                <LoaderCircle className="size-3.5 animate-spin text-muted-foreground" />
              ) : (
                <Circle className="size-3.5 text-faint" />
              )}
              <span className={cn(it.status === "completed" && "text-muted-foreground line-through")}>{it.text}</span>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  )
}

function NoProject() {
  const openSettings = useApp((s) => s.openSettings)
  const { distros } = useDistros()
  return (
    <div className="flex h-full flex-col items-center justify-center bg-background px-6 pb-[10vh]">
      <Mark size={48} label="oah" />
      <h1 className="mt-5 text-[22px] font-semibold tracking-tight">Open Agent Harness</h1>
      <p className="mt-2 max-w-md text-center text-[13px] leading-relaxed text-muted-foreground">
        Chat with Claude Code, Codex and OpenCode in your projects: on this computer, in WSL, or on an SSH host. Add a project to start.
      </p>
      <div className="mt-6 flex gap-2">
        <Button onClick={() => void addLocalFolder()}>
          <FolderPlus /> Open folder
        </Button>
        {distros?.length ? (
          <OpenProjectMenu>
            <Button variant="outline">
              <LocationIcon location={{ kind: "wsl", distro: "" }} /> Folder in WSL <ChevronDown />
            </Button>
          </OpenProjectMenu>
        ) : null}
        <Button variant="outline" onClick={() => openSettings("ssh")}>
          <Server /> Folder on an SSH host
        </Button>
      </div>
    </div>
  )
}
