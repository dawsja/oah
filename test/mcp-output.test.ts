import { expect, test } from "bun:test"
import { mcpText } from "../src/agent"

test("MCP text caps preserve error flags and complete bounded structured values", () => {
  const data = { value: "v".repeat(20_000) }
  const r = mcpText({ isError: true, content: [{ type: "text", text: "x".repeat(50_000) }], structuredContent: data })
  expect(r.isError).toBe(true)
  expect(r.text.length).toBeLessThanOrEqual(30_000)
  expect(r.text).toEndWith(JSON.stringify(data))
  expect(mcpText({ content: [{ type: "image", mimeType: "image/png", data: "private-base64" }] }).text).not.toContain("private-base64")
})
test("oversized structured values are omitted whole, not serialized then sliced", () => {
  const r = mcpText({ structuredContent: { value: "private".repeat(10_000) } })
  expect(r.text).toContain("structured content omitted")
  expect(r.text).not.toContain('"value"')
})
