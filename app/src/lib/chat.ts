// Chat events (crates/oah-agents/src/event.rs) and the transcript they fold into. `reduce` is
// pure: live events and a reopened chat's log go through the same path.

import type { AgentKind } from "./types"

export type ToolKind = "read" | "edit" | "write" | "bash" | "search" | "web" | "task" | "other"
export type ToolCall = { kind: ToolKind; name: string; title: string; input: unknown; path: string | null }
export type FileDiff = { path: string; patch: string; added: number; removed: number }
export type Decision = "allow" | "allowAlways" | "deny"
export type ApprovalRequest = {
  requestId: string
  itemId: string | null
  kind: "command" | "edit" | "other"
  title: string
  detail: string | null
  diff: FileDiff | null
  canRemember: boolean
}
export type PlanItem = { text: string; status: "pending" | "inProgress" | "completed" }
export type Usage = { inputTokens: number; outputTokens: number; cacheReadTokens: number; cacheWriteTokens: number }

export type ChatEvent =
  | { type: "started"; sessionId: string | null; model: string | null; cwd: string | null }
  | { type: "userMessage"; id: string; text: string; images: string[] }
  | { type: "turnStarted"; turnId: string }
  | { type: "textDelta"; itemId: string; text: string }
  | { type: "text"; itemId: string; text: string }
  | { type: "reasoningDelta"; itemId: string; text: string }
  | { type: "reasoning"; itemId: string; text: string }
  | { type: "toolStarted"; itemId: string; tool: ToolCall }
  | { type: "toolFinished"; itemId: string; ok: boolean; output: string; diff: FileDiff | null }
  | { type: "approval"; request: ApprovalRequest }
  | { type: "approvalResolved"; requestId: string; decision: Decision }
  | { type: "plan"; items: PlanItem[] }
  | { type: "turnFinished"; turnId: string; usage: Usage | null; costUsd: number | null; durationMs: number | null; interrupted: boolean }
  | { type: "context"; usedTokens: number; windowTokens: number }
  | { type: "notice"; level: "info" | "warn"; text: string }
  | { type: "error"; message: string }
  | { type: "exited"; code: number | null; message: string | null }

export type ChatMeta = {
  id: string
  projectId: string
  agent: AgentKind
  model: string
  title: string
  cwd: string
  agentSessionId: string | null
  createdAt: number
  updatedAt: number
  tokens: number
  costUsd: number
}

export type ToolItem = { kind: "tool"; id: string; tool: ToolCall; status: "running" | "ok" | "error"; output: string; diff: FileDiff | null }
export type Item =
  | { kind: "user"; id: string; text: string; images: string[] }
  | { kind: "assistant"; id: string; text: string }
  | { kind: "reasoning"; id: string; text: string }
  | ToolItem
  | { kind: "approval"; id: string; request: ApprovalRequest; decision: Decision | null }
  | { kind: "turnEnd"; id: string; usage: Usage | null; costUsd: number | null; durationMs: number | null; interrupted: boolean }
  | { kind: "notice"; id: string; level: "info" | "warn" | "error"; text: string }

export type Transcript = {
  items: Item[]
  running: boolean
  plan: PlanItem[]
  context: { used: number; window: number } | null
  model: string | null
  /** The approval waiting for an answer, if any. */
  pending: ApprovalRequest | null
}

export const emptyTranscript = (): Transcript => ({ items: [], running: false, plan: [], context: null, model: null, pending: null })

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
      return { ...t, model: e.model ?? t.model }
    case "userMessage":
      return push({ kind: "user", id: e.id, text: e.text, images: e.images }, { running: true })
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
      return push({ kind: "approval", id: e.request.requestId, request: e.request, decision: null }, { pending: e.request })
    case "approvalResolved": {
      const i = find(e.requestId, "approval")
      const pending = t.pending?.requestId === e.requestId ? null : t.pending
      if (i < 0) return { ...t, pending }
      return { ...replace(i, { ...(items[i] as Extract<Item, { kind: "approval" }>), decision: e.decision }), pending }
    }
    case "plan":
      return { ...t, plan: e.items }
    case "turnFinished": {
      // Tools still "running" when the turn ends were cut off.
      const settled = items.map((it) => (it.kind === "tool" && it.status === "running" ? { ...it, status: "error" as const, output: it.output || "Stopped." } : it))
      return {
        ...t,
        running: false,
        pending: null,
        items: [...settled, { kind: "turnEnd", id: e.turnId, usage: e.usage, costUsd: e.costUsd, durationMs: e.durationMs, interrupted: e.interrupted }],
      }
    }
    case "context":
      return { ...t, context: { used: e.usedTokens, window: e.windowTokens } }
    case "notice":
      return push({ kind: "notice", id: uid("n"), level: e.level, text: e.text })
    case "error":
      return push({ kind: "notice", id: uid("e"), level: "error", text: e.message }, { running: false, pending: null })
    case "exited":
      return e.code && e.code !== 0
        ? push({ kind: "notice", id: uid("x"), level: "error", text: e.message ?? `The agent exited (code ${e.code}).` }, { running: false, pending: null })
        : { ...t, running: false, pending: null }
  }
}

export function replay(events: ChatEvent[]): Transcript {
  // A reopened chat isn't running, whatever its log ended with.
  return { ...events.reduce(reduce, emptyTranscript()), running: false, pending: null }
}

/** A turn's items between its user message and its end, grouped for display: each run of
 *  reasoning and tool calls becomes one "work" block; text, approvals and notices stand alone. */
export type Block = { kind: "work"; id: string; items: Item[] } | { kind: "item"; item: Item }

export function blocks(items: Item[]): Block[] {
  const out: Block[] = []
  for (const it of items) {
    const work = it.kind === "reasoning" || it.kind === "tool"
    const last = out[out.length - 1]
    if (work && last?.kind === "work") last.items.push(it)
    else if (work) out.push({ kind: "work", id: `w-${it.id}`, items: [it] })
    else out.push({ kind: "item", item: it })
  }
  return out
}

export function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(n >= 10_000 ? 0 : 1)}k`
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
