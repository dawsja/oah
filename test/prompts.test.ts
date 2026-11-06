import { describe, expect, test } from "bun:test"
import { mkdtempSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"

// Keep the store's files (settings, sessions, memory) out of the real config.
process.env.OAH_HOME = mkdtempSync(join(tmpdir(), "oah-test-"))

const { loadSettings } = await import("../src/config")
const { Mcp } = await import("../src/mcp")
const { Memory } = await import("../src/memory")
const { newSession } = await import("../src/session")
const { Skills } = await import("../src/skills")
const { Store } = await import("../src/ui/store")

function store() {
  const cwd = mkdtempSync(join(tmpdir(), "oah-cwd-"))
  const settings = loadSettings()
  return new Store(settings, newSession(settings.model, settings.effort, settings.mode), cwd, new Skills(cwd), new Memory(cwd), new Mcp(cwd, () => {}), () => false)
}

const req = (title: string) => ({ title, kind: "run command", preview: [] })

describe("approval prompts from parallel tool calls", () => {
  test("queue instead of replacing each other", async () => {
    const s = store()
    const a = s.requestPermission(req("A"))
    const b = s.requestPermission(req("B"))
    const c = s.requestPermission(req("C"))
    expect(s.permission?.title).toBe("A")
    expect(s.waitingPrompts).toBe(2)
    s.answerPermission("once")
    expect(s.permission?.title).toBe("B")
    s.answerPermission("deny")
    expect(s.permission?.title).toBe("C")
    s.answerPermission("always")
    expect(s.permission).toBeNull()
    expect(await Promise.all([a, b, c])).toEqual(["once", "deny", "always"])
  })

  test("questions queue too", async () => {
    const s = store()
    const q = (question: string) => ({ question, options: [], multiSelect: false })
    const a = s.requestAnswer(q("first?"))
    const b = s.requestAnswer(q("second?"))
    expect(s.question?.question).toBe("first?")
    s.answerQuestion("1")
    expect(s.question?.question).toBe("second?")
    s.answerQuestion("2")
    expect(await Promise.all([a, b])).toEqual(["1", "2"])
  })

  test("shutdown denies every waiting prompt", async () => {
    const s = store()
    const all = [s.requestPermission(req("A")), s.requestPermission(req("B"))]
    s.shutdown()
    expect(await Promise.all(all)).toEqual(["deny", "deny"])
  })
})
