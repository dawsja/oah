import { useEffect, useState } from "react"
import { Check, ChevronDown, ChevronRight, Circle, FolderPlus, LoaderCircle, MoreHorizontal, Server, Trash2 } from "lucide-react"
import { AgentGlyph, Mark } from "@/components/icons"
import { promptText } from "@/components/Prompt"
import { Tip } from "@/components/Tip"
import { Button } from "@/components/ui/button"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { agentChoices, agentKey, agentName, providerFor, type AgentRef } from "@/lib/agents"
import { emptyTranscript, formatTokens, type ChatMeta, type ModelOption, type SlashCommand, type PlanItem, type Transcript as T } from "@/lib/chat"
import { copyText } from "@/lib/clipboard"
import { LocationIcon } from "@/components/LocationIcon"
import { addLocalFolder, OpenProjectMenu, useDistros } from "@/components/OpenProject"
import { ipc, type Catalog } from "@/lib/ipc"
import { locationLabel, type Project } from "@/lib/types"
import { cn } from "@/lib/utils"
import { useApp, useProject } from "@/store/app"
import { useChats } from "@/store/chats"
import { useGit, useScope } from "@/store/git"
import { useStartOpenDistro } from "@/store/wsl"
import { Viewer } from "./Viewer"
import { ApprovalCard } from "./Approval"
import { QuestionCard } from "./QuestionCard"
import { Composer } from "./Composer"
import { CwdContext } from "./cwd"
import { Transcript } from "./Transcript"

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
  if (chatId && meta && meta.projectId === project.id) return <OpenChat project={project} meta={meta} transcript={transcript ?? emptyTranscript()} />
  return <NewChat project={project} />
}

function useAgentDefaults(project: Project, agent: AgentRef) {
  const settings = useApp((s) => s.settings)!
  const provider = providerFor(settings, agent)
  return { model: project.defaultModel || provider?.defaultModel || "", permission: provider?.permissionMode ?? "ask" }
}

/** Each agent's models and slash commands, asked once per project (and kept for the session). */
const catalogCache = new Map<string, Promise<Catalog>>()
/** The commands a running chat of each agent last listed, so a new chat has them at once. */
const seenCommands = new Map<string, SlashCommand[]>()
function useCatalog(agent: AgentRef, projectId: string, live: { models: ModelOption[]; commands: SlashCommand[] } = { models: [], commands: [] }) {
  const key = `${agentKey(agent)}|${projectId}`
  const [fetched, setFetched] = useState<Catalog | undefined>(undefined)
  useEffect(() => setFetched(undefined), [key])
  if (live.commands.length) seenCommands.set(key, live.commands)
  const load = () => {
    if (agent.kind === "acp") return setFetched({ models: [], commands: [] })
    if (!catalogCache.has(key)) catalogCache.set(key, ipc.providerCatalog(agent.kind, projectId).catch(() => ({ models: [], commands: [] })))
    catalogCache.get(key)!.then(setFetched)
  }
  const models = live.models.length ? live.models : fetched?.models
  const commands = live.commands.length ? live.commands : (seenCommands.get(key) ?? fetched?.commands ?? [])
  return { models, commands, load }
}

function NewChat({ project }: { project: Project }) {
  const settings = useApp((s) => s.settings)!
  const choices = agentChoices(settings)
  const preferred = project.defaultAgent ?? settings.providers.defaultAgent
  const [agent, setAgent] = useState<AgentRef>(choices.find((c) => c.kind === preferred) ?? choices[0] ?? { kind: "claude", acpId: null })
  const d = useAgentDefaults(project, agent)
  const [permission, setPermission] = useState(d.permission)
  const [model, setModel] = useState<string | null>(null)
  const [effort, setEffort] = useState("")
  const { models, commands, load } = useCatalog(agent, project.id)
  const { distro, starting } = useStartOpenDistro(project.location)
  const draftCwd = useApp((s) => s.draftCwd)
  const worktree = useGit((s) => s.worktrees[project.id]?.find((w) => !w.main && w.path === draftCwd))
  const send = (text: string, images: { mime: string; data: string }[] = []) =>
    useChats.getState().send(null, text, { agent: agent.kind, acpId: agent.acpId, model: model ?? undefined, effort: effort || undefined, permission, cwd: draftCwd ?? undefined, images })

  return (
    <div className="flex h-full min-w-0 flex-col bg-background">
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center px-6 pb-[12vh]">
        <div className="w-full max-w-[46rem]">
          <div className="mb-6 flex items-end gap-3">
            <Mark size={34} />
            <div>
              <h1 className="text-[22px] leading-tight font-semibold tracking-tight">What should we work on?</h1>
              <p className="mt-1 text-[12.5px] text-muted-foreground">
                {agentName(settings, agent)} in <span className="font-medium text-foreground">{project.name}</span>
                {worktree ? (
                  <span className="rounded-sm ml-1 border px-1 font-mono text-[11px] text-muted-foreground">⎇ {worktree.branch}</span>
                ) : null}
                <span className="text-faint"> · {locationLabel(project.location)}</span>
                {starting ? (
                  <span className="ml-1.5 inline-flex items-center gap-1 text-muted-foreground">
                    <LoaderCircle className="size-3 animate-spin" /> starting {distro}…
                  </span>
                ) : null}
              </p>
            </div>
          </div>
          <Composer
            agent={agent}
            agents={choices}
            onAgent={(a) => {
              setAgent(a)
              setModel(null)
              setEffort("")
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
              <button key={t} className="rounded-lg border bg-card px-3 py-2.5 text-left text-muted-foreground hover:border-ring/60 hover:bg-accent hover:text-foreground" onClick={() => send(t)}>
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
  const agent: AgentRef = { kind: meta.agent, acpId: meta.acpAgent }
  const d = useAgentDefaults(project, agent)
  const { models, commands, load } = useCatalog(agent, project.id, transcript)
  const permission = useChats((s) => s.permission[meta.id]) ?? d.permission
  const chats = useChats.getState()

  return (
    <CwdContext.Provider value={transcript.cwd ?? meta.cwd}>
    <div className="flex h-full min-w-0 flex-col bg-background">
      <ChatHeader meta={meta} transcript={transcript} model={transcript.model || meta.model || d.model} />
      <Transcript transcript={transcript} className="flex-1" />
      <div className="mx-auto flex w-full max-w-[46rem] flex-col gap-2 px-6 pb-4">
        {transcript.plan.length ? <PlanDock items={transcript.plan} /> : null}
        {transcript.questions[0] ? (
          <QuestionCard request={transcript.questions[0]} onAnswer={(a) => chats.answerQuestion(meta.id, transcript.questions[0].requestId, a)} />
        ) : transcript.approvals[0] ? (
          <ApprovalCard request={transcript.approvals[0]} queued={transcript.approvals.length - 1} onAnswer={(dec) => chats.answer(meta.id, transcript.approvals[0].requestId, dec)} />
        ) : null}
        <Composer
          agent={agent}
          model={meta.model || transcript.model || d.model}
          effort={meta.effort}
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
          onSend={(text, images) => chats.send(meta.id, text, { agent: meta.agent, acpId: meta.acpAgent, permission, images })}
          onStop={() => chats.interrupt(meta.id)}
          placeholder={transcript.running ? "Queue a follow-up…" : "Reply, @ to add files, / for commands"}
          lockAgent
          autoFocus
        />
      </div>
    </div>
    </CwdContext.Provider>
  )
}

function ChatHeader({ meta, transcript, model }: { meta: ChatMeta; transcript: T; model: string }) {
  const ctx = transcript.context
  const pct = ctx ? Math.min(100, (ctx.used / ctx.window) * 100) : 0
  return (
    <div className="flex h-10 shrink-0 items-center gap-2 border-b px-4">
      <AgentGlyph agent={meta.agent} className="size-3.5" />
      <span className="truncate text-[13px] font-medium">{meta.title || "New chat"}</span>
      {transcript.running ? <LoaderCircle className="size-3.5 animate-spin text-muted-foreground" /> : null}
      <span className="ml-auto font-mono text-[11px] text-faint">{model}</span>
      {ctx ? (
        <Tip content={`Context: ${formatTokens(ctx.used)} of ${formatTokens(ctx.window)} tokens`} side="bottom">
          <div className="flex items-center gap-1.5">
            <div className="flex h-2 w-16 gap-px">
              {Array.from({ length: 10 }, (_, i) => (
                <span key={i} className={cn("flex-1", i < Math.round(pct / 10) ? (pct > 85 ? "bg-red" : pct > 65 ? "bg-warn" : "bg-primary") : "bg-muted")} />
              ))}
            </div>
            <span className="font-mono text-[11px] text-muted-foreground">{Math.round(pct)}%</span>
          </div>
        </Tip>
      ) : null}
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
          <DropdownMenuItem onSelect={() => copyText(meta.agentSessionId ?? "")} disabled={!meta.agentSessionId}>
            Copy the agent's session id
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem variant="destructive" onSelect={() => useChats.getState().remove(meta.id)}>
            <Trash2 /> Delete chat
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  )
}

/** The agent's checklist, folded to its current step. */
function PlanDock({ items }: { items: PlanItem[] }) {
  const [open, setOpen] = useState(false)
  const done = items.filter((i) => i.status === "completed").length
  const current = items.find((i) => i.status === "inProgress") ?? items.find((i) => i.status === "pending")
  return (
    <div className="rounded-lg border bg-card text-[12.5px]">
      <button className="rounded-lg flex h-8 w-full items-center gap-2 px-3 text-left hover:bg-accent" onClick={() => setOpen(!open)}>
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
