import { expect, test } from "bun:test"
import { jsonSchema, stepCountIs, streamText, tool, wrapLanguageModel } from "ai"
import { MockLanguageModelV4 } from "ai/test"
import { reviewBarrier } from "../src/safety-batch"

const usage = { inputTokens: { total: 10, noCache: 10, cacheRead: 0, cacheWrite: 0 }, outputTokens: { total: 10, text: 10, reasoning: 0 } }
function model(parts: any[]) {
  return new MockLanguageModelV4({ doStream: async () => ({ stream: new ReadableStream({ start(c) { for (const part of parts) c.enqueue(part); c.close() } }) }) })
}
const finish = { type: "finish", finishReason: { unified: "tool-calls", raw: "tool_calls" }, usage }
test("one complete batch is reviewed before any SDK tool executes", async () => {
  const events: string[] = []
  const wrapped = wrapLanguageModel({ model: model([
    { type: "tool-call", toolCallId: "1", toolName: "inspect", input: '{"n":1}' },
    { type: "tool-call", toolCallId: "2", toolName: "inspect", input: '{"n":2}' }, finish,
  ]), middleware: reviewBarrier(async (calls) => {
    expect(calls.map((c) => c.id)).toEqual(["1", "2"])
    events.push("review-start")
    await Bun.sleep(10)
    events.push("review-end")
  }) })
  const result = streamText({ model: wrapped, prompt: "inspect", stopWhen: stepCountIs(1), tools: {
    inspect: tool({ inputSchema: jsonSchema<{ n: number }>({ type: "object", properties: { n: { type: "number" } }, required: ["n"] }), execute: async ({ n }) => { events.push(`execute-${n}`); return "ok" } }),
  } })
  for await (const _ of result.fullStream) {}
  expect(events).toEqual(["review-start", "review-end", "execute-1", "execute-2"])
})
test("malformed calls remain visible to completion policy and repair gets final arguments", async () => {
  let rawInvalid = false
  let executed = false
  const wrapped = wrapLanguageModel({ model: model([{ type: "tool-call", toolCallId: "bad", toolName: "write", input: '{"n":' }, finish]), middleware: reviewBarrier(async (calls) => {
    rawInvalid = calls[0]?.invalid === true
  }) })
  const result = streamText({ model: wrapped, prompt: "repair", stopWhen: stepCountIs(1), repairToolCall: async ({ toolCall }) => ({ ...toolCall, input: '{"n":1}' }), tools: {
    write: tool({ inputSchema: jsonSchema<{ n: number }>({ type: "object", properties: { n: { type: "number" } }, required: ["n"] }), execute: async ({ n }) => { expect(rawInvalid).toBe(true); expect(n).toBe(1); executed = true; return "ok" } }),
  } })
  for await (const _ of result.fullStream) {}
  expect(executed).toBe(true)
})

test("verdict parsing fails closed for omissions, duplicates and unexpected IDs", async () => {
  const { parseVerdicts, reviewBatch } = await import("../src/safety")
  const { defaultSettings } = await import("../src/config")
  const actions = [{ id: "a", name: "bash", args: { command: "test" }, kind: "exec" }, { id: "b", name: "bash", args: {}, kind: "exec" }]
  const allow = { id: "a", decision: "allow", reason: "okay" }
  for (const verdicts of [[allow], [allow, allow], [allow, { ...allow, id: "unknown" }]]) expect(() => parseVerdicts(JSON.stringify({ verdicts }), actions)).toThrow()
  expect(parseVerdicts(JSON.stringify({ verdicts: [allow, { id: "b", decision: "block", reason: "unsafe" }] }), actions).get("b")?.allow).toBe(false)
  const oversized = await reviewBatch({ model: "not-used", settings: defaultSettings, cwd: "/", intent: "", signal: new AbortController().signal }, [{ ...actions[0]!, args: { command: "x".repeat(60_000) } }])
  expect(oversized.get("a")?.error).toBe(true)
})
