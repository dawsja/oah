import { expect, test } from "bun:test"
import type { ModelMessage } from "ai"
import { elideImages, elideToolResults, splitRecent } from "../src/context"

function history(): ModelMessage[] {
  return Array.from({ length: 8 }, (_, i): ModelMessage[] => [
    { role: "assistant", content: [{ type: "tool-call", toolCallId: `c${i}`, toolName: "read_file", input: { path: `file${i}` }, providerOptions: { test: { signature: "keep" } } }] },
    { role: "tool", content: [{ type: "tool-result", toolCallId: `c${i}`, toolName: "read_file", output: { type: "text", value: Array.from({ length: 150 }, (_, l) => `${l}\t${"x".repeat(100)}`).join("\n") } }] },
  ]).flat()
}
test("elision preserves tool pairing, metadata, recent results and original history", () => {
  const h = history()
  const out = elideToolResults(h)
  expect(out.length).toBe(h.length)
  expect(out[0]).toBe(h[0])
  expect(JSON.stringify(out[1])).toContain("elided read_file file0")
  expect(out.slice(-8)).toEqual(h.slice(-8))
  expect(JSON.stringify(h[1])).not.toContain("elided")
  expect(elideToolResults(out)).toEqual(out)
  expect(JSON.stringify(out).length).toBeLessThan(JSON.stringify(h).length)
})
test("small histories are untouched and keep=0 is supported", () => {
  expect(elideToolResults([])).toEqual([])
  expect(JSON.stringify(elideToolResults(history(), { keep: 0, threshold: 0 }).at(-1))).toContain("elided")
})
test("retain complete recent exchanges", () => {
  const h: ModelMessage[] = [{ role: "user", content: "first" }, ...history(), { role: "user", content: "second" }, ...history(), { role: "user", content: "third" }, ...history()]
  const { prefix, tail } = splitRecent(h)
  expect(prefix.length).toBe(17)
  expect(tail[0]).toEqual({ role: "user", content: "second" })
})
test("image elision leaves text and other parts intact", () => {
  const h: ModelMessage[] = [{ role: "user", content: [{ type: "text", text: "look" }, { type: "image", image: "secret-base64" }] }]
  expect(JSON.stringify(elideImages(h))).not.toContain("secret-base64")
  expect(JSON.stringify(elideImages(h))).toContain("look")
})

test("signed Anthropic history is not silently rebased by pruning or image removal", () => {
  const h = history()
  h.unshift({ role: "assistant", content: [{ type: "reasoning", text: "signed", providerOptions: { anthropic: { signature: "bound-to-prefix" } } }] })
  expect(elideToolResults(h, { threshold: 0 })).toBe(h)
  expect(elideImages(h)).toBe(h)
})
