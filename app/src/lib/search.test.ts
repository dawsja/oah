import { describe, expect, it } from "vitest"
import type { ChatMeta } from "./chat"
import { termsOf, titleMatches } from "./search"

const meta = (id: string, title: string, projectId = "p"): ChatMeta =>
  ({ id, projectId, agent: "claude", effort: "", acpAgent: null, model: "", title, cwd: "", agentSessionId: null, createdAt: 0, updatedAt: 0, tokens: 0, costUsd: 0, pinned: false }) as ChatMeta

describe("titleMatches", () => {
  const metas = [meta("a", "Fix the retry logic"), meta("b", "Retry backoff in the client"), meta("c", "Login page", "q"), meta("d", "")]
  const names = (id: string) => (id === "q" ? "web app" : "server")

  it("needs every word, in any case", () => {
    expect(titleMatches(metas, names, "RETRY").map((m) => m.id)).toEqual(["b", "a"])
    expect(titleMatches(metas, names, "retry logic").map((m) => m.id)).toEqual(["a"])
    expect(titleMatches(metas, names, "retry zebra")).toEqual([])
  })

  it("matches the project's name and untitled chats", () => {
    expect(titleMatches(metas, names, "web login").map((m) => m.id)).toEqual(["c"])
    expect(titleMatches(metas, names, "new chat").map((m) => m.id)).toEqual(["d"])
  })

  it("matches nothing for a blank query", () => {
    expect(titleMatches(metas, names, "   ")).toEqual([])
    expect(termsOf("  A  b ")).toEqual(["a", "b"])
  })
})
