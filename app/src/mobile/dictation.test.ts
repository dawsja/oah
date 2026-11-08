import { describe, expect, it, vi } from "vitest"
import { PhoneDictation, MAX_SECONDS } from "./dictation"

function pending<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((r) => { resolve = r })
  return { promise, resolve }
}
function fixture() {
  const recording = { cancel: vi.fn(), stop: vi.fn(() => new Float32Array(16_000)), level: () => 0 }
  const changed = vi.fn(), text = vi.fn(), failed = vi.fn()
  const capture = vi.fn(async () => recording)
  const transcribe = vi.fn(async (_pcm: Float32Array) => "Hello")
  return { recording, changed, text, failed, capture, transcribe,
    session: new PhoneDictation(changed, text, failed, capture, transcribe) }
}

describe("phone dictation", () => {
  it("releases a microphone grant that arrives after cancellation", async () => {
    const f = fixture(), grant = pending<typeof f.recording>()
    f.capture.mockReturnValue(grant.promise)
    const started = f.session.start()
    f.session.cancel()
    grant.resolve(f.recording)
    await started
    expect(f.recording.cancel).toHaveBeenCalledOnce()
    expect(f.session.phase).toBe("idle")
    expect(f.transcribe).not.toHaveBeenCalled()
  })
  it("does not insert a transcript into a different chat", async () => {
    const f = fixture(), result = pending<string>()
    f.transcribe.mockReturnValue(result.promise)
    await f.session.start()
    const stopped = f.session.stop()
    f.session.cancel(false)
    result.resolve("Old chat")
    await stopped
    expect(f.text).not.toHaveBeenCalled()
  })
  it("ignores repeated taps while requesting the microphone or transcribing", async () => {
    const f = fixture(), result = pending<string>()
    f.transcribe.mockReturnValue(result.promise)
    await Promise.all([f.session.start(), f.session.start()])
    expect(f.capture).toHaveBeenCalledOnce()
    const stopped = f.session.stop()
    await f.session.start()
    await f.session.stop()
    expect(f.capture).toHaveBeenCalledOnce()
    expect(f.transcribe).toHaveBeenCalledOnce()
    result.resolve("Hello")
    await stopped
    expect(f.text).toHaveBeenCalledExactlyOnceWith("Hello")
    expect(f.session.phase).toBe("idle")
  })
  it("bounds audio sent to native inference even if the stop timer is late", async () => {
    const f = fixture()
    f.recording.stop.mockReturnValue(new Float32Array((MAX_SECONDS + 5) * 16_000))
    await f.session.start()
    await f.session.stop()
    expect(f.transcribe.mock.calls[0][0].length).toBe(MAX_SECONDS * 16_000)
  })
  it("recovers from microphone and transcription errors", async () => {
    const f = fixture()
    f.capture.mockRejectedValueOnce(new Error("permission denied"))
    await f.session.start()
    expect(f.session.phase).toBe("idle")
    f.transcribe.mockRejectedValueOnce(new Error("model unavailable"))
    await f.session.start()
    await f.session.stop()
    expect(f.failed).toHaveBeenCalledTimes(2)
    expect(f.session.phase).toBe("idle")
    expect(f.text).not.toHaveBeenCalled()
  })
})
