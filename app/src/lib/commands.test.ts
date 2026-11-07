import { describe, expect, it } from "vitest"
import { filterCommands, menuCommands, ownCommand, parseSlash, slashQuery } from "./commands"

const agent = [
  { name: "compact", description: "theirs", hint: "", skill: false },
  { name: "review", description: "Review", hint: "", skill: false },
  { name: "deploy", description: "Ship", hint: "", skill: false },
  { name: "pdf", description: "PDF skill", hint: "", skill: true },
]

describe("slash commands", () => {
  it("puts oah's first and drops the agent's same-named ones", () => {
    const names = menuCommands("claude", agent, true).map((c) => c.name)
    expect(names).toEqual(["model", "plan", "compact", "clear", "review", "deploy", "pdf"])
    expect(menuCommands("claude", agent, true).find((c) => c.name === "compact")?.own).toBe("compact")
  })

  it("leaves chat-only commands out of a new chat and falls back before the agent lists any", () => {
    const names = menuCommands("claude", [], false).map((c) => c.name)
    expect(names).toEqual(["model", "plan", "init", "review"])
  })

  it("opens only at the start of the prompt", () => {
    expect(slashQuery("/", 1)).toBe("")
    expect(slashQuery("/rev", 4)).toBe("rev")
    expect(slashQuery("/review now", 11)).toBeNull()
    expect(slashQuery("see /rev", 8)).toBeNull()
  })

  it("matches prefixes first, skills last", () => {
    const all = menuCommands("claude", agent, true)
    expect(filterCommands(all, "p").map((c) => c.name)).toEqual(["plan", "pdf"])
    expect(filterCommands([...all, { name: "code-review", description: "", hint: "", skill: false }], "rev").map((c) => c.name)).toEqual(["review", "code-review"])
    expect(filterCommands(all, "").length).toBe(all.length)
  })

  it("parses commands and finds oah's own", () => {
    expect(parseSlash("/compact keep the API notes")).toEqual({ name: "compact", args: "keep the API notes" })
    expect(parseSlash("/ a")).toBeNull()
    expect(parseSlash("a/b")).toBeNull()
    expect(ownCommand("/model opus", false)).toEqual({ own: "model", args: "opus" })
    expect(ownCommand("/clear", false)).toBeNull()
    expect(ownCommand("/clear", true)).toEqual({ own: "clear", args: "" })
    expect(ownCommand("/review", true)).toBeNull()
  })
})

import { emptyTranscript, reduce } from "./chat"

describe("old chat logs", () => {
  it("replays model lists saved before effort levels", () => {
    const old = { type: "models", models: [{ id: "haiku", name: "Haiku", description: "" }], current: "haiku" } as unknown as Parameters<typeof reduce>[1]
    const t = reduce(emptyTranscript(), old)
    expect(t.models[0].efforts).toEqual([])
    expect(t.models[0].defaultEffort).toBeNull()
  })
})
