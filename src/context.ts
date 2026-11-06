import type { ModelMessage } from "ai"
import { capText } from "./tools/text"

export interface ElisionPolicy { keep?: number; threshold?: number }

/** Anthropic signed blocks may bind their preceding prefix. Never silently rebase them. */
export function hasBoundHistory(messages: ModelMessage[]): boolean {
  return messages.some((m) => Array.isArray(m.content) && m.content.some((p) => {
    const metadata = "providerOptions" in p ? p.providerOptions?.anthropic : undefined
    return metadata?.signature != null || metadata?.redactedData != null
  }))
}

/** One-way content-only pruning. Never remove messages, tool parts, IDs or provider metadata. */
export function elideToolResults(messages: ModelMessage[], policy: ElisionPolicy = {}): ModelMessage[] {
  if (hasBoundHistory(messages)) return messages
  const keep = Math.max(0, policy.keep ?? 4)
  const calls = new Map<string, { name: string; path?: string }>()
  const results: { id: string; value: string; name: string }[] = []
  for (const m of messages) {
    if (!Array.isArray(m.content)) continue
    for (const p of m.content) {
      if (p.type === "tool-call") calls.set(p.toolCallId, { name: p.toolName, path: (p.input as any)?.path })
      if (p.type === "tool-result" && p.output.type === "text") results.push({ id: p.toolCallId, value: p.output.value, name: p.toolName })
    }
  }
  if (results.reduce((n, r) => n + r.value.length, 0) < (policy.threshold ?? 60_000)) return messages
  const targets = new Map(results.slice(0, Math.max(0, results.length - keep)).filter((r) => r.value.length > 1200 && !r.value.startsWith("[oah: elided") && !/^Error:|exit code: [1-9]/m.test(r.value)).map((r) => [r.id, r]))
  if (!targets.size) return messages
  return messages.map((m) => {
    if (m.role !== "tool") return m
    return { ...m, content: m.content.map((p) => {
      const r = p.type === "tool-result" ? targets.get(p.toolCallId) : undefined
      if (!r || p.type !== "tool-result" || p.output.type !== "text") return p
      const c = calls.get(r.id)
      const lines = r.value.split("\n")
      const stub = `[oah: elided ${r.name}${c?.path ? ` ${c.path}` : ""} — ${lines.length} lines, ${r.value.length} chars; re-run the tool with a narrower range if needed]`
      const text = r.name === "read_file" ? `${stub}\n${capText(lines.slice(0, 15).join("\n"), 1800)}\n…\n${capText(lines.slice(-15).join("\n"), 1800)}` : stub
      return { ...p, output: { ...p.output, value: text } }
    }) }
  })
}

/** Keep the last two complete user exchanges, never split assistant/tool sequences. */
export function splitRecent(messages: ModelMessage[], count = 2): { prefix: ModelMessage[]; tail: ModelMessage[] } {
  const starts = messages.flatMap((m, i) => m.role === "user" && !(typeof m.content === "string" && /^\[oah (?:context update|verification:)/.test(m.content)) ? [i] : [])
  const at = starts.length > count ? starts[starts.length - count]! : 0
  return { prefix: messages.slice(0, at), tail: messages.slice(at) }
}

export function elideImages(messages: ModelMessage[]): ModelMessage[] {
  if (hasBoundHistory(messages)) return messages
  return messages.map((m) => m.role === "user" && Array.isArray(m.content) ? {
    ...m, content: m.content.map((p) => p.type === "image" ? { type: "text" as const, text: "[image elided after its turn; ask the user to reattach if needed]" } : p),
  } : m)
}

export function estimateContext(messages: ModelMessage[]): number {
  // Conservative estimate with schema/system headroom; provider usage remains authoritative.
  return Math.ceil(JSON.stringify(messages).length / 3) + 6000
}
