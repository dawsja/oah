import { beforeEach, describe, expect, it, vi } from "vitest"
import { ipc } from "@/lib/ipc"
import type { Limits, Location, Settings } from "@/lib/types"
import { LOCAL_USAGE, nextReset, resetsIn, shortLabel, shownAgents, usageLocationKey, useUsage } from "./usage"

vi.mock("@/lib/ipc", () => ({ ipc: { providerLimits: vi.fn() } }))

const w = (label: string, resetsAt: number | null = null) => ({ label, usedPercent: 10, resetsAt })

describe("usage labels", () => {
  it("keeps a model group's name on its windows (Antigravity)", () => {
    expect(shortLabel(w("Gemini · 5 hours"))).toBe("Gemini 5h")
    expect(shortLabel(w("Claude and GPT · this week"))).toBe("Claude and GPT wk")
  })

  it("shortens window names", () => {
    expect(shortLabel(w("Current session (5 hours)"))).toBe("5h")
    expect(shortLabel(w("This week, all models"))).toBe("wk")
    expect(shortLabel(w("This week, Fable"))).toBe("Fable")
    expect(shortLabel(w("Weekly window"))).toBe("wk")
    expect(shortLabel(w("This month"))).toBe("30d")
    expect(shortLabel(w("2 day window"))).toBe("2d")
  })
  it("says when the next reset is", () => {
    const now = 1_000_000_000_000
    expect(resetsIn(now + (2 * 1440 + 17 * 60) * 60_000, now)).toBe("in 2d 17h")
    expect(resetsIn(now + (4 * 60 + 47) * 60_000, now)).toBe("in 4h 47m")
    expect(resetsIn(now + 12 * 60_000, now)).toBe("in 12m")
    expect(resetsIn(null)).toBeNull()
    expect(nextReset({ windows: [w("a", 5), w("b", 3), w("c")], error: null, extra: [] })).toBe(3)
  })
})

describe("usage locations", () => {
  const ubuntu: Location = { kind: "wsl", distro: "Ubuntu" }
  const ssh: Location = { kind: "ssh", host: "build" }
  const limits = (used: number): Limits => ({ windows: [{ ...w("Weekly window"), usedPercent: used }], error: null, extra: [] })
  beforeEach(() => {
    vi.mocked(ipc.providerLimits).mockReset()
    useUsage.setState({ entries: {} })
  })

  it("queries WSL directly and keeps local and SSH usage separate", async () => {
    vi.mocked(ipc.providerLimits).mockImplementation(async (_agent, location) => limits(location?.kind === "wsl" ? 30 : location?.kind === "ssh" ? 60 : 10))
    for (const location of [LOCAL_USAGE, ubuntu, ssh]) await useUsage.getState().load(location)
    expect(ipc.providerLimits).toHaveBeenCalledWith("claude", ubuntu)
    expect(ipc.providerLimits).toHaveBeenCalledWith("opencode", ubuntu)
    expect(ipc.providerLimits).toHaveBeenCalledWith("antigravity", ubuntu)
    expect(ipc.providerLimits).toHaveBeenCalledWith("grok", ubuntu)
    const entries = useUsage.getState().entries
    expect(entries[usageLocationKey(ubuntu)].limits.claude?.windows[0].usedPercent).toBe(30)
    expect(entries[usageLocationKey(LOCAL_USAGE)].limits.claude?.windows[0].usedPercent).toBe(10)
    expect(entries[usageLocationKey(ssh)].limits.claude?.windows[0].usedPercent).toBe(60)
    await useUsage.getState().load(ubuntu)
    expect(ipc.providerLimits).toHaveBeenCalledTimes(15)
    await useUsage.getState().load(ubuntu, true)
    expect(ipc.providerLimits).toHaveBeenCalledTimes(20)
  })

  it("asks again for an agent that failed (signed out, no key), but reuses ones that answered", async () => {
    let signedIn = false
    vi.mocked(ipc.providerLimits).mockImplementation(async (agent) =>
      agent === "opencode" && !signedIn ? { windows: [], error: "No OpenCode Go key found for this location.", extra: [] } : limits(40),
    )
    await useUsage.getState().load(ubuntu)
    expect(ipc.providerLimits).toHaveBeenCalledTimes(5)
    signedIn = true
    await useUsage.getState().load(ubuntu)
    expect(ipc.providerLimits).toHaveBeenCalledTimes(6)
    expect(ipc.providerLimits).toHaveBeenLastCalledWith("opencode", ubuntu)
    expect(useUsage.getState().entries[usageLocationKey(ubuntu)].limits.opencode?.error).toBeNull()
    await useUsage.getState().load(ubuntu)
    expect(ipc.providerLimits).toHaveBeenCalledTimes(6)
  })

  it("a thrown request counts as a failure and is asked again", async () => {
    vi.mocked(ipc.providerLimits).mockImplementation(async (agent) => {
      if (agent === "codex") throw new Error("Codex didn't start")
      return limits(5)
    })
    await useUsage.getState().load(LOCAL_USAGE)
    expect(useUsage.getState().entries.local.limits.codex?.error).toContain("Codex didn't start")
    await useUsage.getState().load(LOCAL_USAGE)
    expect(ipc.providerLimits).toHaveBeenCalledTimes(6)
    expect(ipc.providerLimits).toHaveBeenLastCalledWith("codex", LOCAL_USAGE)
  })

  it("does not let slow local failures overwrite WSL results after switching", async () => {
    let finish!: (result: Limits) => void
    const pending = new Promise<Limits>((resolve) => { finish = resolve })
    vi.mocked(ipc.providerLimits).mockImplementation((_agent, location) => location?.kind === "local" ? pending : Promise.resolve(limits(25)))
    const localLoad = useUsage.getState().load(LOCAL_USAGE)
    await useUsage.getState().load(ubuntu)
    finish({ windows: [], error: "Local credential rejected", extra: [] })
    await localLoad
    expect(useUsage.getState().entries[usageLocationKey(ubuntu)].limits.claude?.error).toBeNull()
    expect(useUsage.getState().entries[usageLocationKey(ubuntu)].limits.claude?.windows[0].usedPercent).toBe(25)
    expect(useUsage.getState().entries.local.limits.claude?.error).toBe("Local credential rejected")
  })
})

describe("shownAgents", () => {
  const settings = (codexOn: boolean) =>
    ({ providers: { claude: { enabled: true }, codex: { enabled: codexOn }, opencode: { enabled: true }, antigravity: { enabled: false }, grok: { enabled: false } } }) as unknown as Settings
  const none = { windows: [], error: null, extra: [] }

  it("leaves out agents not installed there, and ones turned off", () => {
    expect(shownAgents(settings(true), { claude: none, codex: { ...none, missing: true }, opencode: { ...none, missing: true } })).toEqual(["claude"])
    expect(shownAgents(settings(false), {})).toEqual(["claude", "opencode"])
    // OpenCode 2 alone still shows OpenCode Go's limits (one account).
    const only2 = { providers: { claude: { enabled: true }, codex: { enabled: true }, opencode: { enabled: false }, opencode2: { enabled: true }, antigravity: { enabled: true }, grok: { enabled: true } } } as unknown as Settings
    expect(shownAgents(only2, {})).toEqual(["claude", "codex", "opencode", "antigravity", "grok"])
    // Not loaded yet: shown (loading), until the app says they're missing.
    expect(shownAgents(settings(true), {})).toEqual(["claude", "codex", "opencode"])
  })
})
