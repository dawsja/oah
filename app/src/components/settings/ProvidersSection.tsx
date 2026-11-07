import { useEffect, useState } from "react"
import { Plus, RefreshCw, Trash2, X } from "lucide-react"
import { AgentGlyph } from "@/components/icons"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { ipc } from "@/lib/ipc"
import { promptText } from "@/components/Prompt"
import { PERMISSION_ICON } from "@/components/permission"
import { AGENTS, AGENT_LABEL, PERMISSION_LABEL, type Account, type AcpAgent, type AgentKind, type PermissionMode, type Probe, type Provider, type Settings } from "@/lib/types"
import { useApp } from "@/store/app"
import { Dot, Group, Row, Section, Segmented } from "./layout"

const BLURB: Record<AgentKind, string> = {
  claude: "Anthropic's agent. Uses your Claude subscription or API key from `claude` itself.",
  codex: "OpenAI's agent. Uses your ChatGPT plan or API key from `codex` itself.",
  opencode: "The open source agent, over ACP (`opencode acp`). Uses the providers you signed in to with OpenCode.",
  acp: "An agent that speaks the Agent Client Protocol on stdio.",
}

const INSTALL: Partial<Record<AgentKind, string>> = {
  claude: "npm i -g @anthropic-ai/claude-code",
  codex: "npm i -g @openai/codex",
  opencode: "npm i -g opencode-ai",
}

/** Edits one agent's settings: a built-in by kind, or an added ACP agent by id. */
type Target = { kind: AgentKind; acpId: string | null }
function providerOf(s: Settings, t: Target): Provider | undefined {
  return t.kind === "acp" ? s.providers.acp.find((a) => a.id === t.acpId) : s.providers[t.kind]
}

export function ProvidersSection() {
  const settings = useApp((s) => s.settings)!
  const update = useApp((s) => s.update)
  return (
    <Section title="Providers" description="The agent CLIs oah chats through. Each one runs on the machine the project is on, with its own sign-in.">
      <Group>
        <Row label="Default agent" description="New chats start with this agent unless the project picks another.">
          <Segmented
            value={settings.providers.defaultAgent}
            options={AGENTS.filter((a) => settings.providers[a].enabled).map((a) => ({ value: a, label: AGENT_LABEL[a] }))}
            onChange={(v) => update((s) => void (s.providers.defaultAgent = v))}
          />
        </Row>
      </Group>
      {AGENTS.map((a) => (
        <ProviderCard key={a} target={{ kind: a, acpId: null }} provider={settings.providers[a]} />
      ))}
      <AcpAgents agents={settings.providers.acp} />
    </Section>
  )
}

function AcpAgents({ agents }: { agents: AcpAgent[] }) {
  const update = useApp((s) => s.update)
  const add = async () => {
    const command = await promptText({ title: "Add an ACP agent", description: "The command that starts it in ACP mode, with its arguments.", placeholder: "my-agent --acp", mono: true, confirm: "Next" })
    if (!command) return
    const words = command.trim().split(/\s+/)
    const name = await promptText({ title: "Name it", value: words[0], confirm: "Add agent" })
    if (!name) return
    update((s) =>
      void s.providers.acp.push({
        id: Math.random().toString(16).slice(2, 10),
        name,
        args: words.slice(1),
        enabled: true,
        binary: words[0],
        defaultModel: "",
        permissionMode: "ask",
        env: [],
      }),
    )
  }
  return (
    <>
      <div className="mt-2 flex items-end gap-3">
        <div className="flex-1">
          <h2 className="text-[15px] font-semibold">ACP agents</h2>
          <p className="mt-0.5 text-[12px] text-muted-foreground">Any other agent that speaks the Agent Client Protocol. Chats with it work like the others.</p>
        </div>
        <Button size="sm" onClick={add}>
          <Plus /> Add agent
        </Button>
      </div>
      {agents.length === 0 ? <div className="rounded-lg border border-dashed px-4 py-3 text-[12px] text-faint">No ACP agents added.</div> : null}
      {agents.map((a) => (
        <ProviderCard key={a.id} target={{ kind: "acp", acpId: a.id }} provider={a} />
      ))}
    </>
  )
}

function ProviderCard({ target, provider }: { target: Target; provider: Provider | AcpAgent }) {
  const update = useApp((s) => s.update)
  const kind = target.kind
  const acp = kind === "acp" ? (provider as AcpAgent) : null
  const [probe, setProbe] = useState<Probe | null>(null)
  const [checking, setChecking] = useState(false)
  const edit = (f: (p: Provider) => void) =>
    update((s) => {
      const p = providerOf(s, target)
      if (p) f(p)
    })
  const editAcp = (f: (a: AcpAgent) => void) =>
    update((s) => {
      const a = s.providers.acp.find((x) => x.id === target.acpId)
      if (a) f(a)
    })

  const check = async () => {
    setChecking(true)
    try {
      setProbe(await ipc.providerProbe(kind, provider.binary))
    } finally {
      setChecking(false)
    }
  }
  useEffect(() => {
    check()
    // Re-check when the binary path changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [provider.binary])

  const status = !probe ? (
    <span className="text-faint">Checking…</span>
  ) : probe.found && probe.version ? (
    <>
      <Dot tone={probe.outdated ? "warn" : "ok"} /> v{probe.version}
      {probe.outdated ? <span className="text-warn">· too old</span> : null}
    </>
  ) : probe.found ? (
    <>
      <Dot tone="warn" /> {probe.error}
    </>
  ) : (
    <>
      <Dot tone="err" /> Not installed
    </>
  )

  return (
    <Group>
      <div className="flex items-center gap-3 px-4 py-3">
        <div className="rounded-md grid size-9 place-items-center border bg-background">
          <AgentGlyph agent={kind} className="size-5" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-[14px] font-semibold">
            {acp ? (
              <input
                className="rounded-sm min-w-0 bg-transparent font-semibold outline-none focus:bg-muted"
                defaultValue={acp.name}
                aria-label="Agent name"
                onBlur={(e) => editAcp((a) => void (a.name = e.target.value.trim() || a.name))}
              />
            ) : (
              AGENT_LABEL[kind]
            )}
            <span className="flex items-center gap-1.5 text-[11.5px] font-normal text-muted-foreground">{status}</span>
          </div>
          <div className="mt-0.5 truncate text-[12px] text-muted-foreground">{probe?.path ?? BLURB[kind]}</div>
        </div>
        <Button variant="ghost" size="icon-sm" aria-label="Check again" onClick={check} disabled={checking}>
          <RefreshCw className={checking ? "animate-spin" : undefined} />
        </Button>
        <Switch checked={provider.enabled} onCheckedChange={(v) => edit((p) => void (p.enabled = v))} aria-label={`Use ${acp?.name ?? AGENT_LABEL[kind]}`} />
        {acp ? (
          <Button variant="ghost" size="icon-sm" aria-label="Remove agent" onClick={() => update((s) => void (s.providers.acp = s.providers.acp.filter((a) => a.id !== acp.id)))}>
            <Trash2 />
          </Button>
        ) : null}
      </div>
      {probe?.outdated ? <div className="border-t border-warn/30 bg-warn/10 px-4 py-2.5 text-[12px] text-warn">{probe.outdated}</div> : null}
      {!probe?.found && probe && INSTALL[kind] ? <div className="bg-del/40 px-4 py-2.5 font-mono text-[12px] text-muted-foreground">Install: {INSTALL[kind]}</div> : null}
      {acp ? null : <SignIn kind={kind} installed={!!probe?.found} />}
      <Row label="Default model" description="Empty uses the CLI's own default.">
        <Input
          className="h-7 w-56 font-mono text-[12px]"
          placeholder={kind === "claude" ? "e.g. opus, sonnet" : kind === "codex" ? "e.g. gpt-5.5-codex" : kind === "opencode" ? "e.g. anthropic/claude-sonnet-5" : "the agent's model id"}
          defaultValue={provider.defaultModel}
          onBlur={(e) => edit((p) => void (p.defaultModel = e.target.value.trim()))}
        />
      </Row>
      <Row label="Permissions" description="What the agent may do without asking.">
        <Segmented<PermissionMode>
          value={provider.permissionMode}
          options={(["ask", "acceptEdits", "plan", "fullAccess"] as const).map((m) => {
            const Icon = PERMISSION_ICON[m]
            return { value: m, label: <span className="flex items-center gap-1.5"><Icon className="size-3.5" />{PERMISSION_LABEL[m]}</span> }
          })}
          onChange={(v) => edit((p) => void (p.permissionMode = v))}
        />
      </Row>
      <Row label={acp ? "Command" : "Executable"} description={acp ? "The program, and the arguments that start it in ACP mode." : "Leave empty to use the one on PATH."}>
        <Input
          className="h-7 w-48 font-mono text-[12px]"
          placeholder={acp ? "command" : kind}
          defaultValue={provider.binary}
          onBlur={(e) => edit((p) => void (p.binary = e.target.value.trim()))}
        />
        {acp ? (
          <Input
            className="h-7 w-48 font-mono text-[12px]"
            placeholder="--acp"
            defaultValue={acp.args.join(" ")}
            onBlur={(e) => editAcp((a) => void (a.args = e.target.value.split(/\s+/).filter(Boolean)))}
          />
        ) : null}
      </Row>
      <EnvRows target={target} label={acp?.name ?? AGENT_LABEL[kind]} provider={provider} />
    </Group>
  )
}

function SignIn({ kind, installed }: { kind: AgentKind; installed: boolean }) {
  const [account, setAccount] = useState<Account | null>(null)
  const check = () => {
    setAccount(null)
    ipc.providerAccount(kind).then(setAccount, (e) => setAccount({ signedIn: false, label: null, plan: null, error: String(e) }))
  }
  useEffect(() => {
    if (installed) check()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [installed])
  const desc = !installed ? (
    "Install it first."
  ) : !account ? (
    "Checking…"
  ) : account.signedIn ? (
    <span className="flex items-center gap-1.5">
      <Dot tone="ok" /> Signed in{account.label ? <> as <span className="text-foreground">{account.label}</span></> : null}
      {account.plan ? <span className="rounded-sm border px-1 font-mono text-[10.5px] uppercase">{account.plan}</span> : null}
    </span>
  ) : (
    <span className="flex items-center gap-1.5">
      <Dot tone="warn" /> {account.error ?? "Not signed in"}
    </span>
  )
  return (
    <Row label="Sign-in" description={desc}>
      {account && !account.signedIn ? (
        <Button size="xs" onClick={() => ipc.providerLogin(kind).then(() => setTimeout(check, 4000))}>
          Sign in
        </Button>
      ) : null}
      <Button variant="outline" size="xs" onClick={check} disabled={!installed}>
        Check
      </Button>
    </Row>
  )
}

function EnvRows({ target, label, provider }: { target: Target; label: string; provider: Provider }) {
  const update = useApp((s) => s.update)
  const edit = (f: (p: Provider) => void) =>
    update((s) => {
      const p = providerOf(s, target)
      if (p) f(p)
    })
  return (
    <div className="px-4 py-2.5">
      <div className="flex items-center">
        <div className="flex-1">
          <div className="text-[13px] font-medium">Environment</div>
          <div className="mt-0.5 text-[12px] text-muted-foreground">Extra variables for the {label} process. Values are kept in your OS keychain, not in oah's settings file.</div>
        </div>
        <Button variant="outline" size="xs" onClick={() => edit((p) => void p.env.push({ key: "", value: "", inKeychain: false }))}>
          <Plus /> Add
        </Button>
      </div>
      {provider.env.length ? (
        <div className="mt-2 flex flex-col gap-1">
          {provider.env.map((v, i) => (
            <div key={i} className="flex gap-1">
              <Input className="h-7 w-48 font-mono text-[12px]" placeholder="NAME" defaultValue={v.key} onBlur={(e) => edit((p) => void (p.env[i].key = e.target.value.trim()))} />
              <Input
                className="h-7 flex-1 font-mono text-[12px]"
                placeholder={v.inKeychain ? "•••••• saved in keychain (type to replace)" : "value"}
                type="password"
                defaultValue={v.value}
                onBlur={(e) => e.target.value && edit((p) => void (p.env[i].value = e.target.value))}
              />
              <Button variant="ghost" size="icon-sm" aria-label="Remove" onClick={() => edit((p) => void p.env.splice(i, 1))}>
                <X />
              </Button>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}
