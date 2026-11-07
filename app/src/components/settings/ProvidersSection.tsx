import { useEffect, useState } from "react"
import { Plus, RefreshCw, X } from "lucide-react"
import { AgentGlyph } from "@/components/icons"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { ipc } from "@/lib/ipc"
import { AGENTS, AGENT_LABEL, PERMISSION_LABEL, type Account, type AgentKind, type PermissionMode, type Probe, type Provider } from "@/lib/types"
import { useApp } from "@/store/app"
import { Dot, Group, Row, Section, Segmented } from "./layout"

const BLURB: Record<AgentKind, string> = {
  claude: "Anthropic's agent. Uses your Claude subscription or API key from `claude` itself.",
  codex: "OpenAI's agent. Uses your ChatGPT plan or API key from `codex` itself.",
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
            options={AGENTS.map((a) => ({ value: a, label: AGENT_LABEL[a] }))}
            onChange={(v) => update((s) => void (s.providers.defaultAgent = v))}
          />
        </Row>
      </Group>
      {AGENTS.map((a) => (
        <ProviderCard key={a} kind={a} provider={settings.providers[a]} />
      ))}
    </Section>
  )
}

function ProviderCard({ kind, provider }: { kind: AgentKind; provider: Provider }) {
  const update = useApp((s) => s.update)
  const [probe, setProbe] = useState<Probe | null>(null)
  const [checking, setChecking] = useState(false)
  const edit = (f: (p: Provider) => void) => update((s) => f(s.providers[kind]))

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
      <Dot tone="ok" /> v{probe.version}
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
        <div className="grid size-9 place-items-center border bg-background">
          <AgentGlyph agent={kind} className="size-5" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-[14px] font-semibold">
            {AGENT_LABEL[kind]}
            <span className="flex items-center gap-1.5 text-[11.5px] font-normal text-muted-foreground">{status}</span>
          </div>
          <div className="mt-0.5 truncate text-[12px] text-muted-foreground">{probe?.path ?? BLURB[kind]}</div>
        </div>
        <Button variant="ghost" size="icon-sm" aria-label="Check again" onClick={check} disabled={checking}>
          <RefreshCw className={checking ? "animate-spin" : undefined} />
        </Button>
        <Switch checked={provider.enabled} onCheckedChange={(v) => edit((p) => void (p.enabled = v))} aria-label={`Use ${AGENT_LABEL[kind]}`} />
      </div>
      {!probe?.found && probe ? (
        <div className="bg-del/40 px-4 py-2.5 font-mono text-[12px] text-muted-foreground">
          Install: {kind === "claude" ? "npm i -g @anthropic-ai/claude-code" : "npm i -g @openai/codex"}
        </div>
      ) : null}
      <SignIn kind={kind} installed={!!probe?.found} />
      <Row label="Default model" description="Empty uses the CLI's own default.">
        <Input
          className="h-7 w-56 font-mono text-[12px]"
          placeholder={kind === "claude" ? "e.g. opus, sonnet" : "e.g. gpt-5.5-codex"}
          defaultValue={provider.defaultModel}
          onBlur={(e) => edit((p) => void (p.defaultModel = e.target.value.trim()))}
        />
      </Row>
      <Row label="Permissions" description="What the agent may do without asking.">
        <Segmented<PermissionMode>
          value={provider.permissionMode}
          options={(["ask", "acceptEdits", "plan", "fullAccess"] as const).map((m) => ({ value: m, label: PERMISSION_LABEL[m] }))}
          onChange={(v) => edit((p) => void (p.permissionMode = v))}
        />
      </Row>
      <Row label="Executable" description="Leave empty to use the one on PATH.">
        <Input
          className="h-7 w-72 font-mono text-[12px]"
          placeholder={kind}
          defaultValue={provider.binary}
          onBlur={(e) => edit((p) => void (p.binary = e.target.value.trim()))}
        />
      </Row>
      <EnvRows kind={kind} provider={provider} />
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
      {account.plan ? <span className="border px-1 font-mono text-[10.5px] uppercase">{account.plan}</span> : null}
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

function EnvRows({ kind, provider }: { kind: AgentKind; provider: Provider }) {
  const update = useApp((s) => s.update)
  const edit = (f: (p: Provider) => void) => update((s) => f(s.providers[kind]))
  return (
    <div className="px-4 py-2.5">
      <div className="flex items-center">
        <div className="flex-1">
          <div className="text-[13px] font-medium">Environment</div>
          <div className="mt-0.5 text-[12px] text-muted-foreground">Extra variables for the {AGENT_LABEL[kind]} process.</div>
        </div>
        <Button variant="outline" size="xs" onClick={() => edit((p) => void p.env.push({ key: "", value: "" }))}>
          <Plus /> Add
        </Button>
      </div>
      {provider.env.length ? (
        <div className="mt-2 flex flex-col gap-1">
          {provider.env.map((v, i) => (
            <div key={i} className="flex gap-1">
              <Input className="h-7 w-48 font-mono text-[12px]" placeholder="NAME" defaultValue={v.key} onBlur={(e) => edit((p) => void (p.env[i].key = e.target.value.trim()))} />
              <Input className="h-7 flex-1 font-mono text-[12px]" placeholder="value" type="password" defaultValue={v.value} onBlur={(e) => edit((p) => void (p.env[i].value = e.target.value))} />
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
