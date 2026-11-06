import { expect, test } from "bun:test"
import { MockLanguageModelV4 } from "ai/test"
import { mkdtempSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { tmpdir } from "node:os"
import { runTurn, type TurnOptions } from "../src/agent"
import { defaultSettings } from "../src/config"
import { Skills } from "../src/skills"
import { Memory } from "../src/memory"

const usage = { inputTokens: { total: 10, noCache: 2, cacheRead: 8, cacheWrite: 0 }, outputTokens: { total: 2, text: 2, reasoning: 0 } }
const call = (id: string, name: string, args: unknown) => ({ type: "tool-call", toolCallId: id, toolName: name, input: JSON.stringify(args) })
function response(parts: any[], reason = "tool-calls") {
  return { stream: new ReadableStream({ start(c) { for (const p of [...parts, { type: "finish", finishReason: { unified: reason, raw: reason }, usage }]) c.enqueue(p); c.close() } }) }
}
test("real SDK loop preserves prepared history and gates completion around writes/checks", async () => {
  const cwd = mkdtempSync(join(tmpdir(), "oah-loop-"))
  writeFileSync(join(cwd, "a.txt"), "old\n")
  await Bun.$`git -C ${cwd} init -q`.quiet()
  await Bun.$`git -C ${cwd} add a.txt`.quiet()
  const model = new MockLanguageModelV4({ doStream: [
    response([call("early", "goal_complete", { summary: "too soon" }), call("write", "write_file", { path: "a.txt", content: "new\n" })]),
    response([call("check", "bash", { command: "git diff --check" })]),
    response([call("done", "goal_complete", { summary: "verified" })]),
    response([{ type: "text-start", id: "text" }, { type: "text-delta", id: "text", delta: "Verified." }, { type: "text-end", id: "text" }], "stop"),
  ] })
  let completed = 0
  const denied: string[] = []
  const usages: any[] = []
  let verification: any
  const o: TurnOptions = {
    model: "openai/test-model", modelInstance: model, effort: "low", mode: "build", permission: "allow", instructions: "Test agent", messages: [{ role: "user", content: "write, verify, complete" }], settings: { ...defaultSettings, repairToolCalls: false },
    ctx: { cwd, skills: new Skills(cwd), memory: new Memory(cwd) }, mcp: { tools: () => ({}) } as any, grants: new Set(), goalActive: true, signal: new AbortController().signal,
    hooks: { text() {}, reasoning() {}, reasoningEnd() {}, toolStart() {}, toolEnd(id, _result, wasDenied) { if (wasDenied) denied.push(id) }, ask: async () => "deny", todos() {}, goalComplete() { completed++ }, usage(u) { usages.push(u) }, spend() {}, toolProgress() {}, question: async () => "", beforeChange: async () => {}, verification(s) { verification = structuredClone(s) } },
  }
  const result = await runTurn(o)
  expect(result.error).toBeUndefined()
  expect(result.aborted).toBe(false)
  expect(denied).toEqual(["early"])
  expect(completed).toBe(1)
  expect(verification.status).toBe("checks-passed")
  expect(usages).toHaveLength(4)
  expect(usages[0].inputTokenDetails.cacheReadTokens).toBe(8)
  expect(JSON.stringify(result.history)).toContain("[oah verification:")
  expect(model.doStreamCalls).toHaveLength(4)
})
