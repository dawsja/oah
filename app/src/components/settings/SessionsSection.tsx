import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { useApp } from "@/store/app"
import { Group, Row, Section } from "./layout"

export function SessionsSection() {
  const minutes = useApp((s) => s.settings!.agentSleepMinutes ?? 0)
  const update = useApp((s) => s.update)
  return (
    <Section title="Sessions" description="Sleeping stops an agent’s process to free memory. Your conversation and saved session stay available; send a message to resume.">
      <Group>
        <Row label="Sleep idle agents automatically" description="Only finished, idle agents with resumable sessions sleep. Running tasks and agents waiting for your answer stay awake. Applies even when you are viewing the chat.">
          <Switch checked={minutes > 0} onCheckedChange={(on) => update((s) => void (s.agentSleepMinutes = on ? 30 : 0))} aria-label="Sleep idle agents automatically" />
        </Row>
        {minutes > 0 ? (
          <Row label="Sleep after" description="Minutes without agent activity after a turn finishes. Checked every 15 seconds.">
            <Input key={minutes} type="number" min={1} max={1440} defaultValue={minutes} aria-label="Idle minutes before sleep" className="h-7 w-20 text-right font-mono text-[12px]"
              onBlur={(e) => {
                const value = Math.max(1, Math.min(1440, Math.round(Number(e.target.value) || 30)))
                e.target.value = String(value)
                void update((s) => void (s.agentSleepMinutes = value))
              }} />
            <span className="text-[12px] text-muted-foreground">minutes</span>
          </Row>
        ) : null}
      </Group>
      <p className="text-[12px] text-muted-foreground">You can also right-click a finished chat in the sidebar and choose Sleep. Agents without session resume support stay awake.</p>
    </Section>
  )
}
