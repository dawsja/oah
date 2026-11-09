import { useEffect, useState } from "react"
import { Check, Copy, Eye, EyeOff, RefreshCw } from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { copyText } from "@/lib/clipboard"
import { ipc } from "@/lib/ipc"
import type { McpActivity, McpStatus } from "@/lib/types"
import { useApp } from "@/store/app"
import { Dot, Group, Row, Section, Segmented } from "./layout"

/** `C:\Users\me\oah.exe` as WSL sees it: `/mnt/c/Users/me/oah.exe`. */
export function wslPath(p: string): string {
  const m = /^([A-Za-z]):[\\/](.*)$/.exec(p)
  return m ? `/mnt/${m[1].toLowerCase()}/${m[2].replace(/\\/g, "/")}` : p.replace(/\\/g, "/")
}

type Snippet = { id: string; label: string; hint: string; text: string }

/** Ready-to-paste client configs that run `<exe> mcp` (stdio). */
export function snippets(exe: string, windows: boolean): Snippet[] {
  const quoted = /\s/.test(exe) ? `"${exe}"` : exe
  const list = [
    { id: "claude", label: "Claude Code", hint: "Run in a terminal.", text: `claude mcp add oah -- ${quoted} mcp` },
    { id: "codex", label: "Codex", hint: "Add to ~/.codex/config.toml.", text: `[mcp_servers.oah]\ncommand = '${exe}'\nargs = ["mcp"]` },
    { id: "antigravity", label: "Antigravity", hint: "Run in a terminal.", text: `agy mcp add oah -- ${quoted} mcp` },
    { id: "json", label: "Other clients", hint: "The usual mcpServers JSON.", text: JSON.stringify({ mcpServers: { oah: { command: exe, args: ["mcp"] } } }, null, 2) },
  ]
  if (windows) {
    const wsl = wslPath(exe)
    list.push({ id: "wsl", label: "Claude Code in WSL", hint: "Runs oah.exe through WSL interop; it reaches this window directly.", text: `claude mcp add oah -- ${/\s/.test(wsl) ? `"${wsl}"` : wsl} mcp` })
  }
  return list
}

/** Ready-to-paste client configs for MCP over HTTP: the URL and the bearer token. */
export function httpSnippets(url: string, token: string): Snippet[] {
  const auth = `Bearer ${token}`
  return [
    { id: "claude", label: "Claude Code", hint: "Run in a terminal.", text: `claude mcp add --transport http oah ${url} --header "Authorization: ${auth}"` },
    { id: "codex", label: "Codex", hint: "Add to ~/.codex/config.toml.", text: `[mcp_servers.oah]\nurl = "${url}"\nhttp_headers = { Authorization = "${auth}" }` },
    { id: "opencode", label: "OpenCode", hint: "Add to opencode.json.", text: JSON.stringify({ mcp: { oah: { type: "remote", url, headers: { Authorization: auth } } } }, null, 2) },
    { id: "antigravity", label: "Antigravity", hint: "Run in a terminal.", text: `agy mcp add --header "Authorization: ${auth}" oah ${url}` },
    { id: "json", label: "Other clients", hint: "The usual mcpServers JSON.", text: JSON.stringify({ mcpServers: { oah: { type: "http", url, headers: { Authorization: auth } } } }, null, 2) },
  ]
}

/** The token as shown: its prefix and last four, the rest hidden. */
export function maskToken(token: string): string {
  return token.length > 12 ? `${token.slice(0, 4)}${"•".repeat(12)}${token.slice(-4)}` : "•".repeat(token.length)
}

export function McpSection() {
  const mcp = useApp((s) => s.settings!.mcp)
  const platform = useApp((s) => s.info?.platform)
  const update = useApp((s) => s.update)
  const [status, setStatus] = useState<McpStatus | null>(null)
  const [token, setToken] = useState<string | null>(null)
  const [transport, setTransport] = useState<"stdio" | "http">(mcp.http ? "http" : "stdio")

  useEffect(() => {
    let live = true
    const poll = () => ipc.mcpStatus().then((s) => live && setStatus(s)).catch(() => {})
    poll()
    const t = setInterval(poll, 3000)
    return () => {
      live = false
      clearInterval(t)
    }
  }, [])
  // The token is made the first time HTTP is on, not before.
  useEffect(() => {
    if (mcp.http && token === null) ipc.mcpToken().then(setToken).catch(() => {})
  }, [mcp.http, token])

  const clients = status?.clients ?? 0
  const http = status?.http
  const url = http?.url ?? `http://127.0.0.1:${mcp.httpPort}/mcp`
  const shown = transport === "http" && mcp.http ? (token ? httpSnippets(url, token) : []) : status ? snippets(status.exe, platform === "windows") : []
  return (
    <Section
      title="MCP"
      description="Let another agent (Claude Code, Codex, OpenCode, Antigravity, any MCP client) list your projects, start chats, send messages, answer approvals and pick models. Its chats show in the sidebar as they run, and you can answer or type in them too."
    >
      <Group title="Server">
        <Row label="Allow MCP clients" description="Off: oah refuses every client. On: clients on this computer connect by running `oah mcp`, or over HTTP when that's on.">
          <Switch checked={mcp.enabled} onCheckedChange={(v) => update((s) => void (s.mcp.enabled = v))} />
        </Row>
        <Row
          label={
            <span className="flex items-center gap-2">
              <Dot tone={!status ? "err" : !mcp.enabled ? "off" : clients > 0 ? "ok" : "warn"} />
              {!status ? "Not listening" : !mcp.enabled ? "Off" : clients > 0 ? `${clients} client${clients === 1 ? "" : "s"} connected` : "Ready, no clients connected"}
            </span>
          }
          description={status ? <span className="font-mono text-[11px] break-all">{status.socket}</span> : "The socket couldn't be opened; see the app's log."}
        />
      </Group>
      <HttpGroup status={status} token={token} onToken={setToken} />
      <Group title="What clients may do">
        <Row label="Full access" description="Let a client run chats that never ask before commands and edits. Off: it can use Ask, Accept edits or Plan.">
          <Switch checked={mcp.allowFullAccess} onCheckedChange={(v) => update((s) => void (s.mcp.allowFullAccess = v))} />
        </Row>
        <Row label="Delete chats" description="Off: clients can close a chat's agent but not delete the chat.">
          <Switch checked={mcp.allowDelete} onCheckedChange={(v) => update((s) => void (s.mcp.allowDelete = v))} />
        </Row>
      </Group>
      {status ? (
        <Group
          title="Connect a client"
          aside={
            mcp.http ? (
              <Segmented
                value={transport}
                options={[
                  { value: "stdio", label: "oah mcp" },
                  { value: "http", label: "HTTP" },
                ]}
                onChange={setTransport}
              />
            ) : null
          }
        >
          {shown.map((s) => (
            <SnippetRow key={s.id} label={s.label} hint={s.hint} text={s.text} secret={transport === "http" && mcp.http ? (token ?? undefined) : undefined} />
          ))}
        </Group>
      ) : null}
    </Section>
  )
}

function HttpGroup({ status, token, onToken }: { status: McpStatus | null; token: string | null; onToken: (t: string) => void }) {
  const mcp = useApp((s) => s.settings!.mcp)
  const update = useApp((s) => s.update)
  const [port, setPort] = useState(String(mcp.httpPort))
  const [reveal, setReveal] = useState(false)
  const [copied, setCopied] = useState(false)
  const http = status?.http
  useEffect(() => setPort(String(mcp.httpPort)), [mcp.httpPort])

  const savePort = () => {
    const n = Number(port)
    if (!Number.isInteger(n) || n < 1024 || n > 65535) {
      toast.error("Pick a port from 1024 to 65535.")
      setPort(String(mcp.httpPort))
      return
    }
    if (n !== mcp.httpPort) update((s) => void (s.mcp.httpPort = n))
  }
  const state = !mcp.http ? "off" : !mcp.enabled ? "waiting" : http?.url ? "on" : http?.error ? "error" : "starting"
  return (
    <Group title="HTTP">
      <Row
        label="Serve over HTTP"
        description="Clients on this computer connect to a URL with a token instead of running `oah mcp`. Nothing outside this computer can reach it."
      >
        <Switch checked={mcp.http} onCheckedChange={(v) => update((s) => void (s.mcp.http = v))} />
      </Row>
      {mcp.http ? (
        <>
          <Row
            label={
              <span className="flex items-center gap-2">
                <Dot tone={state === "on" ? (http!.sessions > 0 ? "ok" : "warn") : state === "error" ? "err" : "off"} />
                {state === "on"
                  ? http!.sessions > 0
                    ? `${http!.sessions} session${http!.sessions === 1 ? "" : "s"}`
                    : "Listening, no sessions yet"
                  : state === "waiting"
                    ? "Waiting for Allow MCP clients"
                    : state === "error"
                      ? "Not listening"
                      : "Starting…"}
              </span>
            }
            description={state === "error" ? <span className="text-red">{http!.error}</span> : <span className="font-mono text-[11px] break-all">{http?.url ?? `http://127.0.0.1:${mcp.httpPort}/mcp`}</span>}
          />
          <Row label="Port" description="On 127.0.0.1 only.">
            <Input
              type="number"
              min={1024}
              max={65535}
              className="h-7 w-24 text-right font-mono text-[12px]"
              value={port}
              onChange={(e) => setPort(e.target.value)}
              onBlur={savePort}
              onKeyDown={(e) => e.key === "Enter" && savePort()}
              aria-label="HTTP port"
            />
          </Row>
          <Row
            label="Token"
            description={
              <>
                <span className="font-mono text-[11.5px] break-all select-text">{token ? (reveal ? token : maskToken(token)) : "…"}</span>
                <span className="mt-0.5 block">
                  {http && !http.tokenKept
                    ? "The keychain couldn't keep it, so it changes when oah restarts."
                    : "Clients send it as `Authorization: Bearer …`. It's kept in your keychain."}
                </span>
              </>
            }
          >
            <Button variant="ghost" size="icon-xs" className="text-muted-foreground" aria-label={reveal ? "Hide token" : "Show token"} disabled={!token} onClick={() => setReveal(!reveal)}>
              {reveal ? <EyeOff className="size-3.5" /> : <Eye className="size-3.5" />}
            </Button>
            <Button
              variant="ghost"
              size="xs"
              className="text-muted-foreground"
              disabled={!token}
              onClick={() => {
                copyText(token!)
                setCopied(true)
                setTimeout(() => setCopied(false), 1500)
              }}
            >
              {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
              {copied ? "Copied" : "Copy"}
            </Button>
            <Button
              variant="outline"
              size="xs"
              onClick={() =>
                ipc.mcpTokenRegenerate().then((t) => {
                  onToken(t)
                  toast.success("New token. Clients using the old one are disconnected; update them with the new one.")
                }, (e) => toast.error(String(e)))
              }
            >
              <RefreshCw className="size-3.5" /> Regenerate
            </Button>
          </Row>
          <Activity items={http?.activity ?? []} />
        </>
      ) : null}
    </Group>
  )
}

const KIND: Record<McpActivity["kind"], string> = { connected: "Connected", closed: "Ended", call: "Tool", refused: "Refused" }

/** What happened over HTTP lately, newest first. */
function Activity({ items }: { items: McpActivity[] }) {
  const [all, setAll] = useState(false)
  if (!items.length) return <div className="px-4 py-3 text-[12px] text-faint">No activity yet.</div>
  const shown = all ? items : items.slice(0, 6)
  return (
    <div className="px-4 py-3">
      <div className="mb-1.5 text-[12px] font-medium text-muted-foreground">Recent activity</div>
      <ul className="flex flex-col gap-1">
        {shown.map((a, i) => (
          <li key={`${a.at}-${i}`} className="flex items-baseline gap-2 text-[12px]">
            <span className="w-14 shrink-0 font-mono text-[11px] text-faint">{new Date(a.at).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })}</span>
            <span className={a.kind === "refused" ? "w-16 shrink-0 text-red" : "w-16 shrink-0 text-muted-foreground"}>{KIND[a.kind]}</span>
            <span className="min-w-0 truncate">{a.detail}</span>
            {a.count > 1 ? <span className="shrink-0 font-mono text-[11px] text-faint">×{a.count}</span> : null}
          </li>
        ))}
      </ul>
      {items.length > 6 ? (
        <button className="mt-1.5 text-[11.5px] text-muted-foreground hover:text-foreground" onClick={() => setAll(!all)}>
          {all ? "Show less" : `Show all ${items.length}`}
        </button>
      ) : null}
    </div>
  )
}

/** A snippet to copy. With `secret`, it's shown masked and copied whole. */
function SnippetRow({ label, hint, text, secret }: { label: string; hint: string; text: string; secret?: string }) {
  const [copied, setCopied] = useState(false)
  return (
    <div className="px-4 py-3">
      <div className="mb-1.5 flex items-center gap-2">
        <div className="text-[13px] font-medium">{label}</div>
        <div className="text-[12px] text-muted-foreground">{hint}</div>
        <Button
          variant="ghost"
          size="xs"
          className="ml-auto text-muted-foreground"
          onClick={() => {
            copyText(text)
            setCopied(true)
            setTimeout(() => setCopied(false), 1500)
          }}
        >
          {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
          {copied ? "Copied" : "Copy"}
        </Button>
      </div>
      <pre className="overflow-x-auto rounded-lg border bg-background px-3 py-2 font-mono text-[11.5px] leading-relaxed whitespace-pre">{secret ? text.split(secret).join(maskToken(secret)) : text}</pre>
    </div>
  )
}
