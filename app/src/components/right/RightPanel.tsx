import { Files as FilesIcon, GitBranch, GitPullRequest, History as HistoryIcon, type LucideIcon } from "lucide-react"
import { Tip } from "@/components/Tip"
import { changeCount } from "@/lib/git"
import { cn } from "@/lib/utils"
import { type RightTab, useApp } from "@/store/app"
import { useScope, useStatus } from "@/store/git"
import { Files } from "./Files"
import { History } from "./History"
import { PullRequest } from "./PullRequest"
import { SourceControl } from "./SourceControl"

const TABS: { id: RightTab; label: string; icon: LucideIcon }[] = [
  { id: "files", label: "Files", icon: FilesIcon },
  { id: "scm", label: "Source Control", icon: GitBranch },
  { id: "history", label: "History", icon: HistoryIcon },
  { id: "pr", label: "Pull Request", icon: GitPullRequest },
]

/** The side panel on the right: files and git for the workspace on screen. */
export function RightPanel() {
  const tab = useApp((s) => s.layout.rightTab)
  const setLayout = useApp((s) => s.setLayout)
  const scope = useScope()
  const { status } = useStatus(scope)
  const count = changeCount(status)
  const current = TABS.find((t) => t.id === tab) ?? TABS[1]

  return (
    <aside className="flex h-full min-w-0 flex-col bg-background">
      <div className="flex h-10 shrink-0 items-center gap-0.5 border-b px-1.5">
        {TABS.map((t) => (
          <Tip key={t.id} content={t.label} side="bottom">
            <button
              role="tab"
              aria-selected={t.id === tab}
              aria-label={t.label}
              className={cn(
                "relative grid h-7 w-8 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground",
                t.id === tab && "bg-accent text-foreground",
              )}
              onClick={() => setLayout({ rightTab: t.id })}
            >
              <t.icon className="size-4" />
              {t.id === "scm" && count > 0 ? (
                <span className="rounded-full absolute -top-0.5 -right-0.5 min-w-3.5 bg-primary px-1 text-center font-mono text-[9px] leading-3.5 text-primary-foreground">{count > 99 ? "99" : count}</span>
              ) : null}
            </button>
          </Tip>
        ))}
        <span className="ml-auto self-center truncate pr-3 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">{current.label}</span>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {!scope ? (
          <div className="p-4 text-[12px] text-faint">No project selected.</div>
        ) : tab === "files" ? (
          <Files scope={scope} />
        ) : tab === "history" ? (
          <History scope={scope} />
        ) : tab === "pr" ? (
          <PullRequest scope={scope} />
        ) : (
          <SourceControl scope={scope} />
        )}
      </div>
    </aside>
  )
}
