import { useEffect, useMemo, useState } from "react"
import { Loader2, X } from "lucide-react"
import { ApprovalCard } from "@/components/chat/Approval"
import { ImageSourceContext, type ImageSource } from "@/components/chat/ChatImages"
import { QuestionCard } from "@/components/chat/QuestionCard"
import { TasksDock } from "@/components/chat/TasksDock"
import { Transcript } from "@/components/chat/Transcript"
import { CwdContext } from "@/components/chat/cwd"
import { emptyTranscript, type ModelOption } from "@/lib/chat"
import { AGENT_LABEL } from "@/lib/types"
import { cn } from "@/lib/utils"
import { MobileComposer } from "./Composer"
import { api, type AgentInfo } from "./rpc"
import { go } from "./route"
import { useMobile } from "./store"
import { LinkBanner, TopBar } from "./ui"

const NONE: never[] = []

/** Models for a chat: what its agent said, else the agent's catalog (loaded when asked). */
function useModels(agent: AgentInfo | undefined, projectId: string | null, live: ModelOption[]) {
  const [catalog, setCatalog] = useState<ModelOption[]>([])
  const load = () => {
    if (!agent || live.length) return
    void useMobile.getState().catalog(agent, projectId).then((c) => c && setCatalog(c.models))
  }
  return { models: live.length ? live : catalog, load }
}

export function ChatScreen({ chatId }: { chatId: string }) {
  const desktopId = useMobile((s) => s.desktopId)
  const meta = useMobile((s) => s.metas.find((m) => m.id === chatId))
  const transcript = useMobile((s) => s.transcripts[chatId])
  const link = useMobile((s) => s.link)
  const queued = useMobile((s) => s.queued[chatId]) ?? NONE
  const agents = useMobile((s) => s.agents)
  const permission = useMobile((s) => s.permission[chatId])
  const project = useMobile((s) => s.projects.find((p) => p.id === meta?.projectId))
  const st = useMobile.getState()

  useEffect(() => {
    void st.open(chatId)
    return () => useMobile.getState().leave()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chatId])

  const agent = agents.find((a) => a.kind === meta?.agent)
  const t = transcript ?? emptyTranscript()
  const { models, load } = useModels(agent, meta?.projectId ?? null, t.models)
  const source = useMemo<ImageSource>(
    () => ({ key: `phone:${desktopId}:${chatId}`, chatImage: (file) => api.image(chatId, file), readImage: (path) => api.readImage(chatId, path) }),
    [desktopId, chatId],
  )
  const model = meta?.model || t.model || agent?.defaultModel || ""
  const modelName = models.find((m) => m.id === model)?.name ?? model

  return (
    <div className="safe flex h-full flex-col bg-background">
      <TopBar
        backable
        title={meta?.title || "New chat"}
        subtitle={[project?.name, meta ? (agent?.name ?? AGENT_LABEL[meta.agent]) : null, modelName].filter(Boolean).join(" · ")}
        actions={t.running ? <Loader2 className="mr-3 size-4 animate-spin text-primary" aria-label="Working" /> : null}
      />
      <LinkBanner />
      <CwdContext.Provider value={t.cwd ?? meta?.cwd ?? null}>
        <ImageSourceContext.Provider value={source}>
          {transcript ? <Transcript transcript={t} className="flex-1" /> : <div className="grid flex-1 place-items-center text-[13px] text-muted-foreground">{link === "connected" ? "Loading…" : ""}</div>}
          <div className="flex flex-col gap-2 px-2.5 pt-1 [&:not(:empty)]:pb-2">
            {t.tasks.length && meta ? <TasksDock agent={meta.agent} tasks={t.tasks} /> : null}
            {t.questions[0] ? (
              <QuestionCard request={t.questions[0]} onAnswer={(a) => st.answerQuestion(chatId, t.questions[0].requestId, a)} />
            ) : t.approvals[0] ? (
              <ApprovalCard request={t.approvals[0]} queued={t.approvals.length - 1} onAnswer={(d) => st.answer(chatId, t.approvals[0].requestId, d)} />
            ) : null}
            {queued.length ? (
              <div className="rounded-2xl divide-y border bg-card">
                {queued.map((q, i) => (
                  <div key={q.id} className="flex items-center gap-2 py-1 pr-1 pl-3 text-[13px]">
                    <span className="text-[11px] text-faint">{i === 0 ? "Next" : "Queued"}</span>
                    <span className="min-w-0 flex-1 truncate">{q.text || `${q.images.length} image${q.images.length > 1 ? "s" : ""}`}</span>
                    {i === 0 ? (
                      <button className="rounded-md h-8 px-2 text-[12px] font-medium text-primary active:bg-accent" onClick={() => void st.sendQueued(chatId)}>
                        Send now
                      </button>
                    ) : null}
                    <button className="rounded-md grid size-8 place-items-center text-muted-foreground active:bg-accent" aria-label="Remove" onClick={() => st.unqueue(chatId, q.id)}>
                      <X className="size-4" />
                    </button>
                  </div>
                ))}
              </div>
            ) : null}
          </div>
        </ImageSourceContext.Provider>
      </CwdContext.Provider>
      <MobileComposer
        running={t.running}
        disabled={link !== "connected"}
        placeholder={t.running ? "Queue a follow-up" : "Reply"}
        onSend={(text, images, how) => {
          const running = useMobile.getState().transcripts[chatId]?.running
          // A command never goes into a turn that's running: it waits its turn.
          if (running && how === "now" && !text.startsWith("/")) void st.steer(chatId, text, images)
          else if (running) st.enqueue(chatId, text, images)
          else void st.send(chatId, text, images)
        }}
        onStop={() => void st.interrupt(chatId)}
        models={models}
        model={model}
        effort={meta?.effort ?? ""}
        onModel={(m, e) => void st.setModel(chatId, m, e)}
        onModelsOpen={load}
        permission={permission ?? agent?.permissionMode ?? null}
        onPermission={(m) => st.setPermission(chatId, m)}
        context={t.context}
        onCompact={() => void st.compact(chatId)}
        onClear={() => void st.clear(chatId)}
      />
    </div>
  )
}

/** A chat that doesn't exist yet: pick the agent, then the first message creates it. */
export function NewChatScreen({ projectId }: { projectId: string }) {
  const project = useMobile((s) => s.projects.find((p) => p.id === projectId))
  const agents = useMobile((s) => s.agents)
  const defaultAgent = useMobile((s) => s.defaultAgent)
  const link = useMobile((s) => s.link)
  const preferred = project?.defaultAgent ?? defaultAgent
  const [pick, setPick] = useState<string | null>(null)
  const agent = agents.find((a) => a.kind === pick) ?? agents.find((a) => a.kind === preferred) ?? agents[0]
  const [model, setModel] = useState("")
  const { models, load } = useModels(agent, projectId, NONE)
  const [sending, setSending] = useState(false)

  return (
    <div className="safe flex h-full flex-col bg-background">
      <TopBar backable title="New chat" subtitle={project?.name} />
      <LinkBanner />
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-6">
        <h3 className="mb-2 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">Agent</h3>
        <div className="flex flex-wrap gap-1.5">
          {agents.map((a) => {
            return (
              <button
                key={a.kind}
                className={cn("rounded-md h-10 border px-3 text-[13px]", agent?.kind === a.kind ? "border-primary bg-primary text-primary-foreground" : "active:bg-accent")}
                onClick={() => (setPick(a.kind), setModel(""))}
              >
                {a.name}
              </button>
            )
          })}
          {agents.length === 0 ? <div className="text-[13px] text-muted-foreground">No agents are turned on in the desktop's Settings → Providers.</div> : null}
        </div>
        <p className="mt-6 text-[12.5px] leading-relaxed text-muted-foreground">
          It works in {project ? <span className="font-mono text-foreground">{project.path}</span> : "the project"} on the desktop. The chat shows up there too.
        </p>
      </div>
      <MobileComposer
        running={false}
        disabled={link !== "connected" || !agent || sending}
        placeholder="What should it do?"
        onSend={async (text, images) => {
          setSending(true)
          const id = await useMobile.getState().send(null, text, images, { projectId, agent, model })
          setSending(false)
          if (id) go(`/chat/${encodeURIComponent(id)}`, true)
        }}
        models={models}
        model={model || agent?.defaultModel || ""}
        effort=""
        onModel={(m) => setModel(m)}
        onModelsOpen={load}
        permission={null}
      />
    </div>
  )
}
