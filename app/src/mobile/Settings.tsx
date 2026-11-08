import { Monitor, Moon, Sun } from "lucide-react"
import type { Theme } from "@/lib/types"
import { cn } from "@/lib/utils"
import { setPhoneTheme, usePhoneTheme } from "./theme"
import { TopBar } from "./ui"
import { VoiceSettings } from "./Voice"

const THEMES: { value: Theme; label: string; icon: typeof Sun }[] = [
  { value: "system", label: "System", icon: Monitor },
  { value: "dark", label: "Dark", icon: Moon },
  { value: "light", label: "Light", icon: Sun },
]

/** Settings that belong to this phone, not to a desktop or a chat. */
export function SettingsScreen() {
  const theme = usePhoneTheme((s) => s.theme)
  return (
    <div className="safe flex h-full flex-col bg-background">
      <TopBar title="Settings" backable />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <section className="px-4 py-4">
          <h2 id="phone-theme-label" className="mb-2 text-[13px] font-medium">Appearance</h2>
          <div className="grid grid-cols-3 gap-2" role="group" aria-labelledby="phone-theme-label">
            {THEMES.map(({ value, label, icon: Icon }) => (
              <button key={value} aria-pressed={theme === value} onClick={() => setPhoneTheme(value)}
                className={cn("flex min-h-12 items-center justify-center gap-2 rounded-md border px-3 text-[14px]", theme === value ? "border-primary bg-primary text-primary-foreground" : "bg-card text-foreground active:bg-accent")}>
                <Icon className="size-4" />{label}
              </button>
            ))}
          </div>
          <p className="mt-2 text-[12px] text-muted-foreground">System follows your phone’s light or dark mode. This setting applies only to this phone.</p>
        </section>
        <VoiceSettings />
      </div>
    </div>
  )
}
