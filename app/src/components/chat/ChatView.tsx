import { useEffect, useState } from "react"
import { Check, ChevronDown, ChevronRight, Circle, FolderPlus, LoaderCircle, MoreHorizontal, Server, Trash2 } from "lucide-react"
import { AgentGlyph, Mark } from "@/components/icons"
import { Tip } from "@/components/Tip"
import { Button } from "@/components/ui/button"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { emptyTranscript, formatTokens, type ChatMeta, type PlanItem, type Transcript as T } from "@/lib/chat"
import { pickFolder } from "@/lib/ipc"
import { AGENT_LABEL, locationLabel, type AgentKind, type Project } from "@/lib/types"
import { cn } from "@/lib/utils"
import { useApp, useProject } from "@/store/app"
import { useChats } from "@/store/chats"
import { ApprovalCard } from "./Approval"
import { Composer } from "./Composer"
import { Transcript } from "./Transcript"

/** The middle of the window: the open chat, or a new one in the current project. */
export function ChatView() {
  const settings = useApp((s) => s.settings)
  const chatId = useApp((s) => s.chatId)
  const project = useProject()
  const meta = useChats((s) => s.metas.find((m) => m.id === chatId))
  const transcript = useChats((s) => (chatId ? s.transcripts[chatId] : undefined))

  useEffect(() => {
    if (chatId && !transcript) useChats.getState().open(chatId)
  }, [chatId, transcript])

  if (!settings) return null
  if (!project) return <NoProject />
  if (chatId && meta && meta.projectId === project.id) return <OpenChat project={project} meta={meta} transcript={transcript ?? emptyTranscript()} />
  return <NewChat project={project} />
}

function useAgentDefaults(project: Project, agent: AgentKind) {
  const settings = useApp((s) => s.settings)!
  const provider = settings.providers[agent]
  return { model: project.defaultModel || provider.defaultModel, permission: provider.permissionMode }
}

function NewChat({ project }: { project: Project }) {
  const settings = useApp((s) => s.settings)!
  const [agent, setAgent] = useState<AgentKind>(project.defaultAgent ?? settings.providers.defaultAgent)
  const d = useAgentDefaults(project, agent)
  const [permission, setPermission] = useState(d.permission)
  const send = (text: string) => useChats.getState().send(null, text, { agent, permission })

  return (
    <div className="flex h-full min-w-0 flex-col bg-background">
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center px-6 pb-[12vh]">
        <div className="w-full max-w-[46rem]">
          <div className="mb-6 flex items-end gap-3">
            <Mark size={34} />
            <div>
              <h1 className="text-[22px] leading-tight font-semibold tracking-tight">What should we work on?</h1>
              <p className="mt-1 text-[12.5px] text-muted-foreground">
                {AGENT_LABEL[agent]} in <span className="font-medium text-foreground">{project.name}</span>
                <span className="text-faint"> · {locationLabel(project.location)}</span>
              </p>
            </div>
          </div>
          <Composer agent={agent} onAgent={setAgent} model={d.model} permission={permission} onPermission={setPermission} onSend={send} autoFocus />
          <div className="mt-3 grid grid-cols-3 gap-px border bg-border text-[12px]">
            {["Explain how this project is laid out", "Find and fix a failing test", "Review my uncommitted changes"].map((t) => (
              <button key={t} className="bg-background px-3 py-2.5 text-left text-muted-foreground hover:bg-accent hover:text-foreground" onClick={() => send(t)}>
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
  const d = useAgentDefaults(project, meta.agent)
  const permission = useChats((s) => s.permission[meta.id]) ?? d.permission
  const chats = useChats.getState()

  return (
    <div className="flex h-full min-w-0 flex-col bg-background">
      <ChatHeader meta={meta} transcript={transcript} model={transcript.model || meta.model || d.model} />
      <Transcript transcript={transcript} className="flex-1" />
      <div className="mx-auto flex w-full max-w-[46rem] flex-col gap-2 px-6 pb-4">
        {transcript.plan.length ? <PlanDock items={transcript.plan} /> : null}
        {transcript.pending ? <ApprovalCard request={transcript.pending} onAnswer={(dec) => chats.answer(meta.id, transcript.pending!.requestId, dec)} /> : null}
        <Composer
          agent={meta.agent}
          model={transcript.model || meta.model || d.model}
          permission={permission}
          onPermission={(m) => chats.setPermission(meta.id, m)}
          running={transcript.running}
          onSend={(text) => chats.send(meta.id, text, { agent: meta.agent, permission })}
          onStop={() => chats.interrupt(meta.id)}
          placeholder={transcript.running ? "Queue a follow-up…" : "Reply, @ to add files"}
          lockAgent
          autoFocus
        />
      </div>
    </div>
  )
}

function ChatHeader({ meta, transcript, model }: { meta: ChatMeta; transcript: T; model: string }) {
  const ctx = transcript.context
  const pct = ctx ? Math.min(100, (ctx.used / ctx.window) * 100) : 0
  return (
    <div className="flex h-10 shrink-0 items-center gap-2 border-b px-4">
      <AgentGlyph agent={meta.agent} className="size-3.5" />
      <span className="truncate text-[13px] font-medium">{meta.title || "New chat"}</span>
      {transcript.running ? <LoaderCircle className="size-3.5 animate-spin text-gold" /> : null}
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
          <button className="grid size-7 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Chat menu">
            <MoreHorizontal className="size-4" />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-48">
          <DropdownMenuItem
            onSelect={() => {
              const t = window.prompt("Rename chat", meta.title)
              if (t) useChats.getState().rename(meta.id, t)
            }}
          >
            Rename
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => navigator.clipboard?.writeText(meta.agentSessionId ?? "")} disabled={!meta.agentSessionId}>
            Copy {AGENT_LABEL[meta.agent]} session id
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
    <div className="border bg-card text-[12.5px]">
      <button className="flex h-8 w-full items-center gap-2 px-3 text-left hover:bg-accent" onClick={() => setOpen(!open)}>
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
                <LoaderCircle className="size-3.5 animate-spin text-gold" />
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
  const addProject = useApp((s) => s.addProject)
  const openSettings = useApp((s) => s.openSettings)
  return (
    <div className="flex h-full flex-col items-center justify-center bg-background px-6 pb-[10vh]">
      <Mark size={48} label="oah" />
      <h1 className="mt-5 text-[22px] font-semibold tracking-tight">Open Agent Harness</h1>
      <p className="mt-2 max-w-md text-center text-[13px] leading-relaxed text-muted-foreground">
        Chat with Claude Code and Codex in your projects, on this computer or on an SSH host. Add a project to start.
      </p>
      <div className="mt-6 flex gap-2">
        <Button
          onClick={async () => {
            const path = await pickFolder("Add a project folder")
            if (path) await addProject({ kind: "local" }, path)
          }}
        >
          <FolderPlus /> Open folder
        </Button>
        <Button variant="outline" onClick={() => openSettings("ssh")}>
          <Server /> Folder on an SSH host
        </Button>
      </div>
    </div>
  )
}
