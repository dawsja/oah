// Chat events (crates/oah-agents/src/event.rs) and the transcript they fold into. `reduce` is
// pure: live events and a reopened chat's log go through the same path.

import type { AgentKind, PermissionMode } from "./types"

export type ToolKind = "read" | "edit" | "write" | "bash" | "search" | "web" | "task" | "other"
export type ToolCall = { kind: ToolKind; name: string; title: string; input: unknown; path: string | null }
export type FileDiff = { path: string; patch: string; added: number; removed: number }
export type Decision = "allow" | "allowAlways" | "deny"
export type ApprovalRequest = {
  requestId: string
  itemId: string | null
  kind: "command" | "edit" | "plan" | "other"
  title: string
  detail: string | null
  diff: FileDiff | null
  canRemember: boolean
}
export type PlanItem = { text: string; status: "pending" | "inProgress" | "completed" }
export type QuestionOption = { label: string; description: string }
export type Question = { id: string; header: string; question: string; options: QuestionOption[]; multi: boolean; other: boolean; secret: boolean }
export type QuestionRequest = { requestId: string; questions: Question[] }
/** `efforts`: the reasoning effort levels the model takes, in order (empty: none to pick). */
export type ModelOption = { id: string; name: string; description: string; efforts: string[]; defaultEffort: string | null }

/** "xhigh" → "Extra high". */
export function effortLabel(e: string): string {
  const named: Record<string, string> = { xhigh: "Extra high", max: "Max", minimal: "Minimal", none: "None" }
  return named[e] ?? e.charAt(0).toUpperCase() + e.slice(1)
}
/** `skill`: a Codex skill, mentioned as `$name` instead of run as `/name`. */
/** Part of the context window: `kind` is used, deferred (loaded when needed), buffer or free. */
export type ContextPart = { name: string; tokens: number; kind: string }
/** How full the context window is, and (when the agent says) what fills it. */
export type ContextFill = { used: number; window: number; parts: ContextPart[] }
export type SlashCommand = { name: string; description: string; hint: string; skill: boolean }
export type Usage = { inputTokens: number; outputTokens: number; cacheReadTokens: number; cacheWriteTokens: number }

export type ChatEvent =
  | { type: "started"; sessionId: string | null; model: string | null; cwd: string | null }
  | { type: "userMessage"; id: string; text: string; images: string[]; files?: string[] }
  | { type: "turnStarted"; turnId: string }
  | { type: "textDelta"; itemId: string; text: string }
  | { type: "text"; itemId: string; text: string }
  | { type: "reasoningDelta"; itemId: string; text: string }
  | { type: "reasoning"; itemId: string; text: string }
  | { type: "toolStarted"; itemId: string; tool: ToolCall }
  | { type: "toolFinished"; itemId: string; ok: boolean; output: string; diff: FileDiff | null }
  | { type: "approval"; request: ApprovalRequest }
  | { type: "approvalResolved"; requestId: string; decision: Decision }
  | { type: "question"; request: QuestionRequest }
  | { type: "questionResolved"; requestId: string; answers: Record<string, string[]> }
  | { type: "models"; models: ModelOption[]; current: string | null }
  | { type: "commands"; commands: SlashCommand[] }
  | { type: "permissionChanged"; mode: PermissionMode }
  | { type: "plan"; items: PlanItem[] }
  | { type: "turnFinished"; turnId: string; usage: Usage | null; costUsd: number | null; durationMs: number | null; interrupted: boolean }
  | { type: "context"; usedTokens: number; windowTokens: number; parts?: ContextPart[] }
  | { type: "notice"; level: "info" | "warn"; text: string }
  | { type: "error"; message: string }
  | { type: "exited"; code: number | null; message: string | null }

export type ChatMeta = {
  id: string
  projectId: string
  agent: AgentKind
  /** Reasoning effort; empty: the model's default. */
  effort: string
  /** Which added ACP agent, when `agent` is "acp". */
  acpAgent: string | null
  model: string
  title: string
  cwd: string
  agentSessionId: string | null
  createdAt: number
  updatedAt: number
  tokens: number
  costUsd: number
  /** Listed first in its project. */
  sleeping?: boolean
  pinned: boolean
}

export type ToolItem = { kind: "tool"; id: string; tool: ToolCall; status: "running" | "ok" | "error"; output: string; diff: FileDiff | null }
export type Item =
  | { kind: "user"; id: string; text: string; images: string[]; files: string[] }
  | { kind: "assistant"; id: string; text: string }
  | { kind: "reasoning"; id: string; text: string }
  | ToolItem
  | { kind: "approval"; id: string; request: ApprovalRequest; decision: Decision | null }
  | { kind: "question"; id: string; request: QuestionRequest; answers: Record<string, string[]> | null }
  | { kind: "turnEnd"; id: string; usage: Usage | null; costUsd: number | null; durationMs: number | null; interrupted: boolean }
  | { kind: "notice"; id: string; level: "info" | "warn" | "error"; text: string }

export type Transcript = {
  items: Item[]
  running: boolean
  plan: PlanItem[]
  context: ContextFill | null
  model: string | null
  /** The agent's working folder, to show paths relative to it. */
  cwd: string | null
  /** Approvals waiting for an answer, oldest first (the first one is shown). */
  approvals: ApprovalRequest[]
  /** Questions waiting for an answer, oldest first. */
  questions: QuestionRequest[]
  /** What the agent offers; empty until it says. */
  models: ModelOption[]
  /** The agent's slash commands, as it last listed them. */
  commands: SlashCommand[]
}

export const emptyTranscript = (): Transcript => ({ items: [], running: false, plan: [], context: null, model: null, cwd: null, approvals: [], questions: [], models: [], commands: [] })

/** Something is waiting for the user. */
export const waiting = (t: Transcript | undefined) => !!t && (t.approvals.length > 0 || t.questions.length > 0)

/** What a chat waits on the user for: an approval, a plan to approve, or a question. */
export type Waiting = "approval" | "plan" | "question"

/** What the transcript waits for, if anything (the card shown first: a question, else an approval). */
export function waitingFor(t: Transcript): Waiting | null {
  if (t.questions.length) return "question"
  const a = t.approvals[0]
  return a ? (a.kind === "plan" ? "plan" : "approval") : null
}

/** What the chat waits on the user for: its transcript knows when it's loaded in this window
 *  (kept current by events), the app's list (`chat_waiting`) otherwise. */
export function waitingOf(s: { transcripts: Record<string, Transcript>; waiting: Record<string, Waiting> }, chatId: string): Waiting | null {
  const t = s.transcripts[chatId]
  return t ? waitingFor(t) : (s.waiting[chatId] ?? null)
}

/** The chats waiting on the user, newest activity first. */
export function needsYou(s: { metas: ChatMeta[]; transcripts: Record<string, Transcript>; waiting: Record<string, Waiting> }): string[] {
  return s.metas.filter((m) => waitingOf(s, m.id)).map((m) => m.id)
}

/** A chat found by its text (`chat_search`): the line around the match, and where it was. */
export type SearchHit = { chatId: string; part: "user" | "agent" | "tool"; snippet: string; matches: number }

let seq = 0
const uid = (p: string) => `${p}-${++seq}`

/** Folds one event into the transcript (returns a new object; items are replaced, not mutated). */
export function reduce(t: Transcript, e: ChatEvent): Transcript {
  const items = t.items
  const find = (id: string, kind: Item["kind"]) => {
    for (let i = items.length - 1; i >= 0; i--) if (items[i].id === id && items[i].kind === kind) return i
    return -1
  }
  const replace = (i: number, item: Item) => ({ ...t, items: [...items.slice(0, i), item, ...items.slice(i + 1)] })
  const push = (item: Item, patch: Partial<Transcript> = {}) => ({ ...t, ...patch, items: [...items, item] })

  switch (e.type) {
    case "started":
      return { ...t, model: e.model ?? t.model, cwd: e.cwd ?? t.cwd }
    case "userMessage":
      return push({ kind: "user", id: e.id, text: e.text, images: e.images, files: e.files ?? [] }, { running: true })
    case "turnStarted":
      return { ...t, running: true }
    case "textDelta":
    case "text": {
      const i = find(e.itemId, "assistant")
      if (i < 0) return push({ kind: "assistant", id: e.itemId, text: e.text })
      const cur = items[i] as Extract<Item, { kind: "assistant" }>
      return replace(i, { ...cur, text: e.type === "text" ? e.text : cur.text + e.text })
    }
    case "reasoningDelta":
    case "reasoning": {
      const i = find(e.itemId, "reasoning")
      if (i < 0) return push({ kind: "reasoning", id: e.itemId, text: e.text })
      const cur = items[i] as Extract<Item, { kind: "reasoning" }>
      return replace(i, { ...cur, text: e.type === "reasoning" ? e.text : cur.text + e.text })
    }
    case "toolStarted": {
      const i = find(e.itemId, "tool")
      const item: ToolItem = { kind: "tool", id: e.itemId, tool: e.tool, status: "running", output: "", diff: null }
      return i < 0 ? push(item) : replace(i, { ...(items[i] as ToolItem), tool: e.tool })
    }
    case "toolFinished": {
      const i = find(e.itemId, "tool")
      if (i < 0) return t
      const cur = items[i] as ToolItem
      return replace(i, { ...cur, status: e.ok ? "ok" : "error", output: e.output, diff: e.diff ?? cur.diff })
    }
    case "approval":
      return push({ kind: "approval", id: e.request.requestId, request: e.request, decision: null }, { approvals: [...t.approvals, e.request] })
    case "approvalResolved": {
      const i = find(e.requestId, "approval")
      const approvals = t.approvals.filter((a) => a.requestId !== e.requestId)
      if (i < 0) return { ...t, approvals }
      return { ...replace(i, { ...(items[i] as Extract<Item, { kind: "approval" }>), decision: e.decision }), approvals }
    }
    case "question":
      return push({ kind: "question", id: e.request.requestId, request: e.request, answers: null }, { questions: [...t.questions, e.request] })
    case "questionResolved": {
      const i = find(e.requestId, "question")
      const questions = t.questions.filter((q) => q.requestId !== e.requestId)
      if (i < 0) return { ...t, questions }
      return { ...replace(i, { ...(items[i] as Extract<Item, { kind: "question" }>), answers: e.answers }), questions }
    }
    case "models":
      // Logs from before effort levels have models without them.
      return { ...t, models: e.models.map((m) => ({ ...m, efforts: m.efforts ?? [], defaultEffort: m.defaultEffort ?? null })), model: e.current ?? t.model }
    case "commands":
      return { ...t, commands: e.commands }
    case "permissionChanged":
      return t
    case "plan":
      return { ...t, plan: e.items }
    case "turnFinished": {
      // Tools still "running" when the turn ends were cut off.
      const settled = items.map((it) => (it.kind === "tool" && it.status === "running" ? { ...it, status: "error" as const, output: it.output || "Stopped." } : it))
      return {
        ...t,
        running: false,
        approvals: [],
        questions: [],
        // A restarted CLI counts turns from 1 again: the position keeps the id unique.
        items: [...settled, { kind: "turnEnd", id: `${e.turnId}@${items.length}`, usage: e.usage, costUsd: e.costUsd, durationMs: e.durationMs, interrupted: e.interrupted }],
      }
    }
    case "context":
      // A total without a breakdown keeps the last breakdown until a new one comes.
      return { ...t, context: { used: e.usedTokens, window: e.windowTokens, parts: e.parts?.length ? e.parts : (t.context?.parts ?? []) } }
    case "notice":
      return push({ kind: "notice", id: uid("n"), level: e.level, text: e.text })
    case "error":
      return push({ kind: "notice", id: uid("e"), level: "error", text: e.message }, { running: false, approvals: [], questions: [] })
    case "exited":
      return e.code && e.code !== 0
        ? push({ kind: "notice", id: uid("x"), level: "error", text: e.message ?? `The agent exited (code ${e.code}).` }, { running: false, approvals: [], questions: [] })
        : { ...t, running: false, approvals: [], questions: [] }
  }
}

/** A chat from its events. `busy`: its agent is mid-turn right now (a snapshot says so), so it
 *  stays running with its open approvals and questions; otherwise it's idle, whatever the log
 *  ended with. */
export function replay(events: ChatEvent[], busy = false): Transcript {
  const t = events.reduce(reduce, emptyTranscript())
  return busy ? { ...t, running: true } : { ...t, running: false, approvals: [], questions: [] }
}

/** A chat as the desktop has it now (`chat_snapshot`, or a phone's `chat.open`). */
export type Snapshot = { events: ChatEvent[]; seq: number; live: boolean; busy: boolean }

/** A turn's items between its user message and its end, grouped for display: each run of
 *  reasoning and tool calls becomes one "work" block; text, approvals and notices stand alone. */
export type Block = { kind: "work"; id: string; items: Item[] } | { kind: "item"; item: Item }

export function blocks(items: Item[]): Block[] {
  const out: Block[] = []
  for (const it of items) {
    // Allowed requests need no row (the tool row shows what ran); waiting ones are docked.
    // Allowed approvals fold away (the tool row shows what ran); a plan stays, approved or not.
    if (it.kind === "approval" && it.decision !== "deny" && it.request.kind !== "plan") continue
    if (it.kind === "question" && !it.answers) continue
    const work = it.kind === "reasoning" || it.kind === "tool"
    const last = out[out.length - 1]
    if (work && last?.kind === "work") last.items.push(it)
    else if (work) out.push({ kind: "work", id: `w-${it.id}`, items: [it] })
    else out.push({ kind: "item", item: it })
  }
  return out
}

export function formatTokens(n: number): string {
  // 1M, 1.5M, 9.9k, 54k: no trailing ".0".
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(n >= 10_000 ? 0 : 1).replace(/\.0$/, "")}k`
  return String(n)
}

export function formatDuration(ms: number): string {
  const s = Math.round(ms / 1000)
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`
}

export function ago(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ms) / 1000))
  if (s < 60) return "now"
  if (s < 3600) return `${Math.floor(s / 60)}m`
  if (s < 86400) return `${Math.floor(s / 3600)}h`
  return `${Math.floor(s / 86400)}d`
}

/** `path` relative to `cwd` when it's inside it; also drops a leading `cd "<cwd>" && `. */
export function relative(text: string, cwd: string | null): string {
  if (!cwd || !text) return text
  const norm = (p: string) => p.replace(/\\/g, "/").replace(/\/+$/, "")
  const base = norm(cwd)
  let out = text
  for (const q of ['"', "'", ""]) out = out.replace(`cd ${q}${cwd}${q} && `, "").replace(`cd ${q}${base}${q} && `, "")
  const flat = norm(out)
  const win = /^[a-z]:\//i.test(base)
  if (win ? flat.toLowerCase().startsWith(base.toLowerCase() + "/") : flat.startsWith(base + "/")) return flat.slice(base.length + 1)
  // Paths inside a longer text (a command): drop the folder prefix wherever it appears.
  return out.split(cwd.replace(/[\\/]+$/, "") + (cwd.includes("\\") ? "\\" : "/")).join("")
}
