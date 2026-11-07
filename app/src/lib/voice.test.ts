import { describe, expect, it } from "vitest"
import { comboOf, matchesKey, resample, toBase64 } from "./voice"

describe("voice helpers", () => {
  it("records a pressed key as a combo matchesKey accepts", () => {
    const k = (key: string, mods: Partial<Record<"ctrlKey" | "altKey" | "shiftKey" | "metaKey", boolean>> = {}) => ({ key, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...mods })
    expect(comboOf(k(" ", { ctrlKey: true }))).toBe("Ctrl+Space")
    expect(comboOf(k("d", { altKey: true, shiftKey: true }))).toBe("Alt+Shift+D")
    expect(comboOf(k("F8"))).toBe("F8")
    expect(comboOf(k("Control", { ctrlKey: true }))).toBeNull()
    const combo = comboOf(k("d", { altKey: true, shiftKey: true }))!
    expect(matchesKey(k("D", { altKey: true, shiftKey: true }) as KeyboardEvent, combo)).toBe(true)
  })
  it("resamples to the target rate", () => {
    const out = resample(new Float32Array([0, 1, 0, -1]), 32_000, 16_000)
    expect(out.length).toBe(2)
    expect(out[0]).toBe(0)
  })
  it("encodes samples as little-endian f32", () => {
    expect(toBase64(new Float32Array([1]))).toBe("AACAPw==")
  })
  it("matches key combos", () => {
    const e = (key: string, ctrl = false, shift = false) => ({ key, ctrlKey: ctrl, altKey: false, shiftKey: shift, metaKey: false }) as KeyboardEvent
    expect(matchesKey(e(" ", true), "Ctrl+Space")).toBe(true)
    expect(matchesKey(e(" ", true, true), "Ctrl+Space")).toBe(false)
    expect(matchesKey(e("m", true, true), "Ctrl+Shift+M")).toBe(true)
  })
})
