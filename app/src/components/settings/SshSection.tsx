import { useEffect, useState } from "react"
import { FolderPlus, LoaderCircle, Plus, Server, Trash2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ipc } from "@/lib/ipc"
import type { HostCheck, Tool } from "@/lib/types"
import { cn } from "@/lib/utils"
import { useApp } from "@/store/app"
import { Dot, Group, Section } from "./layout"
import { pickRemoteFolder } from "./RemoteFolder"

export function SshSection() {
  const hosts = useApp((s) => s.settings!.sshHosts)
  const update = useApp((s) => s.update)
  const addProject = useApp((s) => s.addProject)
  const [configHosts, setConfigHosts] = useState<string[]>([])
  const [draft, setDraft] = useState("")
  const [checks, setChecks] = useState<Record<string, HostCheck | "checking">>({})

  useEffect(() => {
    ipc.sshConfigHosts().then(setConfigHosts, () => setConfigHosts([]))
  }, [])

  const add = () => {
    const h = draft.trim()
    if (!h || hosts.some((x) => x.host === h)) return
    update((s) => void s.sshHosts.push({ host: h, port: 0, identityFile: "", label: "" }))
    setDraft("")
    test(h)
  }

  const test = async (host: string) => {
    setChecks((c) => ({ ...c, [host]: "checking" }))
    const r = await ipc.hostCheck({ kind: "ssh", host }).catch((e) => ({ ok: false, error: String(e) }) as HostCheck)
    setChecks((c) => ({ ...c, [host]: r }))
  }

  const addFolder = async (host: string) => {
    const path = await pickRemoteFolder({ kind: "ssh", host })
    if (path) {
      await addProject({ kind: "ssh", host }, path)
      useApp.getState().closeSettings()
    }
  }

  const all = [
    ...hosts.map((h) => ({ host: h.host, label: h.label, manual: true })),
    ...configHosts.filter((c) => !hosts.some((h) => h.host === c)).map((c) => ({ host: c, label: "", manual: false })),
  ]

  return (
    <Section
      title="SSH Hosts"
      description="Machines oah can work on. The agent CLI runs there over ssh, so its tools see that machine's files and shell. Key-based logins only (BatchMode)."
    >
      <Group title="Add a host">
        <div className="flex gap-2 p-3">
          <Input className="h-8 flex-1 font-mono text-[12px]" placeholder="user@hostname, or an alias from ~/.ssh/config" value={draft} onChange={(e) => setDraft(e.target.value)} onKeyDown={(e) => e.key === "Enter" && add()} />
          <Button size="sm" onClick={add} disabled={!draft.trim()}>
            <Plus /> Add host
          </Button>
        </div>
      </Group>
      <Group title="Hosts" aside={<span className="text-[11px] text-faint">{configHosts.length} from ~/.ssh/config</span>}>
        {all.length === 0 ? (
          <div className="px-4 py-3 text-[12px] text-faint">No hosts yet.</div>
        ) : (
          all.map((h) => {
            const c = checks[h.host]
            return (
              <div key={h.host}>
                <div className="flex items-center gap-3 px-4 py-2.5">
                  <Server className="size-4 text-cyan" />
                  <div className="min-w-0 flex-1">
                    <div className="font-mono text-[12.5px]">{h.label || h.host}</div>
                    <div className="mt-0.5 flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
                      {c === "checking" ? (
                        <>
                          <LoaderCircle className="size-3 animate-spin" /> Connecting…
                        </>
                      ) : c?.ok ? (
                        <>
                          <Dot tone="ok" /> Connected · {c.os} · {c.home}
                        </>
                      ) : c ? (
                        <>
                          <Dot tone="err" /> <span className="truncate text-red">{c.error}</span>
                        </>
                      ) : (
                        <>
                          <Dot tone="off" /> Not checked · {h.manual ? "added here" : "~/.ssh/config"}
                        </>
                      )}
                    </div>
                  </div>
                  <Button variant="outline" size="xs" onClick={() => test(h.host)} disabled={c === "checking"}>
                    Test
                  </Button>
                  <Button variant="outline" size="xs" onClick={() => addFolder(h.host)}>
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
                {c && c !== "checking" && c.ok ? <ToolGrid check={c} /> : null}
              </div>
            )
          })
        )}
      </Group>
    </Section>
  )
}

const TOOLS = [
  ["claude", "Claude Code"],
  ["codex", "Codex"],
  ["opencode", "OpenCode"],
  ["opencode2", "OpenCode 2"],
  ["antigravity", "Antigravity"],
  ["grok", "Grok Build"],
  ["git", "git"],
  ["gh", "gh"],
] as const

/** What a host or distro has installed. Agents are optional (any one will do); git is needed. */
export function ToolGrid({ check, extra }: { check: HostCheck; extra?: (tool: (typeof TOOLS)[number][0]) => React.ReactNode }) {
  return (
    <div className="grid grid-cols-5 gap-px border-t bg-border">
      {TOOLS.map(([t, name]) => (
        <ToolCell key={t} name={name} tool={check[t]} needed={t === "git"}>
          {extra?.(t)}
        </ToolCell>
      ))}
    </div>
  )
}

function ToolCell({ name, tool, needed, children }: { name: string; tool: Tool; needed: boolean; children?: React.ReactNode }) {
  const version = tool.version?.match(/\d+\.\d+(\.\d+)?/)?.[0]
  return (
    <div className="bg-card px-3 py-2" title={tool.windows ? `Only a Windows copy is on this distro's PATH (${tool.windows}). Install the Linux one there.` : (tool.path ?? undefined)}>
      <div className="flex items-center gap-1.5 text-[12px]">
        <Dot tone={tool.path ? "ok" : needed || tool.windows ? "warn" : "off"} /> {name}
      </div>
      <div className={cn("mt-0.5 truncate font-mono text-[11px]", tool.path ? "text-muted-foreground" : tool.windows ? "text-warn" : "text-faint")}>
        {tool.path ? (version ? `v${version}` : "installed") : tool.windows ? "Windows copy only" : "not installed"}
      </div>
      {children}
    </div>
  )
}
