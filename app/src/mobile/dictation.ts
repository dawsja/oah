import { record, voice, type Recording } from "@/lib/voice"

export type Phase = "idle" | "starting" | "recording" | "working"
export const MAX_SECONDS = 30

/** Owns async mic access and transcription so late results cannot cross chat boundaries. */
export class PhoneDictation {
  phase: Phase = "idle"
  private recording: Recording | null = null
  private generation = 0

  constructor(
    private changed: (phase: Phase) => void,
    private text: (text: string) => void,
    private failed: (error: unknown) => void,
    private capture = record,
    private transcribe = voice.transcribe,
  ) {}

  private set(phase: Phase) { this.phase = phase; this.changed(phase) }

  cancel(notify = true) {
    this.generation++
    this.recording?.cancel()
    this.recording = null
    this.phase = "idle"
    if (notify) this.changed("idle")
  }

  async start() {
    if (this.phase !== "idle") return
    const id = ++this.generation
    this.set("starting")
    try {
      const recording = await this.capture()
      if (id !== this.generation) { recording.cancel(); return }
      this.recording = recording
      this.set("recording")
    } catch (e) {
      if (id !== this.generation) return
      this.set("idle")
      this.failed(e)
    }
  }

  async stop() {
    const recording = this.recording
    if (!recording) return
    this.recording = null
    const id = this.generation
    this.set("working")
    try {
      const pcm = recording.stop().subarray(0, MAX_SECONDS * 16_000)
      if (pcm.length < 1600) return
      const text = await this.transcribe(pcm)
      if (id === this.generation) this.text(text)
    } catch (e) {
      if (id === this.generation) this.failed(e)
    } finally {
      if (id === this.generation) this.set("idle")
    }
  }
}
