import { describe, expect, it } from "vitest"
import { blocks, emptyTranscript, reduce, replay, type ChatEvent } from "./chat"

const run = (events: ChatEvent[]) => events.reduce(reduce, emptyTranscript())

describe("reduce", () => {
  it("streams text into one item and replaces it with the final text", () => {
    const t = run([
      { type: "userMessage", id: "u1", text: "hi", images: [] },
      { type: "textDelta", itemId: "a", text: "Hel" },
      { type: "textDelta", itemId: "a", text: "lo" },
    ])
    expect(t.running).toBe(true)
    expect(t.items.at(-1)).toMatchObject({ kind: "assistant", text: "Hello" })
    expect(reduce(t, { type: "text", itemId: "a", text: "Hello!" }).items.at(-1)).toMatchObject({ text: "Hello!" })
  })

  it("tracks tools, approvals and the end of a turn", () => {
    const tool = { kind: "edit" as const, name: "Edit", title: "a.rs", input: {}, path: "a.rs" }
    let t = run([
      { type: "turnStarted", turnId: "t1" },
      { type: "toolStarted", itemId: "x", tool },
      { type: "approval", request: { requestId: "r", itemId: "x", kind: "edit", title: "Edit a.rs", detail: null, diff: null, canRemember: true } },
    ])
    expect(t.pending?.requestId).toBe("r")
    t = reduce(t, { type: "approvalResolved", requestId: "r", decision: "allow" })
    expect(t.pending).toBeNull()
    expect(t.items.find((i) => i.kind === "approval")).toMatchObject({ decision: "allow" })
    t = reduce(t, { type: "toolStarted", itemId: "y", tool: { ...tool, kind: "bash", title: "sleep 9" } })
    t = reduce(t, { type: "turnFinished", turnId: "t1", usage: null, costUsd: null, durationMs: 10, interrupted: true })
    expect(t.running).toBe(false)
    expect(t.items.find((i) => i.id === "y")).toMatchObject({ status: "error" })
    expect(t.items.at(-1)).toMatchObject({ kind: "turnEnd", interrupted: true })
  })

  it("replays a log as not running", () => {
    const t = replay([
      { type: "userMessage", id: "u1", text: "go", images: [] },
      { type: "approval", request: { requestId: "r", itemId: null, kind: "command", title: "Run", detail: "ls", diff: null, canRemember: false } },
    ])
    expect(t.running).toBe(false)
    expect(t.pending).toBeNull()
  })
})

describe("blocks", () => {
  it("groups runs of reasoning and tools", () => {
    const t = run([
      { type: "userMessage", id: "u", text: "x", images: [] },
      { type: "reasoningDelta", itemId: "r", text: "think" },
      { type: "toolStarted", itemId: "a", tool: { kind: "read", name: "Read", title: "f", input: {}, path: "f" } },
      { type: "textDelta", itemId: "m", text: "done" },
      { type: "toolStarted", itemId: "b", tool: { kind: "bash", name: "Bash", title: "ls", input: {}, path: null } },
    ])
    expect(blocks(t.items).map((b) => (b.kind === "work" ? `work:${b.items.length}` : b.item.kind))).toEqual(["user", "work:2", "assistant", "work:1"])
  })
})

