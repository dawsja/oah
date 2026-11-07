import { useState } from "react"
import { ChevronDown, FolderPlus, LoaderCircle, Play, Trash2 } from "lucide-react"
import { LocationIcon } from "@/components/LocationIcon"
import { addWslFolder, OpenProjectMenu, useDistros } from "@/components/OpenProject"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { ipc } from "@/lib/ipc"
import { AGENTS, AGENT_LABEL, locationLabel, type Account, type AgentKind, type Distro, type HostCheck, type Location, type Project } from "@/lib/types"
import { useApp } from "@/store/app"
import { Dot, Group, Row, Section } from "./layout"
import { ToolGrid } from "./SshSection"

export function ProjectsSection() {
  const projects = useApp((s) => s.settings!.projects)

  return (
    <Section
      title="Projects"
      description="Folders oah works in: on this computer, in a WSL distro, or on an SSH host. Git options for each project live here too."
      actions={
        <OpenProjectMenu align="end">
          <Button size="sm">
            <FolderPlus /> Add project <ChevronDown />
          </Button>
        </OpenProjectMenu>
      }
    >
      <WslGroup />
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

  return (
    <Group>
      <div className="flex items-center gap-3 px-4 py-3">
        <div className="rounded-md grid size-9 place-items-center border bg-background">
          <LocationIcon location={project.location} className="size-4" />
        </div>
        <div className="min-w-0 flex-1">
          <input
            className="rounded-sm w-full bg-transparent text-[14px] font-semibold outline-none focus:bg-muted"
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

/** WSL distros on this computer: start one, see what's installed and signed in, add a folder. */
function WslGroup() {
  const { distros, reload } = useDistros()
  if (!distros?.length) return null
  return (
    <Group title="WSL" aside={<span className="text-[11px] text-faint">Projects in a distro run their agent, git and files inside it</span>}>
      {distros.map((d) => (
        <DistroRow key={d.name} distro={d} onChange={reload} />
      ))}
    </Group>
  )
}

const AGENT_TOOLS: Partial<Record<string, AgentKind>> = { claude: "claude", codex: "codex", opencode: "opencode" }

function DistroRow({ distro, onChange }: { distro: Distro; onChange: () => void }) {
  const location: Location = { kind: "wsl", distro: distro.name }
  const [check, setCheck] = useState<HostCheck | "checking" | null>(null)
  const [accounts, setAccounts] = useState<Partial<Record<AgentKind, Account | "checking">>>({})
  const [starting, setStarting] = useState(false)

  const test = async () => {
    setCheck("checking")
    const c = await ipc.hostCheck(location).catch((e) => ({ ok: false, error: String(e) }) as HostCheck)
    setCheck(c)
    onChange()
    if (!c.ok) return
    for (const kind of ["claude", "codex", "opencode"] as AgentKind[]) {
      if (!c[kind as "claude"].path) continue
      setAccounts((a) => ({ ...a, [kind]: "checking" }))
      ipc.providerAccount(kind, location).then(
        (acc) => setAccounts((a) => ({ ...a, [kind]: acc })),
        (e) => setAccounts((a) => ({ ...a, [kind]: { signedIn: false, label: null, plan: null, error: String(e) } })),
      )
    }
  }
  const start = async () => {
    setStarting(true)
    await ipc.wslStart(distro.name).catch((e) => setCheck({ ok: false, error: String(e) } as HostCheck))
    setStarting(false)
    onChange()
  }

  const c = check
  return (
    <div>
      <div className="flex items-center gap-3 px-4 py-2.5">
        <LocationIcon location={location} className="size-4" />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 font-mono text-[12.5px]">
            {distro.name}
            {distro.default ? <span className="rounded-sm border px-1 font-sans text-[10px] text-faint">default</span> : null}
            {distro.version === 1 ? <span className="rounded-sm border px-1 font-sans text-[10px] text-faint">WSL 1</span> : null}
          </div>
          <div className="mt-0.5 flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
            {c === "checking" ? (
              <>
                <LoaderCircle className="size-3 animate-spin" /> {distro.running ? "Checking…" : "Starting and checking…"}
              </>
            ) : c?.ok ? (
              <>
                <Dot tone="ok" /> {c.os} · {c.home}
              </>
            ) : c ? (
              <>
                <Dot tone="err" /> <span className="truncate text-red">{c.error}</span>
              </>
            ) : (
              <>
                <Dot tone={distro.running ? "ok" : "off"} /> {distro.running ? "Running" : "Stopped"}
              </>
            )}
          </div>
        </div>
        {!distro.running ? (
          <Button variant="outline" size="xs" onClick={start} disabled={starting}>
            {starting ? <LoaderCircle className="animate-spin" /> : <Play />} Start
          </Button>
        ) : null}
        <Button variant="outline" size="xs" onClick={test} disabled={c === "checking"}>
          Check
        </Button>
        <Button variant="outline" size="xs" onClick={() => void addWslFolder(distro.name)}>
          <FolderPlus /> Add folder
        </Button>
      </div>
      {c && c !== "checking" && c.ok ? (
        <ToolGrid
          check={c}
          extra={(t) => {
            const kind = AGENT_TOOLS[t]
            if (!kind || !c[t].path) return null
            const a = accounts[kind]
            if (!a) return null
            if (a === "checking") return <div className="mt-1 text-[10.5px] text-faint">Checking sign-in…</div>
            if (a.signedIn) return <div className="mt-1 truncate text-[10.5px] text-green">{a.label ?? "Signed in"}</div>
            return (
              <button
                className="mt-1 text-[10.5px] text-foreground underline underline-offset-2"
                onClick={() => ipc.providerLogin(kind, location).then(() => setTimeout(test, 5000), (e) => setCheck({ ok: false, error: String(e) } as HostCheck))}
              >
                Sign in…
              </button>
            )
          }}
        />
      ) : null}
    </div>
  )
}
