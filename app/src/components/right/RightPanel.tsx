import { Files, GitBranch, GitPullRequest, History, type LucideIcon } from "lucide-react"
import { Tip } from "@/components/Tip"
import { cn } from "@/lib/utils"
import { type RightTab, useApp, useProject } from "@/store/app"

const TABS: { id: RightTab; label: string; icon: LucideIcon }[] = [
  { id: "files", label: "Files", icon: Files },
  { id: "scm", label: "Source Control", icon: GitBranch },
  { id: "history", label: "History", icon: History },
  { id: "pr", label: "Pull Request", icon: GitPullRequest },
]

/** The side panel on the right: files and git for the current workspace. */
export function RightPanel() {
  const tab = useApp((s) => s.layout.rightTab)
  const setLayout = useApp((s) => s.setLayout)
  const project = useProject()
  const current = TABS.find((t) => t.id === tab) ?? TABS[1]

  return (
    <aside className="flex h-full min-w-0 flex-col border-l bg-sidebar">
      <div className="flex h-9 shrink-0 items-stretch border-b">
        {TABS.map((t) => (
          <Tip key={t.id} content={t.label} side="bottom">
            <button
              role="tab"
              aria-selected={t.id === tab}
              aria-label={t.label}
              className={cn(
                "relative grid w-10 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground",
                t.id === tab && "bg-background text-foreground after:absolute after:inset-x-0 after:bottom-0 after:h-0.5 after:bg-primary",
              )}
              onClick={() => setLayout({ rightTab: t.id })}
            >
              <t.icon className="size-4" />
            </button>
          </Tip>
        ))}
        <span className="ml-auto self-center pr-3 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">{current.label}</span>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {project ? (
          <div className="p-4 text-[12px] leading-relaxed text-muted-foreground">
            <div className="border border-dashed p-3">
              {current.label} for <span className="text-foreground">{project.name}</span> shows up here.
            </div>
          </div>
        ) : (
          <div className="p-4 text-[12px] text-faint">No project selected.</div>
        )}
      </div>
    </aside>
  )
}
