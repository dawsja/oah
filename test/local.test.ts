import { afterAll, expect, test } from "bun:test"
import { detectLocal, isRunning } from "../src/local"
import { contextWindow, modelsFor } from "../src/catalog"

const server = Bun.serve({
  port: 0,
  fetch: async (req) => {
    const path = new URL(req.url).pathname
    if (path === "/api/tags") return Response.json({ models: [{ name: "coder:7b" }, { name: "seeing:4b" }, { name: "embed:1" }] })
    if (path === "/api/show") {
      const { model } = (await req.json()) as any
      const caps = { "coder:7b": ["completion", "tools"], "seeing:4b": ["completion", "tools", "vision"], "embed:1": ["embedding"] }[model as string]
      return Response.json({ capabilities: caps, model_info: { "x.context_length": 131072 } })
    }
    return new Response("nope", { status: 404 })
  },
})
afterAll(() => {
  server.stop(true)
  delete process.env.OAH_OLLAMA_BASE_URL
  delete process.env.OAH_LMSTUDIO_BASE_URL
  delete process.env.OLLAMA_CONTEXT_LENGTH
})

test("detects a running Ollama, lists tool-capable chat models, honors OLLAMA_CONTEXT_LENGTH", async () => {
  process.env.OAH_OLLAMA_BASE_URL = `http://127.0.0.1:${server.port}/v1`
  process.env.OAH_LMSTUDIO_BASE_URL = "http://127.0.0.1:9/v1" // nothing there
  process.env.OLLAMA_CONTEXT_LENGTH = "32768"
  expect(await detectLocal()).toEqual(["ollama"])
  expect(isRunning("ollama")).toBe(true)
  expect(isRunning("lmstudio")).toBe(false)
  expect(modelsFor(["ollama"]).map((m) => m.full).sort()).toEqual(["ollama/coder:7b", "ollama/seeing:4b"])
  expect(contextWindow("ollama/coder:7b")).toBe(32768)
})
