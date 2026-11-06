import { expect, test } from "bun:test"
import { RGBA, SyntaxStyle } from "@opentui/core"
import { useRenderer } from "@opentui/react"
import { testRender } from "@opentui/react/test-utils"
import { useMemo } from "react"
import { Transcript } from "../src/ui/Transcript"
import type { Entry } from "../src/session"

const entries: Entry[] = [
  { kind: "user", text: "add a --json flag to the export command" },
  { kind: "reasoning", text: "Looking at how flags are parsed.", started: 0, ms: 1200 },
  { kind: "tool", callId: "1", name: "todo_write", title: "Update todos", summary: "0/3 done", detail: ["◐ Parse", "☐ Emit"], status: "ok", started: 0, ms: 5 },
  { kind: "tool", callId: "2", name: "edit_file", title: "Edit src/cli.rs", summary: "+1 -1", detail: ["-a", "+b"], status: "ok", started: 0, ms: 1500 },
  { kind: "tool", callId: "3", name: "bash", title: "Bash cargo test", summary: "exit 0 · 3 lines", detail: ["   Compiling export v0.3.1", "    Finished test profile in 2.41s", "test result: ok. 48 passed"], status: "ok", started: 0, ms: 0 },
  { kind: "tool", callId: "4", name: "bash", title: "Bash cargo test", summary: "", detail: [], status: "running", started: 0 },
  { kind: "assistant", text: "Added a `--json` flag:\n\n- `Args` gains a field\n\n`cargo test`: **48 passed**.", model: "openai/gpt-5.6" },
  { kind: "info", text: "◎ **Goal complete** — done" },
  { kind: "error", text: "boom" },
]

// The style must belong to the renderer that draws it (markdown reads it synchronously).
function Styled() {
  const renderer = useRenderer()
  const style = useMemo(() => SyntaxStyle.fromStyles({ default: { fg: RGBA.fromHex("#ffffff") } }, (renderer as any).nativeScene), [renderer])
  return <Transcript entries={entries} style={style} liveText={null} busy={false} />
}

test("renders every entry kind", async () => {
  const s2 = await testRender(<Styled />, { width: 100, height: 60 })
  await s2.renderOnce()
  const frame = s2.captureCharFrame()
  expect(frame).toMatch(/\d steps? · Update, Edit, Bash/)
  expect(frame).toMatch(/› add a --json flag/)
  s2.renderer.destroy()
})

// Regression: while streaming, the instant preview used to be inline-only ("## Steps"),
// then tree-sitter swapped in the concealed form ("Steps") — every delta flickered.
test("streaming preview conceals block markers", async () => {
  const live: Entry[] = [
    { kind: "user", text: "plan it" },
    { kind: "assistant", text: "Here is the plan.\n\n## Steps\n\n1. Read **store.ts**\n2. Ch", model: "openai/gpt-5.6" },
  ]
  function Live() {
    const renderer = useRenderer()
    const style = useMemo(() => SyntaxStyle.fromStyles({ default: { fg: RGBA.fromHex("#ffffff") } }, (renderer as any).nativeScene), [renderer])
    return <Transcript entries={live} style={style} liveText={1} busy />
  }
  const s = await testRender(<Live />, { width: 80, height: 30 })
  await s.renderOnce()
  const frame = s.captureCharFrame()
  expect(frame).toMatch(/Steps/)
  expect(frame).not.toMatch(/## Steps/)
  expect(frame).not.toMatch(/\*\*store\.ts\*\*/)
  s.renderer.destroy()
})
