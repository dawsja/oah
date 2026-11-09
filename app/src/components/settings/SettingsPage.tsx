import { Activity, AudioLines, Bot, FolderGit2, Moon, Palette, Plug, Server, Smartphone, X, type LucideIcon } from "lucide-react"
import { cn } from "@/lib/utils"
import { updater, useUpdates } from "@/lib/updates"
import { updateStatus } from "@/components/shell/Updates"
import { type SettingsSection, useApp } from "@/store/app"
import { AppearanceSection } from "./AppearanceSection"
import { McpSection } from "./McpSection"
import { PhoneSection } from "./PhoneSection"
import { ProjectsSection } from "./ProjectsSection"
import { ProvidersSection } from "./ProvidersSection"
import { SshSection } from "./SshSection"
import { UsageSection } from "./UsageSection"
import { SessionsSection } from "./SessionsSection"
import { VoiceSection } from "./VoiceSection"

const SECTIONS: { id: SettingsSection; label: string; icon: LucideIcon }[] = [
  { id: "providers", label: "Providers", icon: Bot },
  { id: "sessions", label: "Sessions", icon: Moon },
  { id: "voice", label: "Voice", icon: AudioLines },
  { id: "appearance", label: "Appearance", icon: Palette },
  { id: "usage", label: "Usage", icon: Activity },
  { id: "ssh", label: "SSH Hosts", icon: Server },
  { id: "projects", label: "Projects", icon: FolderGit2 },
  { id: "mcp", label: "MCP", icon: Plug },
  { id: "phone", label: "Phone", icon: Smartphone },
]

export function SettingsPage() {
  const section = useApp((s) => s.section)
  const info = useApp((s) => s.info)
  const close = useApp((s) => s.closeSettings)
  const update = useUpdates((u) => u.state)

  return (
    <div className="flex h-full min-w-0 bg-background">
      <nav className="flex w-52 shrink-0 flex-col border-r bg-sidebar/60 px-1.5 py-3">
        <div className="mb-2 px-4 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">Settings</div>
        {SECTIONS.map((s) => (
          <button
            key={s.id}
            className={cn("rounded-md flex h-8 items-center gap-2.5 px-3 text-[12.5px] text-muted-foreground hover:bg-accent hover:text-foreground", s.id === section && "row-active")}
            onClick={() => useApp.setState({ section: s.id })}
          >
            <s.icon className="size-3.5" />
            {s.label}
          </button>
        ))}
        <div className="mt-auto px-4 font-mono text-[10.5px] leading-relaxed break-all text-faint">
          oah {info?.version}
          <br />
          {info?.dataDir}
          <br />
          <span className={update.kind === "error" ? "text-red" : undefined}>{updateStatus()}</span>
          {update.kind !== "off" ? (
            <button className="mt-1 block rounded-md border px-2 py-0.5 font-sans text-[11px] text-muted-foreground hover:bg-accent hover:text-foreground" onClick={() => void updater.check(true)}>
              Check for updates
            </button>
          ) : null}
        </div>
      </nav>
      <div className="relative min-w-0 flex-1 overflow-y-auto">
        <button className="rounded-full absolute top-3 right-3 grid size-8 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Close settings" onClick={close}>
          <X className="size-4" />
        </button>
        {section === "providers" ? <ProvidersSection /> : null}
        {section === "sessions" ? <SessionsSection /> : null}
        {section === "voice" ? <VoiceSection /> : null}
        {section === "appearance" ? <AppearanceSection /> : null}
        {section === "usage" ? <UsageSection /> : null}
        {section === "ssh" ? <SshSection /> : null}
        {section === "projects" ? <ProjectsSection /> : null}
        {section === "mcp" ? <McpSection /> : null}
        {section === "phone" ? <PhoneSection /> : null}
      </div>
    </div>
  )
}
