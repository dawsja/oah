import { expect, test } from "bun:test"
import { RGBA, SyntaxStyle } from "@opentui/core"
import { testRender } from "@opentui/react/test-utils"
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

test("renders every entry kind", async () => {
  const setup = await testRender(<box />, { width: 100, height: 60 })
  const style = SyntaxStyle.fromStyles({ default: { fg: RGBA.fromHex("#ffffff") } }, (setup.renderer as any).nativeScene)
  setup.renderer.destroy()
  const s2 = await testRender(<Transcript entries={entries} style={style} liveText={null} busy={false} />, { width: 100, height: 60 })
  await s2.renderOnce()
  const frame = s2.captureCharFrame()
  expect(frame).toMatch(/\d steps? · Update, Edit, Bash/)
  expect(frame).toMatch(/› add a --json flag/)
  s2.renderer.destroy()
})
