import { expect, test } from "bun:test"
import { RGBA, TextAttributes, type CapturedSpan } from "@opentui/core"
import { testRender } from "@opentui/react/test-utils"
import { act } from "react"
import { TodoPanel } from "../src/ui/App"
import type { Store } from "../src/ui/store"
import { SPINNER, t } from "../src/ui/theme"

const todos = [
  { text: "Inspect the repo", status: "completed" },
  { text: "Add the benchmark", status: "completed" },
  { text: "Run final verification", status: "in_progress" },
  { text: "Write the summary", status: "pending" },
]

const fakeStore = (busy: boolean) => ({ session: { todos }, busy: busy ? { kind: "turn", started: 0, abort: new AbortController() } : null }) as unknown as Store

const hex = (c: RGBA) => "#" + c.toInts().slice(0, 3).map((n) => n.toString(16).padStart(2, "0")).join("")

async function render(busy: boolean) {
  const s = await testRender(<TodoPanel store={fakeStore(busy)} />, { width: 60, height: 10 })
  await act(async () => void (await s.renderOnce()))
  return s
}

/** The spans on the row containing `text`. */
function row(spans: { lines: { spans: CapturedSpan[] }[] }, text: string): CapturedSpan[] {
  return spans.lines.find((l) => l.spans.map((x) => x.text).join("").includes(text))!.spans
}

test("completed tasks: green ☑, grey strikethrough text", async () => {
  const s = await render(false)
  const r = row(s.captureSpans(), "Inspect the repo")
  const box = r.find((x) => x.text.includes("☑"))!
  expect(hex(box.fg)).toBe(t.green)
  const label = r.find((x) => x.text.includes("Inspect"))!
  expect(hex(label.fg)).toBe(t.faint)
  expect(label.attributes & TextAttributes.STRIKETHROUGH).toBeTruthy()
  expect(box.attributes & TextAttributes.STRIKETHROUGH).toBeFalsy()
  expect(s.captureCharFrame()).not.toContain("☒")
  s.renderer.destroy()
})

test("the current task spins while working and rests as ◐ when idle", async () => {
  const idle = await render(false)
  expect(idle.captureCharFrame()).toContain("◐ Run final verification")
  idle.renderer.destroy()

  const s = await render(true)
  const glyph = () => s.captureCharFrame().split("\n").find((l) => l.includes("Run final verification"))!.trim()[2]
  const seen = new Set<string>()
  for (let i = 0; i < 6; i++) {
    seen.add(glyph()!)
    await act(async () => {
      await new Promise((r) => setTimeout(r, 90))
      await s.renderOnce()
    })
  }
  expect(seen.size).toBeGreaterThan(1)
  for (const g of seen) expect(SPINNER).toContain(g)
  s.renderer.destroy()
})
