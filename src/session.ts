// Saved sessions (~/.config/oah/sessions/<id>.json), the transcript model and
// prompt history.
import { appendFileSync, readdirSync, readFileSync, statSync } from "node:fs"
import { join } from "node:path"
import type { ModelMessage } from "ai"
import type { Checkpoint } from "./checkpoints"
import { ensureDir, paths, readJson, writeFileAtomic, type Effort, type Mode } from "./config"

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
  /** Time spent working on the goal (turns and compactions, not pauses), in ms. */
  ms?: number
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
  /** Workspace snapshots for /undo, oldest first. */
  checkpoints?: Checkpoint[]
  /** Total cost in USD for metered providers. */
  cost?: number
}

export const newId = () => crypto.randomUUID().replace(/-/g, "").slice(0, 12)

export function newSession(model: string, effort: Effort, mode: Mode): SessionData {
  const now = Date.now()
  return { id: newId(), created: now, updated: now, cwd: process.cwd(), title: "", model, effort, mode, messages: [], transcript: [], contextTokens: 0, totalTokens: 0, todos: [], compactions: 0 }
}

const file = (id: string) => join(paths.sessions, `${id}.json`)
const metaFile = (id: string) => join(paths.sessions, "meta", `${id}.json`)

/** Sidecar summary so listing doesn't parse every full session. `mtime`/`size` are the session file's when written. */
interface SessionMeta extends SessionSummary {
  mtime: number
  size: number
}

function writeMeta(s: Pick<SessionData, "id" | "title" | "cwd" | "updated">) {
  try {
    const st = statSync(file(s.id))
    const meta: SessionMeta = { id: s.id, title: s.title, cwd: s.cwd, updated: s.updated, mtime: st.mtimeMs, size: st.size }
    writeFileAtomic(metaFile(s.id), JSON.stringify(meta), true)
  } catch {}
}

export function saveSession(s: SessionData) {
  if (!s.messages.length) return
  s.updated = Date.now()
  writeFileAtomic(file(s.id), JSON.stringify(s), true)
  writeMeta(s)
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

/**
 * Summaries of every saved session, newest first. Reads each session's small meta
 * file; a missing or stale one (older sessions, files written by /import) falls back
 * to parsing the full session and is rewritten.
 */
export function listSessions(): SessionSummary[] {
  const out: SessionSummary[] = []
  let files: string[]
  try {
    files = readdirSync(paths.sessions)
  } catch {
    return out
  }
  for (const f of files) {
    if (!f.endsWith(".json")) continue
    const path = join(paths.sessions, f)
    try {
      const st = statSync(path)
      const meta = readJson<SessionMeta | null>(metaFile(f.slice(0, -5)), null)
      if (meta && meta.mtime === st.mtimeMs && meta.size === st.size) {
        out.push({ id: meta.id, title: meta.title, cwd: meta.cwd, updated: meta.updated })
        continue
      }
      const s = JSON.parse(readFileSync(path, "utf8")) as SessionData
      out.push({ id: s.id, title: s.title, cwd: s.cwd, updated: s.updated })
      writeMeta(s)
    } catch {}
  }
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
