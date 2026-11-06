// AI SDK models for each provider.
//  * chatgpt   → OpenAI Responses API, Authorization from Sign in with ChatGPT
//  * openai    → OpenAI Responses API with an API key
//  * anthropic → Anthropic Messages API (also MiniMax's Anthropic-compatible API)
//  * google, xai, deepseek, openrouter → their AI SDK providers
//  * synthetic, kimi, z.ai → OpenAI-compatible chat completions
import { createAnthropic } from "@ai-sdk/anthropic"
import { createDeepSeek } from "@ai-sdk/deepseek"
import { createGoogleGenerativeAI } from "@ai-sdk/google"
import { createOpenAI } from "@ai-sdk/openai"
import { createOpenAICompatible } from "@ai-sdk/openai-compatible"
import { createXai } from "@ai-sdk/xai"
import { createOpenRouter } from "@openrouter/ai-sdk-provider"
import { streamText, type LanguageModel, type LanguageModelUsage, type SystemModelMessage } from "ai"
import * as auth from "./auth"
import { clampEffort, lookup } from "./catalog"
import { localBaseURL, type LocalProvider } from "./local"
import { info, splitModel, VERSION, type Effort, type Settings } from "./config"

const UA = `oah/${VERSION} (${process.platform}; ${process.arch})`

/** Bound connection/header stalls without imposing a total deadline on a healthy stream. */
const deadlineFetch: typeof fetch = Object.assign(async (input: RequestInfo | URL, init?: RequestInit) => {
  const abort = new AbortController()
  const timer = setTimeout(() => abort.abort(new Error("Provider connection timed out after 120s")), 120_000)
  const inherited = init?.signal ?? (input instanceof Request ? input.signal : undefined)
  try { return await fetch(input, { ...init, signal: inherited ? AbortSignal.any([inherited, abort.signal]) : abort.signal }) }
  finally { clearTimeout(timer) }
}, { preconnect: fetch.preconnect })

/** fetch that injects a fresh ChatGPT access token and retries once after a 401. */
const chatgptFetch: typeof fetch = Object.assign(
  async (input: RequestInfo | URL, init?: RequestInit) => {
    const send = async (token: string) => {
      const headers = new Headers(init?.headers)
      headers.set("Authorization", `Bearer ${token}`)
      headers.set("User-Agent", UA)
      return deadlineFetch(input, { ...init, headers })
    }
    let r = await send(await auth.credential("chatgpt"))
    if (r.status === 401) {
      const body = await r.clone().text()
      if (!body.includes("subscription_sharing_invalid_user")) {
        try {
          r = await send(await auth.refresh())
        } catch {}
      }
    }
    return r
  },
  { preconnect: fetch.preconnect },
)

/** providerOptions key for an OpenAI-compatible provider (camelCase: zai-coding → zaiCoding). */
const optionsKey = (p: string) => p.replace(/-(\w)/g, (_, c: string) => c.toUpperCase())

const modelCache = new Map<string, LanguageModel>()

export async function languageModel(full: string, settings: Settings): Promise<LanguageModel> {
  const { provider } = splitModel(full)
  // Resolve credentials before lookup: sign-in and endpoint changes must invalidate clients.
  const apiKey = provider === "chatgpt" ? "chatgpt-plan" : await auth.credential(provider)
  const key = JSON.stringify([full, apiKey, settings.openaiBaseURL, settings.anthropicBaseURL, process.env.OPENAI_BASE_URL, process.env.ANTHROPIC_BASE_URL, process.env[`OAH_${provider.toUpperCase().replace(/-/g, "_")}_BASE_URL`], provider === "ollama" || provider === "lmstudio" ? localBaseURL(provider) : null])
  const hit = modelCache.get(key)
  if (hit) return hit
  const model = createModel(full, settings, apiKey)
  modelCache.set(key, model)
  if (modelCache.size > 32) modelCache.delete(modelCache.keys().next().value!)
  return model
}

function createModel(full: string, settings: Settings, apiKey: string): LanguageModel {
  const { provider, slug } = splitModel(full)
  const headers = { "User-Agent": UA }
  if (provider === "chatgpt") {
    return createOpenAI({ apiKey: "chatgpt-plan", baseURL: settings.openaiBaseURL ?? process.env.OPENAI_BASE_URL, fetch: chatgptFetch }).responses(slug)
  }
  const def = info(provider)
  // OAH_<PROVIDER>_BASE_URL points any provider at a proxy or gateway.
  const override = process.env[`OAH_${provider.toUpperCase().replace(/-/g, "_")}_BASE_URL`]
  const baseURL = def.local ? localBaseURL(provider as LocalProvider) : (override ?? def.baseURL)
  switch (def.sdk) {
    case "openai":
      return createOpenAI({ apiKey, baseURL: override ?? settings.openaiBaseURL ?? process.env.OPENAI_BASE_URL, headers, fetch: deadlineFetch }).responses(slug)
    case "anthropic": {
      const base = provider === "anthropic" ? (settings.anthropicBaseURL ?? process.env.ANTHROPIC_BASE_URL) : undefined
      return createAnthropic({ apiKey, baseURL: base ? `${base.replace(/\/$/, "")}/v1` : baseURL, headers, fetch: deadlineFetch })(slug)
    }
    case "google":
      return createGoogleGenerativeAI({ apiKey, baseURL, headers, fetch: deadlineFetch })(slug)
    case "xai":
      return createXai({ apiKey, baseURL, headers, fetch: deadlineFetch })(slug)
    case "deepseek":
      return createDeepSeek({ apiKey, baseURL, headers, fetch: deadlineFetch })(slug)
    case "openrouter":
      return createOpenRouter({ apiKey, baseURL, headers: { ...headers, "HTTP-Referer": "https://oah.dev", "X-Title": "oah" }, fetch: deadlineFetch })(slug)
    case "compat":
      return createOpenAICompatible({ name: optionsKey(provider), apiKey, baseURL: baseURL!, headers, fetch: deadlineFetch, includeUsage: true })(slug)
  }
}

/** Models that accept Anthropic's server-side refusal fallbacks. */
const FALLBACK_MODELS = new Set(["claude-opus-5-5", "claude-opus-5", "claude-fable-5-1", "claude-sonnet-5-5"])

/** Anthropic models with adaptive thinking (4.6+; not Haiku 4.x). */
function adaptiveThinking(slug: string): boolean {
  const m = /^claude-(\w+)-(\d+)(?:-(\d{1,2}))?(?:-|$)/.exec(slug)
  if (!m) return false
  const [, family, major, minor] = m
  const maj = Number(major)
  const min = Number(minor ?? 0)
  return maj >= 5 || (maj === 4 && min >= 6 && family !== "haiku")
}

export function providerOptions(full: string, effort: Effort, sessionId?: string): Record<string, any> {
  const { provider, slug } = splitModel(full)
  const e = clampEffort(full, effort)
  const sdk = info(provider).sdk
  if (provider === "anthropic") {
    const o: Record<string, any> = { cacheControl: { type: "ephemeral" } }
    if (adaptiveThinking(slug)) {
      o.thinking = { type: "adaptive", display: "summarized" }
      if (e) o.effort = e === "minimal" ? "low" : e
    }
    if (FALLBACK_MODELS.has(slug)) o.fallbacks = "default"
    return { anthropic: o }
  }
  if (sdk === "anthropic") return { anthropic: e ? { effort: e } : {} } // MiniMax
  if (sdk === "google") return { google: { thinkingConfig: { includeThoughts: true, ...(e ? { thinkingLevel: e === "xhigh" || e === "max" ? "high" : e } : {}) } } }
  if (sdk === "xai") return e ? { xai: { reasoningEffort: e === "minimal" ? "low" : e === "max" ? "xhigh" : e } } : {}
  if (sdk === "deepseek") return e ? { deepseek: { reasoningEffort: e } } : {}
  if (sdk === "openrouter") return e ? { openrouter: { reasoning: { effort: e === "max" ? "xhigh" : e } } } : {}
  if (sdk === "compat") return e ? { [optionsKey(provider)]: { reasoningEffort: e } } : {}
  const o: Record<string, any> = { store: false, parallelToolCalls: true }
  if (sessionId) o.promptCacheKey = `oah:${sessionId}`
  const reasoning = lookup(full)?.reasoning ?? /^(gpt-5|gpt-6|o\d)/.test(slug)
  if (reasoning) {
    if (e) o.reasoningEffort = e
    o.reasoningSummary = "auto"
    o.include = ["reasoning.encrypted_content"]
  }
  return { openai: o }
}

/**
 * Output cap for providers whose SDK can't know the model's limit (MiniMax via the
 * Anthropic SDK would otherwise default to 4096 tokens). Undefined = SDK default.
 */
export function maxOutputTokens(full: string): number | undefined {
  const { provider } = splitModel(full)
  if (provider === "anthropic" || info(provider).sdk !== "anthropic") return undefined
  return Math.min(lookup(full)?.limit?.output ?? 32_000, 64_000)
}

export { adaptiveThinking }

/** One explicit stable-system endpoint plus Anthropic's existing automatic history caching. */
export function cachedSystem(full: string, content: string): SystemModelMessage {
  return { role: "system", content, ...(splitModel(full).provider === "anthropic" ? { providerOptions: { anthropic: { cacheControl: { type: "ephemeral" } } } } : {}) }
}

/**
 * A one-shot completion, streamed: some backends (the ChatGPT plan) reject
 * non-streaming requests, so this is used instead of `generateText`.
 */
export async function complete(args: Parameters<typeof streamText>[0]): Promise<{ text: string; usage: LanguageModelUsage }> {
  // onError: the error is rethrown below; the default handler would print it over the TUI.
  const r = streamText({ ...args, onError: () => {} })
  let text = ""
  let error: unknown
  for await (const part of r.fullStream) {
    if (part.type === "text-delta") text += part.text
    else if (part.type === "error") error = part.error
    else if (part.type === "abort") error = new Error("aborted")
  }
  if (error) throw error
  return { text, usage: await r.totalUsage }
}
