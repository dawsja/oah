// Model catalog from https://models.dev — context windows, reasoning effort
// levels, costs and capabilities. Cached in ~/.config/oah/models.json for a day;
// a small built-in table keeps oah working offline.
import { statSync } from "node:fs"
import { info, paths, PROVIDERS, readJson, splitModel, writeFileAtomic, type Effort, type Provider } from "./config"

const URL = "https://models.dev/api.json"
const TTL_MS = 24 * 60 * 60 * 1000

export interface CatalogModel {
  id: string
  name: string
  family?: string
  reasoning?: boolean
  tool_call?: boolean
  release_date?: string
  status?: string
  type?: string
  reasoning_options?: { type: string; values?: string[] }[]
  limit?: { context?: number; input?: number; output?: number }
  modalities?: { input?: string[]; output?: string[] }
  attachment?: boolean
  cost?: { input?: number; output?: number; cache_read?: number; cache_write?: number }
}

type Catalog = Record<string, { id: string; name: string; models: Record<string, CatalogModel> }>

const FALLBACK: Catalog = {
  openai: {
    id: "openai",
    name: "OpenAI",
    models: {
      "gpt-5.6": { id: "gpt-5.6", name: "GPT-5.6", reasoning: true, tool_call: true, limit: { context: 1050000, input: 922000 }, reasoning_options: [{ type: "effort", values: ["none", "low", "medium", "high", "xhigh", "max"] }] },
      "gpt-5.5": { id: "gpt-5.5", name: "GPT-5.5", reasoning: true, tool_call: true, limit: { context: 1050000, input: 922000 }, reasoning_options: [{ type: "effort", values: ["minimal", "low", "medium", "high", "xhigh"] }] },
      "gpt-5.4-mini": { id: "gpt-5.4-mini", name: "GPT-5.4 mini", reasoning: true, tool_call: true, limit: { context: 400000, input: 272000 } },
    },
  },
  anthropic: {
    id: "anthropic",
    name: "Anthropic",
    models: {
      "claude-opus-5-5": { id: "claude-opus-5-5", name: "Claude Opus 5.5", reasoning: true, tool_call: true, limit: { context: 1000000 }, reasoning_options: [{ type: "effort", values: ["low", "medium", "high", "xhigh", "max"] }] },
      "claude-sonnet-5-5": { id: "claude-sonnet-5-5", name: "Claude Sonnet 5.5", reasoning: true, tool_call: true, limit: { context: 1000000 }, reasoning_options: [{ type: "effort", values: ["low", "medium", "high", "xhigh", "max"] }] },
      "claude-haiku-5-5": { id: "claude-haiku-5-5", name: "Claude Haiku 5.5", reasoning: true, tool_call: true, limit: { context: 1000000 }, reasoning_options: [{ type: "effort", values: ["low", "medium", "high", "xhigh", "max"] }] },
    },
  },
}

let catalog: Catalog = { ...FALLBACK, ...readJson<Catalog>(paths.modelsCache, {}) }
/** Models reported by local servers (Ollama, LM Studio); never cached to disk. */
const localCatalog: Catalog = {}
const providerModels = (id: string) => localCatalog[id]?.models ?? catalog[id]?.models

export function registerLocal(id: string, name: string, models: CatalogModel[]) {
  localCatalog[id] = { id, name, models: Object.fromEntries(models.map((m) => [m.id, m])) }
}

/** Re-read the cached catalog (after the config dir changes; used by tests). */
export function reloadCatalog() {
  catalog = { ...FALLBACK, ...readJson<Catalog>(paths.modelsCache, {}) }
}

function cacheAge(): number {
  try {
    return Date.now() - statSync(paths.modelsCache).mtimeMs
  } catch {
    return Infinity
  }
}

/** Refresh from models.dev when the cache is stale. Never throws. */
export async function refreshCatalog(force = false): Promise<boolean> {
  const missing = PROVIDERS.some((p) => !info(p).local && !catalog[info(p).catalog])
  if (!force && !missing && cacheAge() < TTL_MS) return false
  try {
    const r = await fetch(process.env.OAH_MODELS_URL ?? URL, { signal: AbortSignal.timeout(10_000) })
    if (!r.ok) return false
    const all = (await r.json()) as Catalog
    const slim: Catalog = {}
    for (const id of new Set(PROVIDERS.filter((p) => !info(p).local).map((p) => info(p).catalog))) if (all[id]) slim[id] = all[id]!
    catalog = slim
    writeFileAtomic(paths.modelsCache, JSON.stringify(slim))
    return true
  } catch {
    return false
  }
}

export function lookup(full: string): CatalogModel | undefined {
  const { provider, slug } = splitModel(full)
  return providerModels(info(provider).catalog)?.[slug]
}

export function contextWindow(full: string, override?: number): number {
  if (override) return override
  const m = lookup(full)
  const p = splitModel(full).provider
  return m?.limit?.input ?? m?.limit?.context ?? (p === "anthropic" ? 1_000_000 : info(p).sdk === "openai" ? 400_000 : info(p).local ? 8_192 : 128_000)
}

/** Effort levels the model accepts, mapped onto oah's scale. */
export function effortsFor(full: string): Effort[] {
  const m = lookup(full)
  const values = m?.reasoning_options?.find((o) => o.type === "effort")?.values
  const map = (v: string): Effort | null => (v === "none" ? null : (["minimal", "low", "medium", "high", "xhigh", "max"].includes(v) ? (v as Effort) : null))
  const out = (values ?? []).map(map).filter((e): e is Effort => e !== null)
  if (out.length) return out
  if (m && m.reasoning === false) return []
  const p = splitModel(full).provider
  if (p === "anthropic") return ["low", "medium", "high", "xhigh", "max"]
  if (info(p).sdk === "openai") return ["minimal", "low", "medium", "high", "xhigh"]
  return []
}

/** Clamp an effort to what the model accepts (nearest level). */
export function clampEffort(full: string, e: Effort): Effort | undefined {
  const allowed = effortsFor(full)
  if (!allowed.length) return undefined
  if (allowed.includes(e)) return e
  const order: Effort[] = ["minimal", "low", "medium", "high", "xhigh", "max"]
  const i = order.indexOf(e)
  return [...allowed].sort((a, b) => Math.abs(order.indexOf(a) - i) - Math.abs(order.indexOf(b) - i))[0]
}

export interface ModelChoice {
  full: string
  provider: Provider
  slug: string
  name: string
  context: number
  release: string
}

function usable(p: Provider, m: CatalogModel): boolean {
  if (!m.tool_call || (m.type && m.type !== "chat") || m.status === "deprecated") return false
  if (/realtime|audio|tts|transcribe|image|imagine|video|embedding|lyria|-vision|deep-research|computer-use|customtools/i.test(m.id)) return false
  if (info(p).sdk === "openai" && /search|-pro$|codex-mini|^gpt-4|^o\d|chat-latest|\d{8}$/.test(m.id)) return false
  return true
}

/** Chat models with tool calling for the given providers, newest first. */
export function modelsFor(providers: Provider[]): ModelChoice[] {
  const out: ModelChoice[] = []
  for (const p of providers) {
    const models = Object.values(providerModels(info(p).catalog) ?? {})
      .filter((m) => usable(p, m))
      .sort((a, b) => (b.release_date ?? "").localeCompare(a.release_date ?? ""))
    for (const m of models) out.push({ full: `${p}/${m.id}`, provider: p, slug: m.id, name: m.name, context: m.limit?.input ?? m.limit?.context ?? 0, release: m.release_date ?? "" })
  }
  return out
}

/** The provider's preferred model, or its newest one if that's gone from the catalog. */
export function defaultFor(p: Provider): string {
  const preferred = info(p).defaultModel
  const models = providerModels(info(p).catalog)
  if (!models || (preferred && models[preferred])) return `${p}/${preferred}`
  return modelsFor([p])[0]?.full ?? `${p}/${preferred}`
}

/** Whether the model accepts image input (unknown models get the benefit of the doubt). */
export function acceptsImages(full: string): boolean {
  const m = lookup(full)
  if (!m) return true
  return m.modalities?.input ? m.modalities.input.includes("image") : m.attachment !== false
}

/** Token usage as the AI SDK reports it per step. */
export interface StepUsage {
  inputTokens?: number
  outputTokens?: number
  inputTokenDetails?: { noCacheTokens?: number; cacheReadTokens?: number; cacheWriteTokens?: number }
}

/** Whether this provider bills per token (not a subscription plan or a local server). */
export function metered(full: string): boolean {
  const { provider } = splitModel(full)
  return provider !== "chatgpt" && !info(provider).billing.includes("plan") && !info(provider).local
}

/** USD for one step from models.dev prices (per million tokens); undefined when unknown or not metered. */
export function costOf(full: string, u: StepUsage): number | undefined {
  if (!metered(full)) return undefined
  const c = lookup(full)?.cost
  if (!c || (c.input === undefined && c.output === undefined)) return undefined
  const read = u.inputTokenDetails?.cacheReadTokens ?? 0
  const write = u.inputTokenDetails?.cacheWriteTokens ?? 0
  const fresh = u.inputTokenDetails?.noCacheTokens ?? Math.max(0, (u.inputTokens ?? 0) - read - write)
  const input = c.input ?? 0
  return (fresh * input + read * (c.cache_read ?? input) + write * (c.cache_write ?? input) + (u.outputTokens ?? 0) * (c.output ?? 0)) / 1e6
}

export function displayName(full: string): string {
  return lookup(full)?.name ?? splitModel(full).slug
}

export function pickDefault(available: Provider[], current: string): string {
  if (current && available.includes(splitModel(current).provider)) return current
  return defaultFor(available[0] ?? "chatgpt")
}
