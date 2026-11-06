import { expect, test } from "bun:test"
import { existsSync, mkdirSync, mkdtempSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { tmpdir } from "node:os"
import { Mcp } from "../src/mcp"
import { Memory } from "../src/memory"

test("instruction cache invalidates and new instruction files are discovered within total budget", () => {
  const home = mkdtempSync(join(tmpdir(), "oah-memory-home-")), root = mkdtempSync(join(tmpdir(), "oah-memory-repo-"))
  const old = process.env.OAH_HOME
  process.env.OAH_HOME = home
  try {
    mkdirSync(join(root, ".git")); mkdirSync(join(root, "sub"))
    writeFileSync(join(root, "AGENTS.md"), "root " + "x".repeat(40_000))
    const m = new Memory(join(root, "sub"))
    expect(m.promptSection().length).toBeLessThanOrEqual(16_000)
    writeFileSync(join(root, "sub", "AGENTS.md"), "local rule")
    expect(m.promptSection()).toContain("local rule")
    writeFileSync(join(root, "sub", "AGENTS.md"), "changed local rule")
    expect(m.promptSection()).toContain("changed local rule")
    m.add("remember this")
    expect(m.promptSection()).toContain("remember this")
    m.remove("remember this")
    expect(m.promptSection()).not.toContain("remember this")
  } finally { if (old === undefined) delete process.env.OAH_HOME; else process.env.OAH_HOME = old }
})
test("untrusted project MCP cannot execute or shadow global config", async () => {
  const home = mkdtempSync(join(tmpdir(), "oah-mcp-home-")), cwd = mkdtempSync(join(tmpdir(), "oah-mcp-repo-"))
  const old = process.env.OAH_HOME
  process.env.OAH_HOME = home
  const mcp = new Mcp(cwd, () => {})
  try {
    const marker = join(cwd, "executed")
    const cfg = { command: process.execPath, args: ["-e", `require('node:fs').writeFileSync(${JSON.stringify(marker)}, 'bad')`], untrustedProject: false }
    writeFileSync(join(cwd, ".mcp.json"), JSON.stringify({ mcpServers: { untrusted: cfg } }))
    await mcp.reload()
    expect(existsSync(marker)).toBe(false)
    expect(mcp.get("untrusted")?.error).toContain("not trusted")
    writeFileSync(join(home, "mcp.json"), JSON.stringify({ mcpServers: { untrusted: { url: "https://global.test/mcp" } } }))
    expect((mcp as any).readConfigs().get("untrusted")[0].url).toBe("https://global.test/mcp")
    writeFileSync(join(home, "settings.json"), JSON.stringify({ trustedMcpProjects: [cwd] }))
    expect((mcp as any).readConfigs().get("untrusted")[0].untrustedProject).toBe(false)
  } finally { await mcp.close(); if (old === undefined) delete process.env.OAH_HOME; else process.env.OAH_HOME = old }
})
