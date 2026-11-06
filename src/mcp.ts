// MCP servers via @ai-sdk/mcp (stdio and Streamable HTTP). Configured in
// ~/.config/oah/mcp.json, ./.oah/mcp.json or ./.mcp.json:
//   { "mcpServers": { "fs": { "command": "npx", "args": ["-y", "…"] },
//                     "docs": { "url": "https://…/mcp", "headers": { … } } } }
// Tools are exposed as `mcp__<server>__<tool>`.
import { createMCPClient, type MCPClient } from "@ai-sdk/mcp"
import { StdioMCPTransport } from "@ai-sdk/mcp/mcp-stdio"
import type { ToolSet } from "ai"
import { isAbsolute, join } from "node:path"
import { paths, readJson, shortPath, VERSION, writeFileAtomic } from "./config"
import { canonicalPath } from "./tools/versions"
import { forget, hasTokens, McpOAuth, signIn } from "./mcp-auth"

interface ServerConfig {
  command?: string
  args?: string[]
  env?: Record<string, string>
  cwd?: string
  url?: string
  type?: "http" | "sse"
  headers?: Record<string, string>
  disabled?: boolean
  untrustedProject?: boolean
}

export interface McpServer {
  name: string
  source: string
  state: "starting" | "ready" | "failed" | "needs-auth"
  error?: string
  /** Remote (http/sse) server: may use OAuth. */
  remote: boolean
  /** Has saved OAuth tokens. */
  signedIn?: boolean
  tools: string[]
  client?: MCPClient
  toolset?: ToolSet
}

export const sanitize = (s: string) => s.replace(/[^A-Za-z0-9_-]/g, "_")
export const exposedName = (server: string, tool: string) => `mcp__${sanitize(server)}__${sanitize(tool)}`.slice(0, 64)

export class Mcp {
  servers: McpServer[] = []

  constructor(
    private cwd: string,
    private onChange: () => void,
  ) {}

  private connecting: Promise<unknown>[] = []
  private configs = new Map<string, ServerConfig>()

  /** Wait for every server to connect or fail (at most `ms`). */
  settled(ms = 15_000): Promise<void> {
    return Promise.race([Promise.allSettled(this.connecting).then(() => {}), new Promise<void>((r) => setTimeout(r, ms))])
  }

  /** Config files, lowest precedence first (a later file's server wins). */
  files() {
    return [paths.mcp, join(this.cwd, ".oah", "mcp.json"), join(this.cwd, ".mcp.json")]
  }

  private readConfigs() {
    const configs = new Map<string, [ServerConfig, string]>()
    const trusted = readJson<{ trustedMcpProjects?: string[] }>(paths.settings, {}).trustedMcpProjects ?? []
    const projectTrusted = trusted.some((p) => isAbsolute(p) && canonicalPath(p) === canonicalPath(this.cwd))
    for (const f of this.files()) {
      const raw = readJson<any>(f, null)
      const servers = raw?.mcpServers ?? raw?.servers ?? {}
      for (const [name, cfg] of Object.entries<ServerConfig>(servers)) {
        const untrustedProject = f !== paths.mcp && !projectTrusted
        if (untrustedProject && configs.has(name)) continue // untrusted checkout cannot shadow a global server
        configs.set(name, [{ ...cfg, untrustedProject }, shortPath(f)])
      }
    }
    return configs
  }

  start() {
    void this.reload()
  }

  /** Re-read the config files: connect new servers, reconnect changed ones, drop removed ones. */
  async reload(): Promise<{ added: string[]; changed: string[]; removed: string[] }> {
    const next = this.readConfigs()
    const added: string[] = []
    const changed: string[] = []
    const removed: string[] = []
    for (const s of [...this.servers]) {
      const n = next.get(s.name)
      if (!n || n[0].disabled) {
        removed.push(s.name)
        this.servers.splice(this.servers.indexOf(s), 1)
        this.configs.delete(s.name)
        void s.client?.close().catch(() => {})
      }
    }
    const work: Promise<unknown>[] = []
    for (const [name, [cfg, source]] of next) {
      if (cfg.disabled) continue
      const prev = this.configs.get(name)
      if (prev && JSON.stringify(prev) === JSON.stringify(cfg)) continue
      this.configs.set(name, cfg)
      if (prev) {
        changed.push(name)
        this.get(name)!.source = source
        work.push(this.reconnect(name))
      } else {
        added.push(name)
        const server: McpServer = { name, source, state: "starting", tools: [], remote: !!cfg.url }
        this.servers.push(server)
        work.push(this.connect(server, cfg).then(this.onChange, this.onChange))
      }
    }
    this.connecting.push(...work)
    this.onChange()
    await Promise.allSettled(work)
    return { added, changed, removed }
  }

  get(name: string) {
    return this.servers.find((s) => s.name === name)
  }

  /** Reconnect one server (after a sign-in, or to retry a failure). */
  async reconnect(name: string) {
    const server = this.get(name)
    const cfg = this.configs.get(name)
    if (!server || !cfg) return
    await server.client?.close().catch(() => {})
    Object.assign(server, { state: "starting", error: undefined, tools: [], client: undefined, toolset: undefined })
    this.onChange()
    await this.connect(server, cfg)
    this.onChange()
  }

  /** Browser OAuth sign-in for a remote server, then reconnect it. */
  async signIn(name: string, o: { onUrl: (url: string) => void; signal: AbortSignal }) {
    const cfg = expandEnv(this.configs.get(name))
    if (!cfg?.url) throw new Error(`${name} isn't a remote (http) server`)
    await signIn(name, cfg.url, o)
    await this.reconnect(name)
  }

  /** Forget saved OAuth tokens for a server. */
  async signOut(name: string): Promise<boolean> {
    const cfg = expandEnv(this.configs.get(name))
    if (!cfg?.url || !forget(name, cfg.url)) return false
    await this.reconnect(name)
    return true
  }

  private async connect(server: McpServer, cfg: ServerConfig) {
    cfg = expandEnv(cfg)
    // Probe mode: a server that wants a sign-in is marked instead of opening a browser.
    const oauth = cfg.url ? new McpOAuth(server.name, cfg.url) : undefined
    let client: MCPClient | undefined
    try {
      if (cfg.untrustedProject) throw new Error(`Project MCP config not trusted. Add ${JSON.stringify(this.cwd)} to trustedMcpProjects in ${paths.settings} after reviewing the config.`)
      const transport = cfg.url
        ? { type: cfg.type ?? ("http" as const), url: cfg.url, headers: cfg.headers, authProvider: oauth }
        : new StdioMCPTransport({ command: cfg.command!, args: cfg.args ?? [], env: { ...(process.env as Record<string, string>), ...(cfg.env ?? {}) }, cwd: cfg.cwd ?? this.cwd, stderr: "ignore" })
      if (!cfg.url && !cfg.command) throw new Error('missing "command" or "url"')
      client = await createMCPClient({ transport, clientName: "oah", version: VERSION })
      const tools = (await client.tools()) as ToolSet
      const exposed: ToolSet = {}
      for (const [name, t] of Object.entries(tools)) {
        const key = exposedName(server.name, name)
        if (exposed[key] || this.servers.some((s) => s !== server && s.toolset?.[key])) throw new Error(`MCP tool name collision: ${key}`)
        exposed[key] = t
      }
      server.client = client
      server.toolset = exposed
      server.tools = Object.keys(tools)
      server.state = "ready"
    } catch (e: any) {
      await client?.close().catch(() => {})
      const msg = e?.message ?? String(e)
      if (oauth?.wantsAuth || e?.name === "UnauthorizedError" || /\b401\b|unauthorized/i.test(msg)) server.state = "needs-auth"
      else server.state = "failed"
      server.error = msg
    }
    server.signedIn = !!cfg.url && hasTokens(server.name, cfg.url)
  }

  tools(): ToolSet {
    return Object.assign({}, ...this.servers.filter((s) => s.state === "ready").map((s) => s.toolset))
  }

  readyNames(): string[] {
    return this.servers.filter((s) => s.state === "ready").map((s) => s.name)
  }

  async close() {
    await Promise.allSettled(this.servers.map((s) => s.client?.close()))
  }
}

/** `${VAR}` / `${VAR:-default}` in any string of a server config (Claude Code's syntax). */
function expandEnv<T>(v: T): T {
  if (typeof v === "string") return v.replace(/\$\{([A-Za-z_][A-Za-z0-9_]*)(?::-([^}]*))?\}/g, (_, k, d) => process.env[k] ?? d ?? "") as T
  if (Array.isArray(v)) return v.map(expandEnv) as T
  if (v && typeof v === "object") return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, expandEnv(x)])) as T
  return v
}

/** Add or replace a server in an mcp.json file (keeps the file's other servers). */
export function writeServer(file: string, name: string, cfg: ServerConfig | null) {
  const raw = readJson<any>(file, {}) ?? {}
  const key = raw.servers && !raw.mcpServers ? "servers" : "mcpServers"
  raw[key] ??= {}
  if (cfg) raw[key][name] = cfg
  else delete raw[key][name]
  writeFileAtomic(file, JSON.stringify(raw, null, 2) + "\n", true)
}

export type { ServerConfig }
