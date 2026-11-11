import { describe, expect, it } from "vitest"
import { closePane, draftSwaps, newChatKey, paneOf, patchPane, splitPanes, type Pane, type Panes } from "./panes"

const pane = (id: string, p: Partial<Pane> = {}): Pane => ({ id, projectId: "A", chatId: null, draftCwd: null, viewer: null, ...p })
const one = (p: Partial<Pane> = {}): Panes => ({ panes: [pane("p1", p)], activePane: "p1" })

describe("panes", () => {
  it("finds a pane, falling back to the active one", () => {
    const s: Panes = { panes: [pane("p1"), pane("p2", { chatId: "c" })], activePane: "p2" }
    expect(paneOf(s).id).toBe("p2")
    expect(paneOf(s, "p1").id).toBe("p1")
    expect(paneOf(s, "closed").id).toBe("p2")
  })

  it("starts another project clean, and keeps the worktree within one", () => {
    const s = one({ chatId: "c1", draftCwd: "/wt", viewer: { kind: "file", path: "a.ts" } })
    expect(patchPane(s, { projectId: "B" }).panes[0]).toEqual(pane("p1", { projectId: "B" }))
    expect(patchPane(s, { projectId: "A", chatId: "c2" }).panes[0]).toMatchObject({ chatId: "c2", draftCwd: "/wt" })
    expect(patchPane(s, { projectId: "B", chatId: "c9" }).panes[0]).toMatchObject({ projectId: "B", chatId: "c9", draftCwd: null, viewer: null })
  })

  it("changes only the pane asked for", () => {
    const s: Panes = { panes: [pane("p1"), pane("p2")], activePane: "p1" }
    const next = patchPane(s, { chatId: "c" }, "p2")
    expect(next.panes.map((p) => p.chatId)).toEqual([null, "c"])
    expect(next.activePane).toBe("p1")
  })

  it("splits to the right, then reuses the other pane", () => {
    const two = splitPanes(one({ chatId: "c1" }), "p2")
    expect(two.panes).toEqual([pane("p1", { chatId: "c1" }), pane("p2")])
    expect(two.activePane).toBe("p2")
    // From the right pane, "to the side" is the left one: never a third.
    const again = splitPanes(two, "p3", { projectId: "B", chatId: "c7" })
    expect(again.panes).toEqual([pane("p1", { projectId: "B", chatId: "c7" }), pane("p2")])
    expect(again.activePane).toBe("p1")
  })

  it("closes a pane, never the last, and keeps the ids of the ones that stay", () => {
    const two = splitPanes(one({ chatId: "c1" }), "p2", { chatId: "c2" })
    const left = closePane(two, "p1")
    expect(left).toEqual({ panes: [pane("p2", { chatId: "c2" })], activePane: "p2" })
    expect(closePane(left)).toBe(left)
    expect(closePane({ ...two, activePane: "p1" }, "p2").activePane).toBe("p1")
  })
})

describe("new-chat drafts in two panes", () => {
  it("gives the second pane on the same new chat its own key", () => {
    const panes = [pane("p1"), pane("p2")]
    expect(newChatKey(panes, panes[0])).toBe("new:A:")
    expect(newChatKey(panes, panes[1])).toBe("new:A:#2")
    const apart = [pane("p1"), pane("p2", { draftCwd: "/wt" })]
    expect(newChatKey(apart, apart[1])).toBe("new:A:/wt")
    const chat = [pane("p1", { chatId: "c" }), pane("p2")]
    expect(newChatKey(chat, chat[1])).toBe("new:A:")
  })

  it("keeps a pane's text when the pane beside it closes", () => {
    const two = splitPanes(one(), "p2")
    expect(draftSwaps(two.panes, closePane(two, "p1").panes)).toEqual([["new:A:#2", "new:A:"]])
    expect(draftSwaps(two.panes, closePane(two, "p2").panes)).toEqual([])
  })

  it("keeps it when the other pane comes to, or leaves, the same new chat", () => {
    const apart: Panes = { panes: [pane("p1", { chatId: "c" }), pane("p2")], activePane: "p1" }
    const same = patchPane(apart, { chatId: null }, "p1")
    expect(draftSwaps(apart.panes, same.panes)).toEqual([["new:A:", "new:A:#2"]])
    expect(draftSwaps(same.panes, apart.panes)).toEqual([["new:A:#2", "new:A:"]])
    // A pane that itself moved on has nothing to keep.
    expect(draftSwaps(same.panes, patchPane(same, { projectId: "B" }, "p2").panes)).toEqual([])
  })
})
