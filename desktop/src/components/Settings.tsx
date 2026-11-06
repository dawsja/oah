// Settings. Everything here is the CLI's own configuration, changed through the
// engine (the same code paths as /model, /auth, /permissions…), so the terminal
// and the desktop app always agree.
import { ChevronDown, FolderOpen, FolderSync } from "lucide-react"
import { useState, type ReactNode } from "react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { useClient } from "@/lib/client"
import type { Effort } from "@/lib/protocol"
import { cn } from "@/lib/utils"
import { PERMISSION, type PermissionLevel } from "./icons"
import { UpdateSettings } from "./Updates"

const EFFORT_INFO: Record<string, string> = {
  minimal: "fastest, almost no reasoning",
  low: "quick answers, light reasoning",
  medium: "balanced speed and depth",
  high: "deeper reasoning for hard problems",
  xhigh: "very deep reasoning for long agentic work",
  max: "everything it has",
}
const PERMS = (Object.keys(PERMISSION) as PermissionLevel[]).map((value) => ({ value, ...PERMISSION[value] }))

const SECTIONS = [
  ["general", "General"],
  ["providers", "Providers & sign-in"],
  ["models", "Models & effort"],
  ["permissions", "Permissions & safety"],
  ["mcp", "MCP servers"],
  ["skills", "Skills"],
  ["memory", "Memory"],
  ["import", "Import"],
  ["updates", "Updates & engine"],
] as const

const Dot = ({ className }: { className: string }) => <span className={cn("size-2 shrink-0 rounded-full", className)} />

/** A row in a settings list: status dot, title + detail, actions. */
function Row({ dot, title, detail, mono, children }: { dot?: string; title: ReactNode; detail?: ReactNode; mono?: boolean; children?: ReactNode }) {
  return (
    <div className="flex items-center gap-3 border-b px-4 py-3 last:border-b-0">
      {dot ? <Dot className={dot} /> : null}
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="text-sm">{title}</span>
        {detail ? <span className={cn("truncate text-xs text-muted-foreground", mono && "font-mono")}>{detail}</span> : null}
      </div>
      {children}
    </div>
  )
}

function KeyForm({ label }: { label: string }) {
  const c = useClient()
  const [v, setV] = useState("")
  return (
    <form
      className="flex items-center gap-2 border-b bg-primary/5 px-4 py-3 last:border-b-0"
      onSubmit={(e) => {
        e.preventDefault()
        c.send({ op: "submit", text: v })
        setV("")
      }}
    >
      <Label htmlFor="key" className="shrink-0">{label} key</Label>
      <Input id="key" type="password" autoFocus className="h-8 font-mono" placeholder="Paste the key" value={v} onChange={(e) => setV(e.target.value)} onKeyDown={(e) => e.key === "Escape" && c.send({ op: "cancelKey" })} />
      <Button type="submit" size="sm" disabled={!v.trim()}>Save</Button>
      <Button type="button" size="sm" variant="ghost" onClick={() => c.send({ op: "cancelKey" })}>Cancel</Button>
    </form>
  )
}

function Section({ id, title, description, children }: { id: string; title: string; description?: ReactNode; children: ReactNode }) {
  return (
    <section id={id} className="scroll-mt-6">
      <Card className="gap-4 bg-card/60">
        <CardHeader>
          <CardTitle className="text-base">{title}</CardTitle>
          {description ? <CardDescription>{description}</CardDescription> : null}
        </CardHeader>
        <CardContent className="flex flex-col gap-4">{children}</CardContent>
      </Card>
    </section>
  )
}

const Code = ({ children }: { children: ReactNode }) => <code className="rounded bg-muted px-1 py-0.5 font-mono text-[12px] text-code">{children}</code>

export function Settings() {
  const c = useClient()
  const s = c.state
  const info = c.info
  if (!s || !info) return <div className="grid flex-1 place-items-center text-muted-foreground">Waiting for the engine…</div>
  const run = (cmd: string, toChat = false) => {
    c.submit(cmd)
    if (toChat) c.setView("chat")
  }
  const os = info.os === "windows" ? "Windows" : "Linux"
  const keyMode = s.inputMode.startsWith("key:") ? s.inputMode.slice(4) : null
  const keyProviders = s.auth.providers.filter((p) => !p.local)
  const local = s.auth.providers.filter((p) => p.local)
  const chatgpt = s.auth.chatgpt

  return (
    <div className="flex min-h-0 flex-1">
      <nav aria-label="Settings sections" className="hidden w-52 shrink-0 flex-col gap-0.5 border-r border-sidebar-border bg-rail p-3 md:flex">
        <h1 className="px-3 pt-1 pb-3 text-lg font-semibold">Settings</h1>
        {SECTIONS.map(([id, label]) => (
          <Button key={id} variant="ghost" size="sm" className="justify-start font-normal text-muted-foreground hover:text-foreground" onClick={() => document.getElementById(id)?.scrollIntoView({ behavior: "smooth" })}>
            {label}
          </Button>
        ))}
        <div className="mt-auto px-3 py-2 font-mono text-xs text-faint">oah {c.hello?.version ?? "?"}</div>
      </nav>
      <ScrollArea className="flex-1">
        <div className="mx-auto flex max-w-3xl flex-col gap-6 px-8 py-8">
          <Section id="general" title="General" description="Sessions, settings, sign-ins, memory, skills and MCP servers live in one folder the terminal app uses too.">
            <div className="flex items-start gap-3 rounded-lg border border-primary/25 bg-primary/5 p-4">
              <FolderSync className="mt-0.5 size-5 shrink-0 text-primary" />
              <div className="flex flex-col gap-1">
                <span className="font-medium">Shared with the oah CLI</span>
                <span className="text-sm text-muted-foreground">
                  Start a conversation here and continue it in a terminal with <Code>oah -c</Code>, or the other way round.
                </span>
              </div>
            </div>
            <div className="grid gap-2 sm:grid-cols-3">
              {[["Linux", "~/.config/oah"], ["Windows", "%USERPROFILE%\\.config\\oah"]].map(([name, path]) => (
                <div key={name} className={cn("flex flex-col gap-0.5 rounded-lg border bg-background px-3 py-2", name === os && "border-primary/40")}>
                  <span className="flex items-center gap-2 text-xs text-muted-foreground">
                    {name}
                    {name === os ? <Badge variant="outline" className="h-4 border-primary/40 px-1.5 text-[10px] text-primary">this computer</Badge> : null}
                  </span>
                  <span className="font-mono text-xs select-text">{path}</span>
                </div>
              ))}
            </div>
            <div className="flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
              <span className="min-w-0 flex-1">
                In use: <span className="font-mono text-foreground select-text">{info.home}</span>
                {info.oahHomeEnv ? " (from $OAH_HOME)" : info.xdgEnv ? " (from $XDG_CONFIG_HOME)" : ""}
              </span>
              <Button variant="outline" size="sm" onClick={() => void c.t.openFolder("home")}>
                <FolderOpen />
                Open folder
              </Button>
            </div>
          </Section>

          <Section id="providers" title="Providers & sign-in" description="The same accounts as /auth in the terminal.">
            <div className="overflow-hidden rounded-lg border">
              <Row dot={chatgpt?.planUsage ? "bg-green" : chatgpt ? "bg-warn" : "border border-faint"} title="ChatGPT" detail={chatgpt ? (chatgpt.planUsage ? `Signed in${chatgpt.email ? ` as ${chatgpt.email}` : ""} · uses your plan` : "Signed in, but plan usage is off") : "Use your ChatGPT plan (Plus, Pro, Business…)"}>
                {chatgpt ? (
                  <Button size="sm" variant="outline" onClick={() => c.send({ op: "auth", action: "logout-chatgpt" })}>Sign out</Button>
                ) : (
                  <Button size="sm" disabled={!!s.busy} onClick={() => c.send({ op: "auth", action: "chatgpt" })}>Continue with ChatGPT</Button>
                )}
              </Row>
              {keyProviders.map((p) =>
                keyMode === p.id ? (
                  <KeyForm key={p.id} label={p.label} />
                ) : (
                  <Row key={p.id} dot={p.source ? "bg-green" : "border border-faint"} title={p.label} mono={!!p.source && p.source !== "saved key"} detail={p.source ? (p.source === "saved key" ? "Key saved in auth.json" : `${p.source} · from the environment`) : p.blurb}>
                    {p.source === "saved key" ? <Button size="sm" variant="ghost" className="text-red hover:text-red" onClick={() => c.send({ op: "auth", action: `remove:${p.id}` })}>Remove</Button> : null}
                    <Button size="sm" variant="outline" onClick={() => c.send({ op: "auth", action: `key:${p.id}` })}>{p.source ? "Replace key" : "Add key"}</Button>
                  </Row>
                ),
              )}
              {local.map((p) => (
                <Row key={p.id} dot={p.source ? "bg-cyan" : "border border-faint"} title={p.label} detail={p.source ? "Running on this computer" : `Not running — ${p.blurb}`}>
                  <Button size="sm" variant="outline" onClick={() => c.send({ op: "auth", action: `local:${p.id}` })}>{p.source ? "Pick a model" : "Check again"}</Button>
                </Row>
              ))}
            </div>
            {s.loginUrl ? (
              <p className="text-sm text-muted-foreground">
                Waiting for the browser sign-in…{" "}
                <Button variant="link" size="sm" className="h-auto p-0" onClick={() => void c.t.copy(s.loginUrl!)}>Copy link</Button>
              </p>
            ) : null}
          </Section>

          <Section id="models" title="Models & effort" description="Applies to this session and becomes the default for new ones — like /model and /effort.">
            <div className="grid gap-4 sm:grid-cols-3">
              <div className="flex flex-col gap-2">
                <Label>Model</Label>
                <Button variant="outline" className="justify-between font-normal" onClick={() => run("/model")}>
                  <span className="truncate">{s.model.name}</span>
                  <ChevronDown className="opacity-50" />
                </Button>
              </div>
              <div className="flex flex-col gap-2">
                <Label>Reasoning effort</Label>
                <Select value={s.settings.effort} disabled={!s.model.efforts.length} onValueChange={(v) => c.send({ op: "effort", value: v as Effort })}>
                  <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    {(s.model.efforts.length ? s.model.efforts : [s.settings.effort]).map((e) => (
                      <SelectItem key={e} value={e}>
                        {e} <span className="text-muted-foreground">— {EFFORT_INFO[e]}</span>
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="flex flex-col gap-2">
                <Label>Mode</Label>
                <Select value={s.settings.mode} onValueChange={(v) => c.send({ op: "mode", value: v as "plan" | "build" })}>
                  <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="build">build <span className="text-muted-foreground">— edit files, run commands</span></SelectItem>
                    <SelectItem value="plan">plan <span className="text-muted-foreground">— read-only research</span></SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>
          </Section>

          <Section id="permissions" title="Permissions & safety">
            <RadioGroup value={s.settings.permission} onValueChange={(v) => c.send({ op: "permissionLevel", value: v as "auto" | "ask" | "allow" })} className="gap-2">
              {PERMS.map((p) => (
                <Label key={p.value} htmlFor={`perm-${p.value}`} className={cn("flex cursor-pointer items-start gap-3 rounded-lg border p-3 font-normal hover:bg-accent/40", s.settings.permission === p.value && "border-primary/40 bg-primary/5")}>
                  <RadioGroupItem id={`perm-${p.value}`} value={p.value} className="mt-0.5" />
                  <span className="flex flex-col gap-0.5">
                    <span className={cn("flex items-center gap-1.5 font-medium", p.className)}><p.icon className="size-4" />{p.label}</span>
                    <span className="text-sm text-muted-foreground">{p.desc}</span>
                  </span>
                </Label>
              ))}
            </RadioGroup>
            <div className="flex items-center justify-between gap-4 rounded-lg border p-3">
              <Label htmlFor="autocompact" className="flex flex-col items-start gap-0.5 font-normal">
                <span className="font-medium">Auto-compact</span>
                <span className="text-sm text-muted-foreground">Summarize the conversation when the context is {Math.round(s.settings.autoCompactAt * 100)}% full</span>
              </Label>
              <Switch id="autocompact" checked={s.settings.autoCompact} onCheckedChange={(v) => c.send({ op: "settings", patch: { autoCompact: v } })} />
            </div>
            <div className="flex items-center justify-between gap-4 rounded-lg border p-3">
              <Label className="flex flex-col items-start gap-0.5 font-normal">
                <span className="font-medium">Checkpoints per session</span>
                <span className="text-sm text-muted-foreground">How far back /undo can restore files</span>
              </Label>
              <Select value={String(s.settings.maxCheckpoints || 20)} onValueChange={(v) => c.send({ op: "settings", patch: { maxCheckpoints: Number(v) } })}>
                <SelectTrigger className="w-24"><SelectValue /></SelectTrigger>
                <SelectContent>
                  {[5, 10, 20, 50, 100].map((n) => <SelectItem key={n} value={String(n)}>{n}</SelectItem>)}
                </SelectContent>
              </Select>
            </div>
          </Section>

          <Section id="mcp" title="MCP servers" description={<>From <Code>~/.config/oah/mcp.json</Code>, <Code>./.oah/mcp.json</Code> and <Code>./.mcp.json</Code>. Ask oah to add one ("add the Linear MCP server").</>}>
            {s.mcp.length ? (
              <div className="overflow-hidden rounded-lg border">
                {s.mcp.map((m) => (
                  <Row key={m.name} dot={m.state === "ready" ? "bg-green" : m.state === "needs-auth" ? "bg-warn" : m.state === "failed" ? "bg-red" : "animate-pulse bg-faint"} title={m.name} detail={`${m.status}${m.signedIn ? " · signed in" : ""} · ${m.source}`}>
                    {m.state === "needs-auth" ? <Button size="sm" onClick={() => c.send({ op: "mcpSignIn", name: m.name })}>Sign in</Button> : null}
                    {m.signedIn ? <Button size="sm" variant="outline" onClick={() => run(`/mcp logout ${m.name}`)}>Sign out</Button> : null}
                  </Row>
                ))}
              </div>
            ) : null}
            <div className="flex flex-wrap gap-2">
              <Button variant="outline" size="sm" onClick={() => run("/mcp reload", true)}>Reload config</Button>
              <Button variant="outline" size="sm" onClick={() => run("/import mcp", true)}>Import from Claude Code</Button>
            </div>
          </Section>

          <Section id="skills" title="Skills" description={<>Folders with a <Code>SKILL.md</Code> in <Code>~/.config/oah/skills</Code> or <Code>./.oah/skills</Code> (<Code>.claude/skills</Code> and <Code>.agents/skills</Code> work too). Run one with <Code>/name</Code>.</>}>
            {s.skills.length ? (
              <div className="overflow-hidden rounded-lg border">
                {s.skills.map((k) => (
                  <Row key={k.name} title={<span className="font-mono">/{k.name}</span>} detail={k.description}>
                    <Badge variant="secondary">{k.scope === "project" ? "Project" : "User"}</Badge>
                  </Row>
                ))}
              </div>
            ) : (
              <p className="text-sm text-muted-foreground">No skills found.</p>
            )}
          </Section>

          <Section id="memory" title="Memory & AGENTS.md" description={<>Notes oah keeps across sessions. Start a message with <Code>#</Code> to save one to project memory.</>}>
            <div className="flex flex-wrap gap-2">
              <Button variant="outline" size="sm" onClick={() => run("/memory", true)}>Show memory</Button>
              <Button variant="outline" size="sm" disabled={!!s.busy} onClick={() => run("/init", true)}>Write AGENTS.md for this project</Button>
            </div>
          </Section>

          <Section id="import" title="Import from Claude Code" description="Bring over MCP servers, skills and sessions.">
            <div className="flex flex-wrap gap-2">
              <Button size="sm" onClick={() => run("/import", true)}>Import everything</Button>
              <Button variant="outline" size="sm" onClick={() => run("/import mcp", true)}>MCP servers</Button>
              <Button variant="outline" size="sm" onClick={() => run("/import skills", true)}>Skills</Button>
              <Button variant="outline" size="sm" onClick={() => run("/import sessions", true)}>Sessions</Button>
            </div>
          </Section>

          <Section id="updates" title="Updates & engine" description="The app updates itself from dl.oah.dev; the engine inside it updates with it. The terminal CLI updates separately (oah update).">
            <UpdateSettings />
            <div className="overflow-hidden rounded-lg border">
              <Row title={<>Engine · oah {c.hello?.version}{s.updateAvailable ? <Badge className="ml-2">oah {s.updateAvailable} available</Badge> : null}</>} detail={c.started?.engine}>
                {c.info?.bundle ? null : <Button size="sm" variant="outline" onClick={() => run("/update", true)}>Update the CLI</Button>}
                <Button size="sm" variant="ghost" onClick={() => c.restart()}>Restart engine</Button>
              </Row>
            </div>
          </Section>
        </div>
      </ScrollArea>
    </div>
  )
}
