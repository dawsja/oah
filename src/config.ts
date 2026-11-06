// Paths (~/.config/oah), settings and shared enums.
import { mkdirSync, readFileSync, renameSync, writeFileSync, chmodSync } from "node:fs"
import { homedir } from "node:os"
import { dirname, join } from "node:path"
import pkg from "../package.json" with { type: "json" }

/** From package.json (bundled into the binary), so a release only bumps one place. */
export const VERSION: string = pkg.version

/** `$OAH_HOME`, else `$XDG_CONFIG_HOME/oah`, else `~/.config/oah`. */
export function homeDir(): string {
  if (process.env.OAH_HOME) return process.env.OAH_HOME
  const xdg = process.env.XDG_CONFIG_HOME
  return join(xdg && xdg.length > 0 ? xdg : join(homedir(), ".config"), "oah")
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
  get mcpAuth() { return join(homeDir(), "mcp-auth.json") },
  get agents() { return join(homeDir(), "AGENTS.md") },
  get modelsCache() { return join(homeDir(), "models.json") },
  get snapshots() { return join(homeDir(), "snapshots") },
  get images() { return join(homeDir(), "images") },
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

/** Which AI SDK provider package talks to a provider. */
export type Sdk = "openai" | "anthropic" | "google" | "xai" | "deepseek" | "openrouter" | "compat"

export interface ProviderDef {
  label: string
  /** Provider id in the models.dev catalog. */
  catalog: string
  /** What usage is billed to, for messages. */
  billing: string
  sdk: Sdk
  /** Environment variables checked for an API key, in order. */
  env: string[]
  baseURL?: string
  /** Preferred default model; the newest catalog model is used if it's gone. */
  defaultModel: string
  /** Shown in /auth next to the key option. */
  blurb: string
  /** A server on this machine: no key, models listed by the server. */
  local?: boolean
}

export const providerInfo = {
  chatgpt: { label: "ChatGPT", catalog: "openai", billing: "your ChatGPT plan", sdk: "openai", env: [], defaultModel: "gpt-5.6", blurb: "use your ChatGPT plan" },
  anthropic: { label: "Anthropic", catalog: "anthropic", billing: "an Anthropic API key", sdk: "anthropic", env: ["ANTHROPIC_API_KEY"], defaultModel: "claude-opus-5-5", blurb: "Claude models" },
  openai: { label: "OpenAI", catalog: "openai", billing: "an OpenAI API key", sdk: "openai", env: ["OPENAI_API_KEY"], defaultModel: "gpt-5.6", blurb: "GPT models" },
  google: { label: "Gemini", catalog: "google", billing: "a Gemini API key", sdk: "google", env: ["GEMINI_API_KEY", "GOOGLE_GENERATIVE_AI_API_KEY", "GOOGLE_API_KEY"], defaultModel: "gemini-4-argon", blurb: "Google Gemini models" },
  xai: { label: "xAI", catalog: "xai", billing: "an xAI API key", sdk: "xai", env: ["XAI_API_KEY"], defaultModel: "grok-4.7", blurb: "Grok models" },
  deepseek: { label: "DeepSeek", catalog: "deepseek", billing: "a DeepSeek API key", sdk: "deepseek", env: ["DEEPSEEK_API_KEY"], defaultModel: "deepseek-v4-pro", blurb: "DeepSeek models" },
  openrouter: { label: "OpenRouter", catalog: "openrouter", billing: "an OpenRouter API key", sdk: "openrouter", env: ["OPENROUTER_API_KEY"], defaultModel: "anthropic/claude-opus-5.5", blurb: "hundreds of models with one key" },
  synthetic: { label: "Synthetic", catalog: "synthetic", billing: "a Synthetic API key", sdk: "compat", env: ["SYNTHETIC_API_KEY"], baseURL: "https://api.synthetic.new/openai/v1", defaultModel: "hf:moonshotai/Kimi-K3", blurb: "open models (GLM, Kimi, DeepSeek…)" },
  kimi: { label: "Kimi", catalog: "moonshotai", billing: "a Moonshot API key", sdk: "compat", env: ["MOONSHOT_API_KEY"], baseURL: "https://api.moonshot.ai/v1", defaultModel: "kimi-k3.1-pro", blurb: "Kimi models via the Moonshot API" },
  "kimi-code": { label: "Kimi Code", catalog: "kimi-code-plan-global", billing: "your Kimi Code plan", sdk: "compat", env: ["KIMI_API_KEY"], baseURL: "https://api.kimi.ai/coding/v1", defaultModel: "kimi-for-coding", blurb: "Kimi Code plan" },
  zai: { label: "Z.ai", catalog: "zai", billing: "a Z.ai API key", sdk: "compat", env: ["ZHIPU_API_KEY", "ZAI_API_KEY"], baseURL: "https://api.z.ai/api/paas/v4", defaultModel: "glm-5.3", blurb: "GLM models via the Z.ai API" },
  "zai-coding": { label: "Z.ai Coding", catalog: "zai-coding-plan", billing: "your GLM Coding plan", sdk: "compat", env: ["ZHIPU_API_KEY", "ZAI_API_KEY"], baseURL: "https://api.z.ai/api/coding/paas/v4", defaultModel: "glm-5.3", blurb: "GLM Coding plan" },
  minimax: { label: "MiniMax", catalog: "minimax", billing: "a MiniMax API key", sdk: "anthropic", env: ["MINIMAX_API_KEY"], baseURL: "https://api.minimax.io/anthropic/v1", defaultModel: "MiniMax-M3", blurb: "MiniMax models via the MiniMax API" },
  "minimax-coding": { label: "MiniMax Coding", catalog: "minimax-coding-plan", billing: "your MiniMax Coding plan", sdk: "anthropic", env: ["MINIMAX_API_KEY"], baseURL: "https://api.minimax.io/anthropic/v1", defaultModel: "MiniMax-M3", blurb: "MiniMax Coding plan" },
  ollama: { label: "Ollama", catalog: "local-ollama", billing: "Ollama running on this machine", sdk: "compat", env: [], baseURL: "http://localhost:11434/v1", defaultModel: "", blurb: "models running in Ollama on this machine", local: true },
  lmstudio: { label: "LM Studio", catalog: "local-lmstudio", billing: "LM Studio running on this machine", sdk: "compat", env: [], baseURL: "http://localhost:1234/v1", defaultModel: "", blurb: "models loaded in LM Studio on this machine", local: true },
} satisfies Record<string, ProviderDef>

export type Provider = keyof typeof providerInfo
export const PROVIDERS = Object.keys(providerInfo) as Provider[]
/** Providers that use an API key (not the ChatGPT sign-in or local servers). */
export const KEY_PROVIDERS = PROVIDERS.filter((p) => p !== "chatgpt" && !(providerInfo[p] as ProviderDef).local) as Exclude<Provider, "chatgpt">[]
export const info = (p: Provider): ProviderDef => providerInfo[p]

/** `provider/slug`; a bare slug is inferred (`claude*` → anthropic, else chatgpt). */
export function splitModel(full: string): { provider: Provider; slug: string } {
  const i = full.indexOf("/")
  if (i > 0) {
    const p = full.slice(0, i) as Provider
    if (PROVIDERS.includes(p)) return { provider: p, slug: full.slice(i + 1) }
  }
  return { provider: inferProvider(full), slug: full }
}

/** Best guess at a provider for a bare model slug. */
export function inferProvider(slug: string): Provider {
  if (slug.startsWith("claude")) return "anthropic"
  if (slug.startsWith("gemini")) return "google"
  if (slug.startsWith("grok")) return "xai"
  if (slug.startsWith("deepseek")) return "deepseek"
  if (slug.startsWith("kimi")) return "kimi"
  if (slug.startsWith("glm")) return "zai"
  if (/^minimax/i.test(slug)) return "minimax"
  return "chatgpt"
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
  auto: { label: "auto", desc: "works on its own; a safety monitor reviews risky actions and blocks or asks" },
  ask: { label: "always ask", desc: "ask before every tool call" },
  allow: { label: "always allow", desc: "never ask (use with care)" },
}
export const nextPermission = (p: Permission): Permission => PERMISSIONS[(PERMISSIONS.indexOf(p) + 1) % PERMISSIONS.length]!

export type Mode = "plan" | "build"
export interface Settings {
  /** `provider/slug`; empty = choose from available credentials. */
  model: string
  effort: Effort
  permission: Permission
  mode: Mode
  autoCompact: boolean
  /** Fraction of the context window that triggers auto-compaction. */
  autoCompactAt: number
  /** Extra command prefixes treated as safe in auto mode. */
  allowedCommands: string[]
  /** Recently used models (`provider/slug`), newest first — pinned at the top of /model. */
  recentModels: string[]
  /** Checkpoints kept per session for /undo; older ones are dropped. */
  maxCheckpoints: number
  /** Auto mode's safety monitor model (`provider/slug`); empty = a small model of the current provider, else the current model. */
  safetyModel?: string
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
  autoCompact: true,
  autoCompactAt: 0.85,
  allowedCommands: [],
  recentModels: [],
  maxCheckpoints: 20,
}

export function loadSettings(): Settings {
  const { design: _removed, ...saved } = readJson<Partial<Settings> & { design?: unknown }>(paths.settings, {})
  return { ...defaultSettings, ...saved }
}

export function saveSettings(s: Settings) {
  writeFileAtomic(paths.settings, JSON.stringify(s, null, 2))
}
