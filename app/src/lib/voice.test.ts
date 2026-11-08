import { afterEach, describe, expect, it, vi } from "vitest"
import { comboOf, matchesKey, record, resample, toBase64 } from "./voice"

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

describe("microphone cleanup", () => {
  afterEach(() => vi.unstubAllGlobals())

  it("releases the microphone if the audio context cannot start", async () => {
    const stop = vi.fn()
    vi.stubGlobal("navigator", { mediaDevices: { getUserMedia: async () => ({ getTracks: () => [{ stop }] }) } })
    vi.stubGlobal("AudioContext", class { constructor() { throw new Error("audio unavailable") } })
    await expect(record()).rejects.toThrow("audio unavailable")
    expect(stop).toHaveBeenCalledOnce()
  })

  it("closes the context and tracks if audio graph setup fails", async () => {
    const stop = vi.fn()
    const close = vi.fn(async () => {})
    vi.stubGlobal("navigator", { mediaDevices: { getUserMedia: async () => ({ getTracks: () => [{ stop }] }) } })
    vi.stubGlobal("AudioContext", class {
      close = close
      createMediaStreamSource() { throw new Error("device removed") }
    })
    await expect(record()).rejects.toThrow("device removed")
    expect(stop).toHaveBeenCalledOnce()
    expect(close).toHaveBeenCalledOnce()
  })
})
