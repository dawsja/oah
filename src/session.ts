// Saved sessions (~/.config/nur/sessions/<id>.json), the transcript model and
// prompt history.
import { appendFileSync, readdirSync, readFileSync } from "node:fs"
import { join } from "node:path"
import type { ModelMessage } from "ai"
import { ensureDir, paths, writeFileAtomic, type Effort, type Mode } from "./config"

export type ToolStatus = "running" | "ok" | "failed" | "denied"

export type Entry = EntryBody & {
  /** When the entry was added (ms). Older sessions may not have it. */
  at?: number
}

type EntryBody =
  | { kind: "user"; text: string }
  | { kind: "assistant"; text: string; model: string }
  | { kind: "reasoning"; text: string; started: number; ms?: number }
  | { kind: "tool"; callId: string; name: string; title: string; summary: string; detail: string[]; status: ToolStatus; started: number; ms?: number }
  | { kind: "info"; text: string }
  | { kind: "error"; text: string }

export interface Usage {
  input: number
  output: number
  total: number
}

export interface Goal {
  text: string
  turns: number
  done: boolean
}

export interface SessionData {
  id: string
  created: number
  updated: number
  cwd: string
  title: string
  model: string
  effort: Effort
  mode: Mode
  /** AI SDK conversation (user, assistant, tool messages). */
  messages: ModelMessage[]
  transcript: Entry[]
  /** Tokens the next request is expected to carry (context meter). */
  contextTokens: number
  totalTokens: number
  goal?: Goal
  todos: { text: string; status: string }[]
  compactions: number
}

export const newId = () => crypto.randomUUID().replace(/-/g, "").slice(0, 12)

export function newSession(model: string, effort: Effort, mode: Mode): SessionData {
  const now = Date.now()
  return { id: newId(), created: now, updated: now, cwd: process.cwd(), title: "", model, effort, mode, messages: [], transcript: [], contextTokens: 0, totalTokens: 0, todos: [], compactions: 0 }
}

const file = (id: string) => join(paths.sessions, `${id}.json`)

export function saveSession(s: SessionData) {
  if (!s.messages.length) return
  s.updated = Date.now()
  writeFileAtomic(file(s.id), JSON.stringify(s), true)
}

export function loadSession(idOrPrefix: string): SessionData | undefined {
  try {
    return JSON.parse(readFileSync(file(idOrPrefix), "utf8"))
  } catch {}
  try {
    const matches = readdirSync(paths.sessions).filter((f) => f.startsWith(idOrPrefix) && f.endsWith(".json"))
    if (matches.length === 1) return JSON.parse(readFileSync(join(paths.sessions, matches[0]!), "utf8"))
  } catch {}
  return undefined
}

export interface SessionSummary {
  id: string
  title: string
  cwd: string
  updated: number
}

export function listSessions(): SessionSummary[] {
  const out: SessionSummary[] = []
  try {
    for (const f of readdirSync(paths.sessions)) {
      if (!f.endsWith(".json")) continue
      try {
        const s = JSON.parse(readFileSync(join(paths.sessions, f), "utf8")) as SessionData
        out.push({ id: s.id, title: s.title, cwd: s.cwd, updated: s.updated })
      } catch {}
    }
  } catch {}
  return out.sort((a, b) => b.updated - a.updated)
}

export function latestSession(cwd: string): SessionData | undefined {
  const s = listSessions().find((x) => x.cwd === cwd)
  return s ? loadSession(s.id) : undefined
}

export function relativeTime(ms: number): string {
  const d = Math.floor((Date.now() - ms) / 1000)
  if (d < 60) return "just now"
  if (d < 3600) return `${Math.floor(d / 60)}m ago`
  if (d < 86400) return `${Math.floor(d / 3600)}h ago`
  return `${Math.floor(d / 86400)}d ago`
}

export function loadHistory(): string[] {
  try {
    return readFileSync(paths.history, "utf8")
      .split("\n")
      .filter(Boolean)
      .map((l) => JSON.parse(l) as string)
  } catch {
    return []
  }
}

export function appendHistory(entry: string) {
  ensureDir(paths.sessions)
  appendFileSync(paths.history, JSON.stringify(entry) + "\n")
}
