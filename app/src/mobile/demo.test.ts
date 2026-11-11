// The demo desktop's script (phone/src-tauri/demo/demo.json, served by demo.rs) is real chat
// events: it has to fold into the screens it's there to show.

import { readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"
import { describe, expect, it } from "vitest"
import { parsePatch } from "@/components/chat/DiffView"
import { blocks, reduce, replay, waitingFor, type ChatEvent, type ChatMeta, type ToolItem } from "@/lib/chat"
import { AGENT_LABEL, type Project } from "@/lib/types"

type Demo = {
  projects: Project[]
  agents: { agents: { kind: keyof typeof AGENT_LABEL }[]; defaultAgent: string }
  models: Record<string, { id: string }[]>
  chats: { meta: ChatMeta; waiting: string | null; events: ChatEvent[] }[]
  afterAllow: ChatEvent[]
  afterDeny: ChatEvent[]
  afterAnswer: string
  reply: string
}
const demo = JSON.parse(readFileSync(fileURLToPath(new URL("../../../phone/src-tauri/demo/demo.json", import.meta.url)), "utf8")) as Demo
const chat = (id: string) => demo.chats.find((c) => c.meta.id === id)!
const open = (id: string) => replay(chat(id).events, chat(id).waiting !== null)

describe("the demo desktop's script", () => {
  it("has chats in its projects, run by its agents and models", () => {
    const agents = demo.agents.agents.map((a) => a.kind)
    for (const { meta } of demo.chats) {
      expect(demo.projects.map((p) => p.id)).toContain(meta.projectId)
      expect(agents).toContain(meta.agent)
      expect(demo.models[meta.agent].map((m) => m.id)).toContain(meta.model)
      expect(AGENT_LABEL[meta.agent]).toBeTruthy()
    }
    expect(new Set(demo.chats.map((c) => c.meta.id)).size).toBe(demo.chats.length)
  })

  it("waits where it says it does", () => {
    for (const c of demo.chats) expect(waitingFor(open(c.meta.id)), c.meta.id).toBe(c.waiting)
  })

  it("shows every kind of row", () => {
    const kinds = new Set(demo.chats.flatMap((c) => open(c.meta.id).items.map((i) => i.kind)))
    for (const kind of ["user", "assistant", "reasoning", "tool", "approval", "question", "turnEnd"]) expect(kinds, kind).toContain(kind)
    for (const c of demo.chats) expect(blocks(open(c.meta.id).items).length, c.meta.id).toBeGreaterThan(1)
    expect(open("c-rename").plan).toHaveLength(3)
    expect(open("c-checkout").context).toEqual({ used: 41200, window: 200000, parts: [] })
  })

  it("has diffs that add up", () => {
    const diffs = demo.chats.flatMap((c) => open(c.meta.id).items).flatMap((i) => (i.kind === "tool" && (i as ToolItem).diff ? [(i as ToolItem).diff!] : []))
    expect(diffs).toHaveLength(2)
    for (const d of diffs) {
      const lines = parsePatch(d.patch)
      expect(lines.filter((l) => l.cls === "a"), d.path).toHaveLength(d.added)
      expect(lines.filter((l) => l.cls === "d"), d.path).toHaveLength(d.removed)
    }
  })

  it("carries on after the approval is answered, either way", () => {
    const approval = open("c-checkout").approvals[0]
    for (const [decision, after] of [["allow", demo.afterAllow], ["deny", demo.afterDeny]] as const) {
      const resolved: ChatEvent = { type: "approvalResolved", requestId: approval.requestId, decision }
      const t = [resolved, ...after].reduce(reduce, open("c-checkout"))
      expect(t.approvals).toHaveLength(0)
      expect(t.items.at(-1)?.kind).toBe("assistant")
    }
    expect(demo.afterAllow.some((e) => e.type === "toolStarted" && e.itemId === approval.itemId)).toBe(true)
  })

  it("says what it is when written to", () => {
    expect(demo.reply).toMatch(/demo/)
    expect(demo.afterAnswer.length).toBeGreaterThan(10)
  })
})
