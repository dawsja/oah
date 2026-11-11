import { describe, expect, it } from "vitest"
import { listed } from "./provider-list"

const facts = (installed: boolean | null, isDefault = false, inUse = false) => ({ installed, isDefault, inUse })

describe("which providers are listed", () => {
  it("lists what's installed and hides what isn't", () => {
    expect(listed(undefined, facts(true))).toBe(true)
    expect(listed(undefined, facts(false))).toBe(false)
  })

  it("keeps the default agent and agents with chats, installed here or not", () => {
    expect(listed(undefined, facts(false, true))).toBe(true)
    expect(listed(undefined, facts(false, false, true))).toBe(true)
  })

  it("lists nothing it hasn't looked for yet, unless something says it's used", () => {
    expect(listed(undefined, facts(null))).toBe(false)
    expect(listed(undefined, facts(null, true))).toBe(true)
  })

  it("follows the user's choice over everything", () => {
    expect(listed("shown", facts(false))).toBe(true)
    expect(listed("hidden", facts(true, true, true))).toBe(false)
  })
})
