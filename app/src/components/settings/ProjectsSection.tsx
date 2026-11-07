import { Folder, FolderPlus, Server, Trash2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { pickFolder } from "@/lib/ipc"
import { AGENTS, AGENT_LABEL, locationLabel, type Project } from "@/lib/types"
import { useApp } from "@/store/app"
import { Group, Row, Section } from "./layout"

export function ProjectsSection() {
  const projects = useApp((s) => s.settings!.projects)
  const addProject = useApp((s) => s.addProject)
  const openSettings = useApp((s) => s.openSettings)

  return (
    <Section
      title="Projects"
      description="Folders oah works in, on this computer or an SSH host. Git options for each project live here too."
      actions={
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={() => openSettings("ssh")}>
            <Server /> Remote folder
          </Button>
          <Button
            size="sm"
            onClick={async () => {
              const path = await pickFolder("Add a project folder")
              if (path) await addProject({ kind: "local" }, path)
            }}
          >
            <FolderPlus /> Add folder
          </Button>
        </div>
      }
    >
      {projects.length === 0 ? (
        <Group>
          <div className="px-4 py-6 text-center text-[12.5px] text-muted-foreground">No projects yet.</div>
        </Group>
      ) : (
        projects.map((p) => <ProjectCard key={p.id} project={p} />)
      )}
    </Section>
  )
}

function ProjectCard({ project }: { project: Project }) {
  const update = useApp((s) => s.update)
  const removeProject = useApp((s) => s.removeProject)
  const edit = (f: (p: Project) => void) =>
    update((s) => {
      const p = s.projects.find((x) => x.id === project.id)
      if (p) f(p)
    })
  const remote = project.location.kind === "ssh"
  const Icon = remote ? Server : Folder

  return (
    <Group>
      <div className="flex items-center gap-3 px-4 py-3">
        <div className="grid size-9 place-items-center border bg-background">
          <Icon className={remote ? "size-4 text-cyan" : "size-4 text-muted-foreground"} />
        </div>
        <div className="min-w-0 flex-1">
          <input
            className="w-full bg-transparent text-[14px] font-semibold outline-none focus:bg-muted"
            defaultValue={project.name}
            aria-label="Project name"
            onBlur={(e) => edit((p) => void (p.name = e.target.value.trim() || p.name))}
          />
          <div className="mt-0.5 truncate font-mono text-[11.5px] text-muted-foreground">
            {locationLabel(project.location)} · {project.path}
          </div>
        </div>
        <Button variant="ghost" size="sm" className="text-muted-foreground hover:text-red" onClick={() => removeProject(project.id)}>
          <Trash2 /> Remove
        </Button>
      </div>
      <Row label="Default agent">
        <Select value={project.defaultAgent ?? "default"} onValueChange={(v) => edit((p) => void (p.defaultAgent = v === "default" ? null : (v as Project["defaultAgent"])))}>
          <SelectTrigger size="sm" className="w-48">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="default">Providers default</SelectItem>
            {AGENTS.map((a) => (
              <SelectItem key={a} value={a}>
                {AGENT_LABEL[a]}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </Row>
      <Row label="Default model" description="Empty uses the provider's default.">
        <Input className="h-7 w-48 font-mono text-[12px]" defaultValue={project.defaultModel} onBlur={(e) => edit((p) => void (p.defaultModel = e.target.value.trim()))} />
      </Row>
      <Row label="Base branch" description="New worktrees branch from this. Empty uses the remote's default branch.">
        <Input className="h-7 w-48 font-mono text-[12px]" placeholder="origin/main" defaultValue={project.git.baseBranch} onBlur={(e) => edit((p) => void (p.git.baseBranch = e.target.value.trim()))} />
      </Row>
      <Row label="Worktree folder" description={`Where new worktrees go. Empty uses ${project.name}-worktrees next to the project.`}>
        <Input className="h-7 w-72 font-mono text-[12px]" placeholder={`../${project.name}-worktrees`} defaultValue={project.git.worktreeDir} onBlur={(e) => edit((p) => void (p.git.worktreeDir = e.target.value.trim()))} />
      </Row>
      <Row label="Delete branch with worktree" description="Removing a worktree also deletes its local branch.">
        <Switch checked={project.git.deleteBranchOnRemove} onCheckedChange={(v) => edit((p) => void (p.git.deleteBranchOnRemove = v))} />
      </Row>
    </Group>
  )
}
