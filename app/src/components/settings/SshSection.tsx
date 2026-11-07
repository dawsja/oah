import { useEffect, useState } from "react"
import { FolderPlus, Plus, Server, Trash2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ipc } from "@/lib/ipc"
import { useApp } from "@/store/app"
import { Dot, Group, Section } from "./layout"

export function SshSection() {
  const hosts = useApp((s) => s.settings!.sshHosts)
  const update = useApp((s) => s.update)
  const addProject = useApp((s) => s.addProject)
  const [configHosts, setConfigHosts] = useState<string[]>([])
  const [draft, setDraft] = useState("")

  useEffect(() => {
    ipc.sshConfigHosts().then(setConfigHosts, () => setConfigHosts([]))
  }, [])

  const add = () => {
    const h = draft.trim()
    if (!h || hosts.some((x) => x.host === h)) return
    update((s) => void s.sshHosts.push({ host: h, port: 0, identityFile: "", label: "" }))
    setDraft("")
  }

  const addRemoteProject = (host: string) => {
    const path = window.prompt(`Folder on ${host}`, "~/code/")
    if (path) addProject({ kind: "ssh", host }, path)
  }

  const all = [
    ...hosts.map((h) => ({ host: h.host, label: h.label, manual: true })),
    ...configHosts.filter((c) => !hosts.some((h) => h.host === c)).map((c) => ({ host: c, label: "", manual: false })),
  ]

  return (
    <Section
      title="SSH Hosts"
      description="Machines oah can work on. The agent CLI runs there over ssh, so its tools see that machine's files and shell. Key-based logins only."
    >
      <Group title="Add a host">
        <div className="flex gap-2 p-3">
          <Input className="h-8 flex-1 font-mono text-[12px]" placeholder="user@hostname or an alias from ~/.ssh/config" value={draft} onChange={(e) => setDraft(e.target.value)} onKeyDown={(e) => e.key === "Enter" && add()} />
          <Button size="sm" onClick={add} disabled={!draft.trim()}>
            <Plus /> Add host
          </Button>
        </div>
      </Group>
      <Group title="Hosts" aside={<span className="text-[11px] text-faint">{configHosts.length} from ~/.ssh/config</span>}>
        {all.length === 0 ? (
          <div className="px-4 py-3 text-[12px] text-faint">No hosts yet.</div>
        ) : (
          all.map((h) => (
            <div key={h.host} className="group flex items-center gap-3 px-4 py-2.5">
              <Server className="size-4 text-cyan" />
              <div className="min-w-0 flex-1">
                <div className="font-mono text-[12.5px]">{h.label || h.host}</div>
                <div className="mt-0.5 flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
                  <Dot tone="off" /> Not checked · {h.manual ? "added here" : "~/.ssh/config"}
                </div>
              </div>
              <Button variant="outline" size="xs">Test</Button>
              <Button variant="outline" size="xs" onClick={() => addRemoteProject(h.host)}>
                <FolderPlus /> Add folder
              </Button>
              {h.manual ? (
                <Button variant="ghost" size="icon-xs" aria-label="Remove host" onClick={() => update((s) => void (s.sshHosts = s.sshHosts.filter((x) => x.host !== h.host)))}>
                  <Trash2 />
                </Button>
              ) : (
                <span className="size-6" />
              )}
            </div>
          ))
        )}
      </Group>
    </Section>
  )
}
