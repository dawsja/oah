import { useEffect, useState } from "react"
import { Check, Copy } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { copyText } from "@/lib/clipboard"
import { ipc } from "@/lib/ipc"
import type { McpStatus } from "@/lib/types"
import { useApp } from "@/store/app"
import { Dot, Group, Row, Section } from "./layout"

/** `C:\Users\me\oah.exe` as WSL sees it: `/mnt/c/Users/me/oah.exe`. */
export function wslPath(p: string): string {
  const m = /^([A-Za-z]):[\\/](.*)$/.exec(p)
  return m ? `/mnt/${m[1].toLowerCase()}/${m[2].replace(/\\/g, "/")}` : p.replace(/\\/g, "/")
}

/** Ready-to-paste client configs that run `<exe> mcp`. */
export function snippets(exe: string, windows: boolean): { id: string; label: string; hint: string; text: string }[] {
  const quoted = /\s/.test(exe) ? `"${exe}"` : exe
  const list = [
    { id: "claude", label: "Claude Code", hint: "Run in a terminal.", text: `claude mcp add oah -- ${quoted} mcp` },
    { id: "codex", label: "Codex", hint: "Add to ~/.codex/config.toml.", text: `[mcp_servers.oah]\ncommand = '${exe}'\nargs = ["mcp"]` },
    { id: "json", label: "Other clients", hint: "The usual mcpServers JSON.", text: JSON.stringify({ mcpServers: { oah: { command: exe, args: ["mcp"] } } }, null, 2) },
  ]
  if (windows) {
    const wsl = wslPath(exe)
    list.push({ id: "wsl", label: "Claude Code in WSL", hint: "Runs oah.exe through WSL interop; it reaches this window directly.", text: `claude mcp add oah -- ${/\s/.test(wsl) ? `"${wsl}"` : wsl} mcp` })
  }
  return list
}

export function McpSection() {
  const mcp = useApp((s) => s.settings!.mcp)
  const platform = useApp((s) => s.info?.platform)
  const update = useApp((s) => s.update)
  const [status, setStatus] = useState<McpStatus | null>(null)

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

  const clients = status?.clients ?? 0
  return (
    <Section
      title="MCP"
      description="Let another agent (Claude Code, Codex, any MCP client) list your projects, start chats, send messages, answer approvals and pick models. Its chats show in the sidebar as they run, and you can answer or type in them too."
    >
      <Group title="Server">
        <Row label="Allow MCP clients" description="Off: oah refuses every client. On: clients that run `oah mcp` on this computer connect.">
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
      <Group title="What clients may do">
        <Row label="Full access" description="Let a client run chats that never ask before commands and edits. Off: it can use Ask, Accept edits or Plan.">
          <Switch checked={mcp.allowFullAccess} onCheckedChange={(v) => update((s) => void (s.mcp.allowFullAccess = v))} />
        </Row>
        <Row label="Delete chats" description="Off: clients can close a chat's agent but not delete the chat.">
          <Switch checked={mcp.allowDelete} onCheckedChange={(v) => update((s) => void (s.mcp.allowDelete = v))} />
        </Row>
      </Group>
      {status ? (
        <Group title="Connect a client">
          {snippets(status.exe, platform === "windows").map((s) => (
            <Snippet key={s.id} label={s.label} hint={s.hint} text={s.text} />
          ))}
        </Group>
      ) : null}
    </Section>
  )
}

function Snippet({ label, hint, text }: { label: string; hint: string; text: string }) {
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
      <pre className="overflow-x-auto rounded-md border bg-background px-3 py-2 font-mono text-[11.5px] leading-relaxed whitespace-pre">{text}</pre>
    </div>
  )
}
