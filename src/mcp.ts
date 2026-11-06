// MCP servers via @ai-sdk/mcp (stdio and Streamable HTTP). Configured in
// ~/.config/nur/mcp.json, ./.nur/mcp.json or ./.mcp.json:
//   { "mcpServers": { "fs": { "command": "npx", "args": ["-y", "…"] },
//                     "docs": { "url": "https://…/mcp", "headers": { … } } } }
// Tools are exposed as `mcp__<server>__<tool>`.
import { createMCPClient, type MCPClient } from "@ai-sdk/mcp"
import { StdioMCPTransport } from "@ai-sdk/mcp/mcp-stdio"
import type { ToolSet } from "ai"
import { join } from "node:path"
import { paths, readJson, shortPath, VERSION } from "./config"

interface ServerConfig {
  command?: string
  args?: string[]
  env?: Record<string, string>
  cwd?: string
  url?: string
  type?: "http" | "sse"
  headers?: Record<string, string>
  disabled?: boolean
}

export interface McpServer {
  name: string
  source: string
  state: "starting" | "ready" | "failed"
  error?: string
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

  start() {
    const files = [paths.mcp, join(this.cwd, ".nur", "mcp.json"), join(this.cwd, ".mcp.json")]
    const configs = new Map<string, [ServerConfig, string]>()
    for (const f of files) {
      const raw = readJson<any>(f, null)
      const servers = raw?.mcpServers ?? raw?.servers ?? {}
      for (const [name, cfg] of Object.entries<ServerConfig>(servers)) configs.set(name, [cfg, shortPath(f)])
    }
    for (const [name, [cfg, source]] of configs) {
      if (cfg.disabled) continue
      const server: McpServer = { name, source, state: "starting", tools: [] }
      this.servers.push(server)
      this.connect(server, cfg).then(this.onChange, this.onChange)
    }
  }

  private async connect(server: McpServer, cfg: ServerConfig) {
    try {
      const transport = cfg.url
        ? { type: cfg.type ?? ("http" as const), url: cfg.url, headers: cfg.headers }
        : new StdioMCPTransport({ command: cfg.command!, args: cfg.args ?? [], env: { ...(process.env as Record<string, string>), ...(cfg.env ?? {}) }, cwd: cfg.cwd ?? this.cwd, stderr: "ignore" })
      if (!cfg.url && !cfg.command) throw new Error('missing "command" or "url"')
      const client = await createMCPClient({ transport, clientName: "nur", version: VERSION })
      const tools = (await client.tools()) as ToolSet
      const exposed: ToolSet = {}
      for (const [name, t] of Object.entries(tools)) exposed[exposedName(server.name, name)] = t
      server.client = client
      server.toolset = exposed
      server.tools = Object.keys(tools)
      server.state = "ready"
    } catch (e: any) {
      server.state = "failed"
      server.error = e?.message ?? String(e)
    }
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
