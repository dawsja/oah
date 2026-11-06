// Local model servers: Ollama and LM Studio. No key — a provider counts as
// connected while its server answers. Models (with context size, tool and
// vision support) come from the server, not models.dev.
import { registerLocal, type CatalogModel } from "./catalog"
import { info, type Provider } from "./config"

export const LOCAL_PROVIDERS = ["ollama", "lmstudio"] as const satisfies Provider[]
export type LocalProvider = (typeof LOCAL_PROVIDERS)[number]

const running = new Set<LocalProvider>()
export const isRunning = (p: Provider) => running.has(p as LocalProvider)

/** OpenAI-compatible base URL (…/v1), honoring OAH_<P>_BASE_URL and OLLAMA_HOST. */
export function localBaseURL(p: LocalProvider): string {
  const override = process.env[`OAH_${p.toUpperCase()}_BASE_URL`]
  if (override) return override.replace(/\/$/, "")
  if (p === "ollama" && process.env.OLLAMA_HOST) {
    const h = process.env.OLLAMA_HOST.replace(/\/$/, "")
    return `${/^https?:\/\//.test(h) ? h : `http://${h}`}/v1`
  }
  return info(p).baseURL!
}

const root = (p: LocalProvider) => localBaseURL(p).replace(/\/v1$/, "")

async function getJson(url: string, ms: number, init?: RequestInit): Promise<any> {
  const r = await fetch(url, { ...init, signal: AbortSignal.timeout(ms) })
  if (!r.ok) throw new Error(`${r.status}`)
  return r.json()
}

async function ollamaModels(): Promise<CatalogModel[]> {
  const tags = await getJson(`${root("ollama")}/api/tags`, 800)
  const envCtx = Number(process.env.OLLAMA_CONTEXT_LENGTH) || 0
  const out: CatalogModel[] = []
  await Promise.all(
    (tags.models ?? []).map(async (m: any) => {
      let caps: string[] | undefined
      let ctx = 0
      try {
        const show = await getJson(`${root("ollama")}/api/show`, 2000, { method: "POST", body: JSON.stringify({ model: m.name }) })
        caps = show.capabilities
        for (const [k, v] of Object.entries<any>(show.model_info ?? {})) if (k.endsWith(".context_length")) ctx = Number(v)
      } catch {}
      if (caps && !caps.includes("completion")) return // embedding models
      out.push({
        id: m.name,
        name: m.name,
        tool_call: caps ? caps.includes("tools") : true,
        reasoning: caps?.includes("thinking"),
        // Ollama serves OLLAMA_CONTEXT_LENGTH tokens (if set), never more than the model supports.
        limit: { context: envCtx && ctx ? Math.min(envCtx, ctx) : envCtx || ctx || undefined },
        modalities: { input: caps?.includes("vision") ? ["text", "image"] : ["text"] },
        release_date: m.modified_at ?? "",
      })
    }),
  )
  return out
}

async function lmstudioModels(): Promise<CatalogModel[]> {
  try {
    // LM Studio's own API knows context length, type and tool support.
    const r = await getJson(`${root("lmstudio")}/api/v0/models`, 800)
    return (r.data ?? [])
      .filter((m: any) => m.type !== "embeddings")
      .map((m: any) => ({
        id: m.id,
        name: m.id,
        tool_call: Array.isArray(m.capabilities) ? m.capabilities.includes("tool_use") : true,
        limit: { context: m.loaded_context_length || m.max_context_length || undefined },
        modalities: { input: m.type === "vlm" ? ["text", "image"] : ["text"] },
        release_date: m.state === "loaded" ? "1" : "",
      }))
  } catch {
    const r = await getJson(`${localBaseURL("lmstudio")}/models`, 800)
    return (r.data ?? []).filter((m: any) => !/embed/i.test(m.id)).map((m: any) => ({ id: m.id, name: m.id, tool_call: true }))
  }
}

/** Probe the local servers and register whichever are running. Never throws. */
export async function detectLocal(): Promise<LocalProvider[]> {
  await Promise.all(
    LOCAL_PROVIDERS.map(async (p) => {
      try {
        const models = p === "ollama" ? await ollamaModels() : await lmstudioModels()
        registerLocal(info(p).catalog, info(p).label, models)
        running.add(p)
      } catch {
        registerLocal(info(p).catalog, info(p).label, [])
        running.delete(p)
      }
    }),
  )
  return [...running]
}
