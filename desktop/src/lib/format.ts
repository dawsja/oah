// Formatting shared with the CLI (src/ui/theme.ts, src/session.ts), plus the
// transcript grouping and command matching the TUI uses.
import type { Entry } from "./protocol"

export function fmtTokens(n: number): string {
  if (n >= 999_500) {
    const m = n / 1e6
    return `${m >= 10 ? Math.round(m) : +m.toFixed(1)}M`
  }
  if (n >= 10_000) return `${Math.round(n / 1000)}k`
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`
  return String(n)
}

export function fmtMs(ms: number): string {
  const s = Math.round(ms / 1000)
  if (s >= 3600) return `${Math.floor(s / 3600)}h${String(Math.floor(s / 60) % 60).padStart(2, "0")}m`
  return s >= 60 ? `${Math.floor(s / 60)}m${String(s % 60).padStart(2, "0")}s` : `${s}s`
}

export function fmtCost(usd: number): string {
  if (usd > 0 && usd < 0.005) return "<$0.01"
  return `$${usd < 100 ? usd.toFixed(2) : Math.round(usd).toLocaleString("en-US")}`
}

export function relativeTime(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ms) / 1000))
  if (s < 45) return "just now"
  const m = Math.round(s / 60)
  if (m < 60) return `${m}m ago`
  const h = Math.round(m / 60)
  if (h < 24) return `${h}h ago`
  const d = Math.round(h / 24)
  if (d < 7) return d === 1 ? "yesterday" : `${d}d ago`
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" })
}

export const basename = (p: string) => p.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || p

/** `anthropic/claude-opus-5-5` → `claude-opus-5-5`. */
export const modelSlug = (full: string) => (full.includes("/") ? full.slice(full.indexOf("/") + 1) : full)

export const verbOf = (title: string) => title.split(" ", 1)[0]!

type ToolEntry = Extract<Entry, { kind: "tool" }>
export type Step = { e: Entry; i: number }

export interface Turn {
  kind: "turn"
  user?: Extract<Entry, { kind: "user" }>
  userIndex?: number
  steps: Step[]
}
export type Group = Turn | { kind: "note"; entry: Entry; i: number }

/** Same grouping as the TUI: your message + oah's work, and standalone notes. */
export function groupTurns(entries: Entry[]): Group[] {
  const out: Group[] = []
  let cur: Turn | null = null
  entries.forEach((e, i) => {
    if (!e) return
    if (e.kind === "user") {
      cur = { kind: "turn", user: e, userIndex: i, steps: [] }
      out.push(cur)
    } else if (e.kind === "info" || e.kind === "error") {
      if (cur && e.kind === "error") cur.steps.push({ e, i })
      else {
        out.push({ kind: "note", entry: e, i })
        cur = null
      }
    } else {
      if (!cur) {
        cur = { kind: "turn", steps: [] }
        out.push(cur)
      }
      cur.steps.push({ e, i })
    }
  })
  return out
}

export type Block = { kind: "steps"; items: Step[]; key: number } | { kind: "entry"; e: Entry; i: number }

/** Fold consecutive thinking/tool entries into one steps block each. */
export function blocksOf(turn: Turn): Block[] {
  const blocks: Block[] = []
  for (const s of turn.steps) {
    if (s.e.kind === "tool" || s.e.kind === "reasoning") {
      const last = blocks.at(-1)
      if (last?.kind === "steps") last.items.push(s)
      else blocks.push({ kind: "steps", items: [s], key: s.i })
    } else blocks.push({ kind: "entry", e: s.e, i: s.i })
  }
  return blocks
}

export function stepsSummary(items: Entry[]) {
  const tools = items.filter((e): e is ToolEntry => e.kind === "tool")
  const verbs = [...new Set(tools.map((x) => verbOf(x.title)))]
  const first = items[0]
  const last = items.at(-1)
  const start = first?.kind === "tool" || first?.kind === "reasoning" ? first.started : first?.at
  const end = last?.kind === "tool" && last.ms !== undefined ? last.started + last.ms : last?.kind === "reasoning" && last.ms !== undefined ? last.started + last.ms : undefined
  const ms = start && end ? end - start : 0
  const failures = tools.filter((x) => x.status === "failed" || x.status === "denied")
  const current = tools.findLast((x) => x.status === "running")
  const edits = tools.filter((x) => /^(Edit|Write|Patch)$/.test(verbOf(x.title)) && x.detail.length)
  return { tools, verbs, ms, failures, current, edits }
}

export interface Command {
  name: string
  args: string
  desc: string
}

/** src/commands.ts matchCommands: prefix match, skills, then a subsequence fallback. */
export function matchCommands(prefix: string, commands: Command[], skills: { name: string; description: string }[]): Command[] {
  const p = prefix.toLowerCase()
  const out = commands.filter((c) => c.name.startsWith(p))
  for (const s of skills) if (s.name.toLowerCase().startsWith(p) && !out.some((c) => c.name === s.name)) out.push({ name: s.name, args: "", desc: `skill · ${s.description}` })
  if (!out.length && p) {
    for (const c of commands) {
      let i = 0
      for (const ch of c.name) if (ch === p[i]) i++
      if (i === p.length) out.push(c)
    }
  }
  return out
}

/** Picker search: every word must appear in the label, detail, group or keywords. */
export function filterItems<T extends { label: string; detail: string; group?: string; keywords?: string; searchOnly?: boolean }>(items: T[], filter: string): T[] {
  const words = filter.toLowerCase().split(/\s+/).filter(Boolean)
  if (!words.length) return items.filter((i) => !i.searchOnly)
  return items.filter((i) => {
    const hay = `${i.label} ${i.detail} ${i.group ?? ""} ${i.keywords ?? ""}`.toLowerCase()
    return words.every((w) => hay.includes(w))
  })
}
