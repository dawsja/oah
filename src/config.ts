// Paths (~/.config/nur), settings and shared enums.
import { mkdirSync, readFileSync, renameSync, writeFileSync, chmodSync } from "node:fs"
import { homedir } from "node:os"
import { dirname, join } from "node:path"

export const VERSION = "0.1.0"

/** `$NUR_HOME`, else `$XDG_CONFIG_HOME/nur`, else `~/.config/nur`. */
export function homeDir(): string {
  if (process.env.NUR_HOME) return process.env.NUR_HOME
  const xdg = process.env.XDG_CONFIG_HOME
  return join(xdg && xdg.length > 0 ? xdg : join(homedir(), ".config"), "nur")
}

export const paths = {
  get settings() { return join(homeDir(), "settings.json") },
  get auth() { return join(homeDir(), "auth.json") },
  get hostId() { return join(homeDir(), "host_id") },
  get history() { return join(homeDir(), "history.jsonl") },
  get sessions() { return join(homeDir(), "sessions") },
  get memory() { return join(homeDir(), "memory") },
  get skills() { return join(homeDir(), "skills") },
  get mcp() { return join(homeDir(), "mcp.json") },
  get agents() { return join(homeDir(), "AGENTS.md") },
  get modelsCache() { return join(homeDir(), "models.json") },
}

export function ensureDir(p: string) {
  mkdirSync(p, { recursive: true })
}

/** Atomic write; `private` sets 0600. */
export function writeFileAtomic(path: string, data: string, priv = false) {
  ensureDir(dirname(path))
  const tmp = `${path}.tmp${process.pid}`
  writeFileSync(tmp, data, priv ? { mode: 0o600 } : undefined)
  if (priv) chmodSync(tmp, 0o600)
  renameSync(tmp, path)
}

export function readJson<T>(path: string, fallback: T): T {
  try {
    return JSON.parse(readFileSync(path, "utf8")) as T
  } catch {
    return fallback
  }
}

export function shortPath(p: string): string {
  const h = homedir()
  return p.startsWith(h) ? "~" + p.slice(h.length) : p
}

// ---------------------------------------------------------------------------

export type Provider = "chatgpt" | "openai" | "anthropic"
export const PROVIDERS: Provider[] = ["chatgpt", "anthropic", "openai"]

export const providerInfo: Record<Provider, { label: string; catalog: string; billing: string }> = {
  chatgpt: { label: "ChatGPT", catalog: "openai", billing: "your ChatGPT plan" },
  openai: { label: "OpenAI", catalog: "openai", billing: "OpenAI API key" },
  anthropic: { label: "Anthropic", catalog: "anthropic", billing: "Anthropic API key" },
}

export const defaultModel: Record<Provider, string> = {
  chatgpt: "gpt-5.6",
  openai: "gpt-5.6",
  anthropic: "claude-opus-5-5",
}

/** `provider/slug`; a bare slug is inferred (`claude*` → anthropic, else chatgpt). */
export function splitModel(full: string): { provider: Provider; slug: string } {
  const i = full.indexOf("/")
  if (i > 0) {
    const p = full.slice(0, i) as Provider
    if (PROVIDERS.includes(p)) return { provider: p, slug: full.slice(i + 1) }
  }
  return { provider: full.startsWith("claude") ? "anthropic" : "chatgpt", slug: full }
}

export const joinModel = (p: Provider, slug: string) => `${p}/${slug}`

export type Effort = "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
export const EFFORTS: Effort[] = ["minimal", "low", "medium", "high", "xhigh", "max"]
export const effortInfo: Record<Effort, string> = {
  minimal: "fastest, almost no reasoning",
  low: "quick answers, light reasoning",
  medium: "balanced speed and depth",
  high: "deeper reasoning for hard problems",
  xhigh: "very deep reasoning for long agentic work",
  max: "everything it has",
}

export type Permission = "auto" | "ask" | "allow"
export const PERMISSIONS: Permission[] = ["auto", "ask", "allow"]
export const permissionInfo: Record<Permission, { label: string; desc: string }> = {
  auto: { label: "auto", desc: "reads and safe commands run; edits, writes and other commands ask" },
  ask: { label: "always ask", desc: "ask before every tool call" },
  allow: { label: "always allow", desc: "never ask (use with care)" },
}
export const nextPermission = (p: Permission): Permission => PERMISSIONS[(PERMISSIONS.indexOf(p) + 1) % PERMISSIONS.length]!

export type Mode = "plan" | "build"
export type Design = "a" | "b" | "c"
export const DESIGNS: Design[] = ["a", "b", "c"]
export const designInfo: Record<Design, { name: string; desc: string }> = {
  a: { name: "A · Panels", desc: "tinted panel for you, labelled column for nur" },
  b: { name: "B · Bubbles", desc: "chat bubbles: you on the right, nur on the left" },
  c: { name: "C · Timeline", desc: "one vertical rail with a node per event" },
}

export interface Settings {
  /** `provider/slug`; empty = choose from available credentials. */
  model: string
  effort: Effort
  permission: Permission
  mode: Mode
  design: Design
  autoCompact: boolean
  /** Fraction of the context window that triggers auto-compaction. */
  autoCompactAt: number
  /** Extra command prefixes treated as safe in auto mode. */
  allowedCommands: string[]
  /** Overrides (mainly for testing / proxies). */
  openaiBaseURL?: string
  anthropicBaseURL?: string
  contextWindow?: number
}

export const defaultSettings: Settings = {
  model: "",
  effort: "medium",
  permission: "auto",
  mode: "build",
  design: "a",
  autoCompact: true,
  autoCompactAt: 0.85,
  allowedCommands: [],
}

export function loadSettings(): Settings {
  return { ...defaultSettings, ...readJson<Partial<Settings>>(paths.settings, {}) }
}

export function saveSettings(s: Settings) {
  writeFileAtomic(paths.settings, JSON.stringify(s, null, 2))
}
