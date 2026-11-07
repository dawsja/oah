import { FolderPlus, Server } from "lucide-react"
import { Mark } from "@/components/icons"
import { Button } from "@/components/ui/button"
import { pickFolder } from "@/lib/ipc"
import { AGENT_LABEL, locationLabel } from "@/lib/types"
import { useApp, useProject } from "@/store/app"
import { Composer } from "./Composer"

/** The middle of the window: the current chat, or a new one. */
export function ChatView() {
  const settings = useApp((s) => s.settings)
  const project = useProject()
  if (!settings) return null
  if (!project) return <NoProject />

  const agent = project.defaultAgent ?? settings.providers.defaultAgent
  const provider = settings.providers[agent]
  return (
    <div className="flex h-full min-w-0 flex-col bg-background">
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center px-6 pb-[12vh]">
        <div className="w-full max-w-[46rem]">
          <div className="mb-6 flex items-end gap-3">
            <Mark size={34} />
            <div>
              <h1 className="text-[22px] leading-tight font-semibold tracking-tight">What should we work on?</h1>
              <p className="mt-1 text-[12.5px] text-muted-foreground">
                {AGENT_LABEL[agent]} in <span className="font-medium text-foreground">{project.name}</span>
                <span className="text-faint"> · {locationLabel(project.location)}</span>
              </p>
            </div>
          </div>
          <Composer
            agent={agent}
            model={project.defaultModel || provider.defaultModel}
            permission={provider.permissionMode}
            onAgent={(a) => useApp.getState().update((s) => {
              const p = s.projects.find((x) => x.id === project.id)
              if (p) p.defaultAgent = a
            })}
            onPermission={(m) => useApp.getState().update((s) => void (s.providers[agent].permissionMode = m))}
          />
          <div className="mt-3 grid grid-cols-3 gap-px border bg-border text-[12px]">
            {["Explain how this project is laid out", "Find and fix a failing test", "Review my uncommitted changes"].map((t) => (
              <button key={t} className="bg-background px-3 py-2.5 text-left text-muted-foreground hover:bg-accent hover:text-foreground">
                {t}
              </button>
            ))}
          </div>
        </div>
      </div>
    </div>
  )
}

function NoProject() {
  const addProject = useApp((s) => s.addProject)
  const openSettings = useApp((s) => s.openSettings)
  return (
    <div className="flex h-full flex-col items-center justify-center bg-background px-6 pb-[10vh]">
      <Mark size={48} label="oah" />
      <h1 className="mt-5 text-[22px] font-semibold tracking-tight">Open Agent Harness</h1>
      <p className="mt-2 max-w-md text-center text-[13px] leading-relaxed text-muted-foreground">
        Chat with Claude Code and Codex in your projects, on this computer or on an SSH host. Add a project to start.
      </p>
      <div className="mt-6 flex gap-2">
        <Button
          onClick={async () => {
            const path = await pickFolder("Add a project folder")
            if (path) await addProject({ kind: "local" }, path)
          }}
        >
          <FolderPlus /> Open folder
        </Button>
        <Button variant="outline" onClick={() => openSettings("ssh")}>
          <Server /> Folder on an SSH host
        </Button>
      </div>
    </div>
  )
}
