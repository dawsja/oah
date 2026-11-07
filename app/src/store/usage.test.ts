import { describe, expect, it } from "vitest"
import { nextReset, resetsIn, shortLabel } from "./usage"

const w = (label: string, resetsAt: number | null = null) => ({ label, usedPercent: 10, resetsAt })

describe("usage labels", () => {
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
