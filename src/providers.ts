// AI SDK models for each provider.
//  * chatgpt   → OpenAI Responses API, Authorization from Sign in with ChatGPT
//  * openai    → OpenAI Responses API with an API key
//  * anthropic → Anthropic Messages API with an API key
import { createAnthropic } from "@ai-sdk/anthropic"
import { createOpenAI } from "@ai-sdk/openai"
import type { LanguageModel } from "ai"
import * as auth from "./auth"
import { clampEffort, lookup } from "./catalog"
import { splitModel, VERSION, type Effort, type Settings } from "./config"

const UA = `nur/${VERSION} (${process.platform}; ${process.arch})`

/** fetch that injects a fresh ChatGPT access token and retries once after a 401. */
const chatgptFetch: typeof fetch = Object.assign(
  async (input: RequestInfo | URL, init?: RequestInit) => {
    const send = async (token: string) => {
      const headers = new Headers(init?.headers)
      headers.set("Authorization", `Bearer ${token}`)
      headers.set("User-Agent", UA)
      return fetch(input, { ...init, headers })
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

export async function languageModel(full: string, settings: Settings): Promise<LanguageModel> {
  const { provider, slug } = splitModel(full)
  if (provider === "anthropic") {
    const apiKey = await auth.credential("anthropic")
    const base = settings.anthropicBaseURL ?? process.env.ANTHROPIC_BASE_URL
    return createAnthropic({ apiKey, baseURL: base ? `${base.replace(/\/$/, "")}/v1` : undefined, headers: { "User-Agent": UA } })(slug)
  }
  const baseURL = settings.openaiBaseURL ?? process.env.OPENAI_BASE_URL
  if (provider === "chatgpt") {
    return createOpenAI({ apiKey: "chatgpt-plan", baseURL, fetch: chatgptFetch }).responses(slug)
  }
  const apiKey = await auth.credential("openai")
  return createOpenAI({ apiKey, baseURL, headers: { "User-Agent": UA } }).responses(slug)
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

export function providerOptions(full: string, effort: Effort): Record<string, any> {
  const { provider, slug } = splitModel(full)
  const e = clampEffort(full, effort)
  if (provider === "anthropic") {
    const o: Record<string, any> = { cacheControl: { type: "ephemeral" } }
    if (adaptiveThinking(slug)) {
      o.thinking = { type: "adaptive", display: "summarized" }
      if (e) o.effort = e === "minimal" ? "low" : e
    }
    if (FALLBACK_MODELS.has(slug)) o.fallbacks = "default"
    return { anthropic: o }
  }
  const o: Record<string, any> = { store: false, parallelToolCalls: true }
  const reasoning = lookup(full)?.reasoning ?? /^(gpt-5|gpt-6|o\d)/.test(slug)
  if (reasoning) {
    if (e) o.reasoningEffort = e
    o.reasoningSummary = "auto"
    o.include = ["reasoning.encrypted_content"]
  }
  return { openai: o }
}

export { adaptiveThinking }
