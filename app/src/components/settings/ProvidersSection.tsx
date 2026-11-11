import { useCallback, useEffect, useState } from "react"
import { Check, ChevronDown, ChevronRight, Copy, EyeOff, ExternalLink, Plus, RefreshCw, X } from "lucide-react"
import { AgentGlyph } from "@/components/icons"
import { Button } from "@/components/ui/button"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { Tip } from "@/components/Tip"
import { listed, loadChoices, saveChoices, type ListChoice } from "@/lib/provider-list"
import { useChats } from "@/store/chats"
import { useShallow } from "zustand/react/shallow"
import { Input } from "@/components/ui/input"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { ipc } from "@/lib/ipc"
import { copyText } from "@/lib/clipboard"
import { PERMISSION_ICON } from "@/components/permission"
import { AGENTS, AGENT_LABEL, PERMISSION_LABEL, type Account, type AgentKind, type PermissionMode, type Probe, type Provider, type Settings } from "@/lib/types"
import { useApp } from "@/store/app"
import { Dot, Group, Row, Section, Segmented } from "./layout"

const BLURB: Record<AgentKind, string> = {
  claude: "Anthropic's agent. Uses your Claude subscription or API key from `claude` itself.",
  codex: "OpenAI's agent. Uses your ChatGPT plan or API key from `codex` itself.",
  opencode: "The open source agent, over ACP (`opencode acp`). Uses the providers you signed in to with OpenCode.",
  opencode2: "OpenCode 2 by its `opencode2` command, for when `opencode` is still 1.x. Same sign-ins and history as OpenCode.",
  antigravity: "Google's agent, over ACP (`agy acp`). Uses your Google AI plan, signed in with `agy` itself.",
  grok: "xAI's agent, over ACP (`grok agent stdio`). Uses your SuperGrok plan, signed in with `grok` itself, or an `XAI_API_KEY`.",
}

/** Each built-in CLI's docs, and the commands that install it. */
type Installer = { label: string; os: "unix" | "windows" | "any"; command: string }
const INSTALL: Partial<Record<AgentKind, { url: string; options: Installer[] }>> = {
  claude: {
    url: "https://code.claude.com/docs/en/setup",
    options: [
      { label: "macOS / Linux / WSL", os: "unix", command: "curl -fsSL https://claude.ai/install.sh | bash" },
      { label: "Windows", os: "windows", command: "irm https://claude.ai/install.ps1 | iex" },
    ],
  },
  codex: {
    url: "https://learn.chatgpt.com/docs/codex/cli",
    options: [
      { label: "macOS / Linux / WSL", os: "unix", command: "curl -fsSL https://chatgpt.com/codex/install.sh | sh" },
      { label: "Windows", os: "windows", command: 'powershell -ExecutionPolicy ByPass -c "irm https://chatgpt.com/codex/install.ps1 | iex"' },
    ],
  },
  opencode: {
    url: "https://opencode.ai/docs",
    options: [
      { label: "macOS / Linux / WSL", os: "unix", command: "curl -fsSL https://opencode.ai/install | bash" },
      { label: "npm", os: "any", command: "npm install -g opencode-ai" },
    ],
  },
  // The v2 installer and @opencode/cli both put `opencode2` (and a 2.x `opencode`) on PATH.
  opencode2: {
    url: "https://opencode.ai/docs",
    options: [
      { label: "macOS / Linux / WSL", os: "unix", command: "curl -fsSL https://opencode.ai/v2/install | bash" },
      { label: "npm", os: "any", command: "npm install -g @opencode/cli" },
    ],
  },
  antigravity: {
    url: "https://antigravity.google/docs/getting-started",
    options: [
      { label: "macOS / Linux / WSL", os: "unix", command: "curl -fsSL https://antigravity.google/cli/install.sh | bash" },
      { label: "Windows", os: "windows", command: "irm https://antigravity.google/cli/install.ps1 | iex" },
    ],
  },
  grok: {
    url: "https://docs.x.ai/build/overview",
    options: [
      { label: "macOS / Linux / WSL", os: "unix", command: "curl -fsSL https://x.ai/cli/install.sh | bash" },
      { label: "Windows", os: "windows", command: "irm https://x.ai/cli/install.ps1 | iex" },
    ],
  },
}

/** Each provider's CLI on this computer, looked for once and again when the window comes back
 *  (after installing one in a terminal). */
function useProbes(providers: Settings["providers"]) {
  const [probes, setProbes] = useState<Partial<Record<AgentKind, Probe>>>({})
  const [checking, setChecking] = useState<Partial<Record<AgentKind, boolean>>>({})
  const check = useCallback(
    async (kind: AgentKind) => {
      setChecking((c) => ({ ...c, [kind]: true }))
      try {
        const p = await ipc.providerProbe(kind, providers[kind].binary)
        setProbes((ps) => ({ ...ps, [kind]: p }))
      } finally {
        setChecking((c) => ({ ...c, [kind]: false }))
      }
    },
    [providers],
  )
  const binaries = AGENTS.map((a) => providers[a].binary).join("\n")
  useEffect(() => {
    for (const a of AGENTS) void check(a)
    // Again when an executable path changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [binaries])
  // What wasn't found is looked for again when the user comes back from the terminal: an agent
  // installed meanwhile shows up by itself.
  useEffect(() => {
    const again = () => {
      for (const a of AGENTS) if (probes[a] && !probes[a]!.found) void check(a)
    }
    window.addEventListener("focus", again)
    return () => window.removeEventListener("focus", again)
  }, [probes, check])
  return { probes, checking, check }
}

export function ProvidersSection() {
  const settings = useApp((s) => s.settings)!
  const update = useApp((s) => s.update)
  const { probes, checking, check } = useProbes(settings.providers)
  // The agents chats use (one per agent, sorted, so it only changes when that does).
  const used = useChats(useShallow((s) => AGENTS.filter((a) => s.metas.some((m) => m.agent === a))))
  const [choices, setChoices] = useState(loadChoices)
  const choose = (kind: AgentKind, choice: ListChoice) => {
    const next = { ...choices, [kind]: choice }
    setChoices(next)
    saveChoices(next)
  }
  const shown = (kind: AgentKind) =>
    listed(choices[kind], {
      installed: probes[kind] ? probes[kind]!.found : null,
      isDefault: settings.providers.defaultAgent === kind,
      inUse: used.includes(kind),
    })
  const listedAgents = AGENTS.filter(shown)
  const others = AGENTS.filter((a) => !shown(a))
  const looking = AGENTS.some((a) => !probes[a])
  return (
    <Section title="Providers" description="The agent CLIs oah chats through. Each one runs on the machine the project is on, with its own sign-in.">
      <Group>
        <Row label="Default agent" description="New chats start with this agent unless the project picks another.">
          <Select
            value={settings.providers.defaultAgent}
            onValueChange={(v) => update((s) => void (s.providers.defaultAgent = v as Settings["providers"]["defaultAgent"]))}
          >
            <SelectTrigger size="sm" className="w-48" aria-label="Default agent">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {AGENTS.filter((a) => settings.providers[a].enabled).map((a) => (
                <SelectItem key={a} value={a}>
                  {AGENT_LABEL[a]}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </Row>
      </Group>
      {listedAgents.map((a) => (
        <ProviderCard
          key={a}
          kind={a}
          provider={settings.providers[a]}
          probe={probes[a] ?? null}
          checking={!!checking[a]}
          onCheck={() => void check(a)}
          onHide={() => choose(a, "hidden")}
        />
      ))}
      {others.length ? (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" className="self-start border-dashed text-muted-foreground hover:text-foreground">
              <Plus /> Add provider <ChevronDown className="opacity-60" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="w-72">
            <DropdownMenuLabel className="text-[11px] font-normal text-muted-foreground">
              {looking ? "Looking for installed agents…" : "Installed agents show up here by themselves."}
            </DropdownMenuLabel>
            {others.map((a) => (
              <DropdownMenuItem key={a} onSelect={() => choose(a, "shown")}>
                <AgentGlyph agent={a} className="size-4" />
                <span className="flex-1">{AGENT_LABEL[a]}</span>
                <span className="text-[11px] text-faint">{!probes[a] ? "Checking…" : probes[a]!.found ? "Hidden" : "Not installed"}</span>
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
    </Section>
  )
}

function ProviderCard({
  kind,
  provider,
  probe,
  checking,
  onCheck,
  onHide,
}: {
  kind: AgentKind
  provider: Provider
  probe: Probe | null
  checking: boolean
  onCheck: () => void
  onHide: () => void
}) {
  const update = useApp((s) => s.update)
  const edit = (f: (p: Provider) => void) => update((s) => f(s.providers[kind]))

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
            {AGENT_LABEL[kind]}
            <span className="flex items-center gap-1.5 text-[11.5px] font-normal text-muted-foreground">{status}</span>
          </div>
          <div className="mt-0.5 truncate text-[12px] text-muted-foreground">{probe?.path ?? BLURB[kind]}</div>
        </div>
        {INSTALL[kind] ? (
          <Button variant="ghost" size="xs" className="text-muted-foreground" onClick={() => import("@tauri-apps/plugin-opener").then((o) => o.openUrl(INSTALL[kind]!.url))}>
            Docs <ExternalLink className="size-3" />
          </Button>
        ) : null}
        <Button variant="ghost" size="icon-sm" aria-label="Check again" onClick={onCheck} disabled={checking}>
          <RefreshCw className={checking ? "animate-spin" : undefined} />
        </Button>
        <Tip content="Hide from this list (Add provider brings it back)">
          <Button variant="ghost" size="icon-sm" aria-label={`Hide ${AGENT_LABEL[kind]}`} onClick={onHide}>
            <EyeOff />
          </Button>
        </Tip>
        <Switch checked={provider.enabled} onCheckedChange={(v) => edit((p) => void (p.enabled = v))} aria-label={`Use ${AGENT_LABEL[kind]}`} />
      </div>
      {probe?.outdated ? <div className="border-t border-warn/30 bg-warn/10 px-4 py-2.5 text-[12px] text-warn">{probe.outdated}</div> : null}
      {probe && !probe.found && INSTALL[kind] ? <InstallHelp kind={kind} /> : null}
      <SignIn kind={kind} installed={!!probe?.found} />
      <details className="group/advanced">
        <summary className="flex min-h-12 cursor-pointer list-none items-center gap-2 px-4 py-2.5 text-[13px] font-medium text-muted-foreground hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
          <ChevronRight className="size-4 shrink-0 transition-transform group-open/advanced:rotate-90" />
          Advanced Settings
        </summary>
        <div className="divide-y border-t">
          <Row label="Default model" description="Empty uses the CLI's own default.">
            <Input
              className="h-7 w-56 font-mono text-[12px]"
              placeholder={kind === "claude" ? "e.g. opus, sonnet" : kind === "codex" ? "e.g. gpt-5.5-codex" : kind === "antigravity" ? "e.g. gemini-3.8-flash" : kind === "grok" ? "e.g. grok-4.7" : "e.g. anthropic/claude-sonnet-5"}
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
          <Row label="Executable" description="Leave empty to use the one on PATH.">
            <Input
              className="h-7 w-48 font-mono text-[12px]"
              placeholder={kind}
              defaultValue={provider.binary}
              onBlur={(e) => edit((p) => void (p.binary = e.target.value.trim()))}
            />
          </Row>
          <EnvRows kind={kind} label={AGENT_LABEL[kind]} provider={provider} />
        </div>
      </details>
    </Group>
  )
}

/** How to install a missing CLI: the command for this OS, copyable. */
function InstallHelp({ kind }: { kind: AgentKind }) {
  const { options } = INSTALL[kind]!
  const platform = useApp((s) => s.info?.platform)
  const os = platform === "windows" ? "windows" : "unix"
  const [label, setLabel] = useState(() => (options.find((o) => o.os === os) ?? options.find((o) => o.os === "any") ?? options[0]).label)
  const [copied, setCopied] = useState(false)
  const command = (options.find((o) => o.label === label) ?? options[0]).command
  const copy = () => {
    copyText(command)
    setCopied(true)
    setTimeout(() => setCopied(false), 1500)
  }
  return (
    <div className="flex flex-col gap-2 bg-muted/40 px-4 py-3">
      <span className="text-[13px] font-medium">Install {AGENT_LABEL[kind]}</span>
      {options.length > 1 ? (
        <div className="self-start">
          <Segmented value={label} options={options.map((o) => ({ value: o.label, label: o.label }))} onChange={setLabel} />
        </div>
      ) : null}
      <div className="flex items-center rounded-md border bg-background">
        <code className="min-w-0 flex-1 truncate px-2.5 py-1.5 font-mono text-[12px] select-all" title={command}>
          {command}
        </code>
        <Button variant="ghost" size="icon-sm" className="mr-0.5" aria-label={copied ? "Copied" : "Copy install command"} onClick={copy}>
          {copied ? <Check className="text-green" /> : <Copy />}
        </Button>
      </div>
      <p className="text-[11.5px] text-faint">Run it in a terminal, then switch back here to re-check. Still not found? Restart oah.</p>
    </div>
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
      {account.plan ? <span className="rounded-full border px-1.5 font-mono text-[10.5px] uppercase">{account.plan}</span> : null}
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

function EnvRows({ kind, label, provider }: { kind: AgentKind; label: string; provider: Provider }) {
  const update = useApp((s) => s.update)
  const edit = (f: (p: Provider) => void) => update((s) => f(s.providers[kind]))
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
