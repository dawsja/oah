import { describe, expect, it } from "vitest"
import { PromptHistory, historyBoundary } from "./prompt-history"

describe("prompt history", () => {
  it("crosses command-only entries and restores the unfinished draft", () => {
    const h = new PromptHistory()
    h.add("first prompt")
    h.add("/compact")
    expect(h.move(-1, "unfinished draft")).toBe("/compact")
    expect(h.move(-1, "/compact")).toBe("first prompt")
    expect(h.move(-1, "first prompt")).toBe("first prompt")
    expect(h.move(1, "first prompt")).toBe("/compact")
    expect(h.move(1, "/compact")).toBe("unfinished draft")
    expect(h.move(1, "unfinished draft")).toBeNull()
  })
  it("keeps a stable snapshot when more prompts arrive and resets after sending", () => {
    const h = new PromptHistory()
    expect(h.move(-1, "")).toBeNull()
    h.add("one")
    h.move(-1, "draft")
    h.add("two")
    expect(h.move(1, "one")).toBe("draft")
    expect(h.move(-1, "draft")).toBe("two")
    h.reset()
    expect(h.move(1, "")).toBeNull()
  })
  it("preserves multiline editing and selections", () => {
    expect(historyBoundary("one\ntwo", 2, 2, -1)).toBe(true)
    expect(historyBoundary("one\ntwo", 5, 5, -1)).toBe(false)
    expect(historyBoundary("one\ntwo", 2, 2, 1)).toBe(false)
    expect(historyBoundary("one\ntwo", 5, 5, 1)).toBe(true)
    expect(historyBoundary("one", 0, 3, -1)).toBe(false)
  })
})
