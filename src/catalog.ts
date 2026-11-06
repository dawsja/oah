// Model catalog from https://models.dev — context windows, reasoning effort
// levels, costs and capabilities. Cached in ~/.config/oah/models.json for a day;
// a small built-in table keeps oah working offline.
import { statSync } from "node:fs"
import { defaultModel, paths, providerInfo, readJson, splitModel, writeFileAtomic, type Effort, type Provider } from "./config"

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

let catalog: Catalog = readJson<Catalog>(paths.modelsCache, FALLBACK)

function cacheAge(): number {
  try {
    return Date.now() - statSync(paths.modelsCache).mtimeMs
  } catch {
    return Infinity
  }
}

/** Refresh from models.dev when the cache is stale. Never throws. */
export async function refreshCatalog(force = false): Promise<boolean> {
  if (!force && cacheAge() < TTL_MS) return false
  try {
    const r = await fetch(process.env.OAH_MODELS_URL ?? URL, { signal: AbortSignal.timeout(10_000) })
    if (!r.ok) return false
    const all = (await r.json()) as Catalog
    const slim: Catalog = {}
    for (const id of ["openai", "anthropic"]) if (all[id]) slim[id] = all[id]!
    catalog = slim
    writeFileAtomic(paths.modelsCache, JSON.stringify(slim))
    return true
  } catch {
    return false
  }
}

export function lookup(full: string): CatalogModel | undefined {
  const { provider, slug } = splitModel(full)
  return catalog[providerInfo[provider].catalog]?.models[slug]
}

export function contextWindow(full: string, override?: number): number {
  if (override) return override
  const m = lookup(full)
  return m?.limit?.input ?? m?.limit?.context ?? (splitModel(full).provider === "anthropic" ? 1_000_000 : 400_000)
}

/** Effort levels the model accepts, mapped onto oah's scale. */
export function effortsFor(full: string): Effort[] {
  const m = lookup(full)
  const values = m?.reasoning_options?.find((o) => o.type === "effort")?.values
  const map = (v: string): Effort | null => (v === "none" ? null : (["minimal", "low", "medium", "high", "xhigh", "max"].includes(v) ? (v as Effort) : null))
  const out = (values ?? []).map(map).filter((e): e is Effort => e !== null)
  if (out.length) return out
  if (m && m.reasoning === false) return []
  return splitModel(full).provider === "anthropic" ? ["low", "medium", "high", "xhigh", "max"] : ["minimal", "low", "medium", "high", "xhigh"]
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

/** Chat models with tool calling for the given providers, newest first. */
export function modelsFor(providers: Provider[]): ModelChoice[] {
  const out: ModelChoice[] = []
  for (const p of providers) {
    const models = catalog[providerInfo[p].catalog]?.models ?? {}
    for (const m of Object.values(models)) {
      if (!m.tool_call || (m.type && m.type !== "chat")) continue
      if (m.status === "deprecated") continue
      if (/realtime|audio|tts|transcribe|image|embedding|search|-pro$|codex-mini|^gpt-4|^o\d|chat-latest|\d{8}$/.test(m.id)) continue
      out.push({ full: `${p}/${m.id}`, provider: p, slug: m.id, name: m.name, context: m.limit?.input ?? m.limit?.context ?? 0, release: m.release_date ?? "" })
    }
  }
  return out.sort((a, b) => (a.provider === b.provider ? b.release.localeCompare(a.release) : 0))
}

export function displayName(full: string): string {
  return lookup(full)?.name ?? splitModel(full).slug
}

export function pickDefault(available: Provider[], current: string): string {
  if (current && available.includes(splitModel(current).provider)) return current
  const p = available[0] ?? "chatgpt"
  return `${p}/${defaultModel[p]}`
}
