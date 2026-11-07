import { Monitor, Moon, Sun } from "lucide-react"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import type { Density, Theme } from "@/lib/types"
import { cn } from "@/lib/utils"
import { useApp } from "@/store/app"
import { Group, Row, Section, Segmented } from "./layout"

const THEMES: { value: Theme; label: string; icon: typeof Sun }[] = [
  { value: "system", label: "System", icon: Monitor },
  { value: "dark", label: "Dark", icon: Moon },
  { value: "light", label: "Light", icon: Sun },
]

const SANS = ["Inter Variable", "Segoe UI", "system-ui"]
const MONO = ["JetBrains Mono Variable", "Cascadia Code", "Consolas"]

export function AppearanceSection() {
  const a = useApp((s) => s.settings!.appearance)
  const update = useApp((s) => s.update)

  return (
    <Section title="Appearance" description="How oah looks. Changes apply right away.">
      <Group title="Theme">
        <div className="grid grid-cols-3 gap-3 p-3">
          {THEMES.map((t) => (
            <button
              key={t.value}
              className={cn("border text-left hover:border-ring/60", a.theme === t.value ? "border-primary" : "border-border")}
              onClick={() => update((s) => void (s.appearance.theme = t.value))}
            >
              <ThemePreview theme={t.value} />
              <div className="flex items-center gap-2 border-t px-3 py-2 text-[12.5px]">
                <t.icon className="size-3.5 text-muted-foreground" />
                {t.label}
                {a.theme === t.value ? <span className="ml-auto size-1.5 bg-primary" /> : null}
              </div>
            </button>
          ))}
        </div>
      </Group>
      <Group title="Text">
        <Row label="Interface size">
          <Stepper value={a.uiFontSize} min={11} max={16} onChange={(v) => update((s) => void (s.appearance.uiFontSize = v))} />
        </Row>
        <Row label="Chat text size">
          <Stepper value={a.chatFontSize} min={12} max={20} onChange={(v) => update((s) => void (s.appearance.chatFontSize = v))} />
        </Row>
        <Row label="Interface font">
          <FontSelect value={a.sansFont} fonts={SANS} onChange={(v) => update((s) => void (s.appearance.sansFont = v))} />
        </Row>
        <Row label="Code font">
          <FontSelect value={a.monoFont} fonts={MONO} mono onChange={(v) => update((s) => void (s.appearance.monoFont = v))} />
        </Row>
      </Group>
      <Group title="Layout">
        <Row label="Density" description="Compact fits more chats and files on screen.">
          <Segmented<Density>
            value={a.density}
            options={[
              { value: "compact", label: "Compact" },
              { value: "comfortable", label: "Comfortable" },
            ]}
            onChange={(v) => update((s) => void (s.appearance.density = v))}
          />
        </Row>
      </Group>
    </Section>
  )
}

function Stepper({ value, min, max, onChange }: { value: number; min: number; max: number; onChange: (v: number) => void }) {
  return (
    <div className="flex items-center border border-input">
      <button className="h-7 w-7 text-muted-foreground hover:bg-accent disabled:opacity-40" disabled={value <= min} onClick={() => onChange(value - 1)} aria-label="Smaller">
        −
      </button>
      <span className="w-12 border-x border-input text-center font-mono text-[12px] leading-7">{value}px</span>
      <button className="h-7 w-7 text-muted-foreground hover:bg-accent disabled:opacity-40" disabled={value >= max} onClick={() => onChange(value + 1)} aria-label="Larger">
        +
      </button>
    </div>
  )
}

function FontSelect({ value, fonts, mono, onChange }: { value: string; fonts: string[]; mono?: boolean; onChange: (v: string) => void }) {
  const all = fonts.includes(value) ? fonts : [value, ...fonts]
  return (
    <Select value={value} onValueChange={onChange}>
      <SelectTrigger size="sm" className="w-60">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {all.map((f) => (
          <SelectItem key={f} value={f} style={{ fontFamily: `"${f}"` }} className={mono ? "font-mono" : undefined}>
            {f.replace(" Variable", "")}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

/** A tiny drawing of the window in that theme. */
function ThemePreview({ theme }: { theme: Theme }) {
  const half = theme === "system"
  const pane = (dark: boolean) => (
    <div className={cn("flex h-full flex-1", dark ? "bg-black" : "bg-[#f6f7f9]")}>
      <div className={cn("w-1/3 border-r p-1.5", dark ? "border-white/10 bg-[#111]" : "border-black/10 bg-[#eef0f3]")}>
        <div className={cn("mb-1 h-1.5 w-3/4", dark ? "bg-white/15" : "bg-black/10")} />
        <div className="mb-1 h-1.5 w-2/3 bg-[#ffd43b]" />
        <div className={cn("h-1.5 w-1/2", dark ? "bg-white/15" : "bg-black/10")} />
      </div>
      <div className="flex flex-1 flex-col justify-end p-1.5">
        <div className={cn("mb-1 h-1.5 w-4/5", dark ? "bg-white/20" : "bg-black/15")} />
        <div className={cn("h-4 border", dark ? "border-white/15 bg-white/5" : "border-black/15 bg-white")} />
      </div>
    </div>
  )
  return <div className="flex h-20 overflow-hidden">{half ? (<>{pane(true)}{pane(false)}</>) : pane(theme === "dark")}</div>
}
