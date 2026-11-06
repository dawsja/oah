// Every provider builds a model that calls the right endpoint with its key, and the
// reasoning options we send pass the AI SDK provider's own validation.
import { afterAll, beforeAll, expect, test } from "bun:test"
import { mkdtempSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"

const home = mkdtempSync(join(tmpdir(), "oah-prov-"))
const effort = [{ type: "effort", values: ["low", "medium", "high"] }]
const model = (id: string, values = effort) => ({ [id]: { id, name: id, tool_call: true, reasoning: true, reasoning_options: values } })
writeFileSync(
  join(home, "models.json"),
  JSON.stringify({
    google: { id: "google", name: "Google", models: model("gemini-4-argon") },
    xai: { id: "xai", name: "xAI", models: model("grok-4.7") },
    deepseek: { id: "deepseek", name: "DeepSeek", models: model("deepseek-v4-pro", [{ type: "effort", values: ["low", "high", "max"] }]) },
    openrouter: { id: "openrouter", name: "OpenRouter", models: model("anthropic/claude-opus-5.5") },
    synthetic: { id: "synthetic", name: "Synthetic", models: model("hf:moonshotai/Kimi-K3") },
    moonshotai: { id: "moonshotai", name: "Kimi", models: model("kimi-k3.1-pro") },
    "kimi-code-plan-global": { id: "kimi-code-plan-global", name: "Kimi Code", models: model("kimi-for-coding") },
    zai: { id: "zai", name: "Z.ai", models: model("glm-5.3") },
    "zai-coding-plan": { id: "zai-coding-plan", name: "Z.ai Coding", models: model("glm-5.3") },
    minimax: { id: "minimax", name: "MiniMax", models: model("MiniMax-M3") },
    "minimax-coding-plan": { id: "minimax-coding-plan", name: "MiniMax Coding", models: model("MiniMax-M3") },
  }),
)

const cases: [string, string, string][] = [
  ["google/gemini-4-argon", "generativelanguage.googleapis.com", "x-goog-api-key"],
  ["xai/grok-4.7", "api.x.ai", "authorization"],
  ["deepseek/deepseek-v4-pro", "api.deepseek.com", "authorization"],
  ["openrouter/anthropic/claude-opus-5.5", "openrouter.ai", "authorization"],
  ["synthetic/hf:moonshotai/Kimi-K3", "api.synthetic.new", "authorization"],
  ["kimi/kimi-k3.1-pro", "api.moonshot.ai", "authorization"],
  ["kimi-code/kimi-for-coding", "api.kimi.ai", "authorization"],
  ["zai/glm-5.3", "api.z.ai", "authorization"],
  ["zai-coding/glm-5.3", "api.z.ai", "authorization"],
  ["minimax/MiniMax-M3", "api.minimax.io", "x-api-key"],
  ["minimax-coding/MiniMax-M3", "api.minimax.io", "x-api-key"],
]

const realFetch = globalThis.fetch
let seen: { url: string; headers: Headers; body: string }[] = []

beforeAll(async () => {
  process.env.OAH_HOME = home
  ;(await import("../src/catalog")).reloadCatalog()
  for (const k of ["GEMINI_API_KEY", "XAI_API_KEY", "DEEPSEEK_API_KEY", "OPENROUTER_API_KEY", "SYNTHETIC_API_KEY", "MOONSHOT_API_KEY", "KIMI_API_KEY", "ZHIPU_API_KEY", "MINIMAX_API_KEY"]) process.env[k] = `test-${k}`
  globalThis.fetch = Object.assign(
    async (input: RequestInfo | URL, init?: RequestInit) => {
      seen.push({ url: String(input instanceof Request ? input.url : input), headers: new Headers(init?.headers), body: String(init?.body ?? "") })
      return new Response(JSON.stringify({ error: { message: "stop here" } }), { status: 400, headers: { "content-type": "application/json" } })
    },
    { preconnect: realFetch.preconnect },
  ) as typeof fetch
})
afterAll(() => {
  globalThis.fetch = realFetch
  delete process.env.OAH_HOME
})

for (const [full, host, header] of cases) {
  test(`${full} → ${host}`, async () => {
    const { generateText } = await import("ai")
    const { languageModel, providerOptions } = await import("../src/providers")
    const { defaultSettings } = await import("../src/config")
    seen = []
    const err = await generateText({
      model: await languageModel(full, defaultSettings),
      prompt: "hi",
      maxRetries: 0,
      providerOptions: providerOptions(full, "high"),
    }).catch((e) => e)
    // A provider-options validation failure would throw before any request.
    expect(seen.length).toBe(1)
    expect(err).toBeInstanceOf(Error)
    const req = seen[0]!
    expect(new URL(req.url).host).toBe(host)
    expect(req.headers.get(header) ?? "").toContain("test-")
  })
}

test("bare slugs infer their provider", async () => {
  const { splitModel } = await import("../src/config")
  expect(splitModel("gemini-4-argon").provider).toBe("google")
  expect(splitModel("grok-4.7").provider).toBe("xai")
  expect(splitModel("glm-5.3").provider).toBe("zai")
  expect(splitModel("openrouter/openai/gpt-5.6")).toEqual({ provider: "openrouter", slug: "openai/gpt-5.6" })
})

test("reasoning effort is sent in each provider's own shape", async () => {
  const { providerOptions } = await import("../src/providers")
  expect(providerOptions("google/gemini-4-argon", "high")).toEqual({ google: { thinkingConfig: { includeThoughts: true, thinkingLevel: "high" } } })
  expect(providerOptions("deepseek/deepseek-v4-pro", "xhigh")).toEqual({ deepseek: { reasoningEffort: "high" } })
  expect(providerOptions("zai/glm-5.3", "max")).toEqual({ zai: { reasoningEffort: "high" } })
  expect(providerOptions("zai-coding/glm-5.3", "high")).toEqual({ zaiCoding: { reasoningEffort: "high" } })
  expect(providerOptions("openrouter/anthropic/claude-opus-5.5", "medium")).toEqual({ openrouter: { reasoning: { effort: "medium" } } })
})

test("MiniMax gets the model's output limit, not the SDK's 4096 default", async () => {
  const { maxOutputTokens } = await import("../src/providers")
  expect(maxOutputTokens("minimax/MiniMax-M3")).toBe(32_000)
  expect(maxOutputTokens("anthropic/claude-opus-5-5")).toBeUndefined()
  expect(maxOutputTokens("google/gemini-4-argon")).toBeUndefined()
})

test("cost uses models.dev prices, including cache reads/writes, and skips plans", async () => {
  const { costOf } = await import("../src/catalog")
  const { writeFileSync } = await import("node:fs")
  const { join } = await import("node:path")
  writeFileSync(
    join(home, "models.json"),
    JSON.stringify({
      anthropic: { id: "anthropic", name: "Anthropic", models: { "claude-x": { id: "claude-x", name: "X", cost: { input: 5, output: 25, cache_read: 0.5, cache_write: 6.25 } } } },
      "zai-coding-plan": { id: "zai-coding-plan", name: "Z", models: { "glm-5.3": { id: "glm-5.3", name: "GLM", cost: { input: 1, output: 4 } } } },
    }),
  )
  process.env.OAH_HOME = home
  ;(await import("../src/catalog")).reloadCatalog()
  // 1M fresh input ($5) + 2M cache reads ($1) + 1M cache writes ($6.25) + 0.1M output ($2.50)
  const u = { inputTokens: 4_000_000, outputTokens: 100_000, inputTokenDetails: { noCacheTokens: 1_000_000, cacheReadTokens: 2_000_000, cacheWriteTokens: 1_000_000 } }
  expect(costOf("anthropic/claude-x", u)).toBeCloseTo(14.75, 6)
  expect(costOf("anthropic/claude-x", { inputTokens: 1_000_000, outputTokens: 0 })).toBeCloseTo(5, 6)
  expect(costOf("zai-coding/glm-5.3", u)).toBeUndefined() // subscription plan
  expect(costOf("chatgpt/gpt-5.6", u)).toBeUndefined()
})

test("clients are memoized but credentials and endpoint overrides invalidate them", async () => {
  const { languageModel, providerOptions, cachedSystem } = await import("../src/providers")
  const { defaultSettings } = await import("../src/config")
  const a = await languageModel("minimax/MiniMax-M3", defaultSettings)
  expect(await languageModel("minimax/MiniMax-M3", defaultSettings)).toBe(a)
  process.env.OAH_MINIMAX_BASE_URL = "https://example.test/v1"
  try { expect(await languageModel("minimax/MiniMax-M3", defaultSettings)).not.toBe(a) }
  finally { delete process.env.OAH_MINIMAX_BASE_URL }
  expect(providerOptions("openai/gpt-5.6", "medium", "session").openai.promptCacheKey).toBe("oah:session")
  expect(providerOptions("minimax/MiniMax-M3", "medium").anthropic.cacheControl).toBeUndefined()
  expect(cachedSystem("anthropic/claude-opus-5-5", "stable").providerOptions).toBeDefined()
  expect(cachedSystem("minimax/MiniMax-M3", "stable").providerOptions).toBeUndefined()
})
