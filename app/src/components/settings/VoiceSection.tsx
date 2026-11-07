import { useEffect, useState } from "react"
import { Input } from "@/components/ui/input"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { useApp } from "@/store/app"
import { useVoice, voice as voiceApi } from "@/lib/voice"
import { toast } from "sonner"
import { confirmAction } from "@/components/Prompt"
import { Group, Row, Section } from "./layout"

/** Microphones the webview can see (labels appear once mic access was granted). */
function useMicrophones() {
  const [mics, setMics] = useState<MediaDeviceInfo[]>([])
  useEffect(() => {
    navigator.mediaDevices
      ?.enumerateDevices()
      .then((d) => setMics(d.filter((x) => x.kind === "audioinput" && x.deviceId !== "default")))
      .catch(() => setMics([]))
  }, [])
  return mics
}

export function VoiceSection() {
  const voice = useApp((s) => s.settings!.voice)
  const update = useApp((s) => s.update)
  const mics = useMicrophones()
  const state = useVoice((s) => s.state)
  const on = state.state === "ready" || state.state === "downloading"
  const toggle = async (v: boolean) => {
    // Voice is opt-in: nothing is downloaded until the user agrees to it here.
    if (v && state.state !== "ready") {
      const ok = await confirmAction({
        title: "Download the voice model?",
        description:
          "Speech to text needs NVIDIA Parakeet (0.6B, about 731 MB), downloaded once from Hugging Face. It runs on this computer: nothing you say leaves it. You can delete it any time by turning voice off.",
        confirm: "Download",
      })
      if (!ok) return
    }
    await (v ? voiceApi.enable() : voiceApi.disable()).catch((e) => toast.error(String(e)))
  }
  const label =
    state.state === "ready"
      ? "ready · runs on this computer"
      : state.state === "downloading"
        ? `downloading ${state.percent}%`
        : state.state === "error" || state.state === "unsupported"
          ? state.error
          : "off"
  const pct = state.state === "downloading" ? state.percent : state.state === "ready" ? 100 : 0

  return (
    <Section title="Voice" description="Dictate prompts. Speech is turned into text on this computer: nothing you say leaves it.">
      <Group>
        <Row
          label="Speech to text"
          description="Optional, and off until you turn it on. Turning it on asks before downloading NVIDIA Parakeet (0.6B, about 731 MB); turning it off deletes it."
        >
          <Switch checked={on} disabled={state.state === "unsupported"} onCheckedChange={toggle} aria-label="Speech to text" />
        </Row>
        <div className="flex items-center gap-3 px-4 py-2.5">
          <div className="flex h-2 flex-1 gap-px">
            {Array.from({ length: 40 }, (_, i) => (
              <span key={i} className={i < Math.round(pct / 2.5) ? "flex-1 bg-primary" : "flex-1 bg-muted"} />
            ))}
          </div>
          <span className={state.state === "error" || state.state === "unsupported" ? "max-w-[60%] truncate text-[11px] text-red" : "font-mono text-[11px] text-faint"}>{label}</span>
        </div>
      </Group>
      <Group title="Input">
        <Row label="Microphone">
          <Select value={voice.microphone || "default"} onValueChange={(v) => update((s) => void (s.voice.microphone = v === "default" ? "" : v))}>
            <SelectTrigger size="sm" className="w-64">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="default">System default</SelectItem>
              {mics.map((m, i) => (
                <SelectItem key={m.deviceId} value={m.deviceId}>
                  {m.label || `Microphone ${i + 1}`}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Row>
        <Row label="Hold to talk" description="Hold this key in the composer to record; let go to insert the text.">
          <Input className="h-7 w-40 text-center font-mono text-[12px]" defaultValue={voice.holdKey} onBlur={(e) => update((s) => void (s.voice.holdKey = e.target.value.trim() || "Ctrl+Space"))} />
        </Row>
        <Row label="Stop on silence" description="Seconds of silence before recording stops by itself. 0 keeps recording until you stop.">
          <Input
            type="number"
            min={0}
            max={30}
            className="h-7 w-20 text-right font-mono text-[12px]"
            defaultValue={voice.silenceStopSecs}
            onBlur={(e) => update((s) => void (s.voice.silenceStopSecs = Math.max(0, Math.min(30, Number(e.target.value) || 0))))}
          />
        </Row>
      </Group>
    </Section>
  )
}
