import { describe, expect, it } from "vitest"
import { matchesKey, resample, toBase64 } from "./voice"

describe("voice helpers", () => {
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
