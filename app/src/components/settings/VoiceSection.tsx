import { useEffect, useState } from "react"
import { Input } from "@/components/ui/input"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { useApp } from "@/store/app"
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

  return (
    <Section title="Voice" description="Dictate prompts. Speech is turned into text on this computer: nothing you say leaves it.">
      <Group>
        <Row
          label="Speech to text"
          description="Turning it on downloads NVIDIA Parakeet (0.6B, about 731 MB) once. Turning it off deletes it."
        >
          <Switch checked={voice.enabled} onCheckedChange={(v) => update((s) => void (s.voice.enabled = v))} aria-label="Speech to text" />
        </Row>
        <div className="flex items-center gap-3 px-4 py-2.5">
          <div className="h-1.5 flex-1 bg-muted">
            <div className="h-full bg-primary" style={{ width: voice.enabled ? "0%" : "0%" }} />
          </div>
          <span className="font-mono text-[11px] text-faint">{voice.enabled ? "model not downloaded" : "off"}</span>
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
