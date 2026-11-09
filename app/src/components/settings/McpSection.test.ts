import { describe, expect, it } from "vitest"
import { httpSnippets, maskToken, snippets, wslPath } from "./McpSection"

describe("MCP client snippets", () => {
  it("maps a Windows path into WSL", () => {
    expect(wslPath("C:\\Users\\me\\AppData\\Local\\oah\\oah.exe")).toBe("/mnt/c/Users/me/AppData/Local/oah/oah.exe")
    expect(wslPath("/usr/bin/oah")).toBe("/usr/bin/oah")
  })

  it("runs `<exe> mcp` in every client", () => {
    const s = snippets("C:\\Program Files\\oah\\oah.exe", true)
    expect(s.map((x) => x.id)).toEqual(["claude", "codex", "antigravity", "grok", "json", "wsl"])
    expect(s[0].text).toBe('claude mcp add oah -- "C:\\Program Files\\oah\\oah.exe" mcp')
    expect(s[1].text).toContain("command = 'C:\\Program Files\\oah\\oah.exe'")
    expect(s[2].text).toBe('agy mcp add oah -- "C:\\Program Files\\oah\\oah.exe" mcp')
    expect(s[3].text).toBe('grok mcp add oah -- "C:\\Program Files\\oah\\oah.exe" mcp')
    expect(JSON.parse(s[4].text).mcpServers.oah).toEqual({ command: "C:\\Program Files\\oah\\oah.exe", args: ["mcp"] })
    expect(s[5].text).toBe('claude mcp add oah -- "/mnt/c/Program Files/oah/oah.exe" mcp')
    expect(snippets("/usr/bin/oah", false).map((x) => x.id)).toEqual(["claude", "codex", "antigravity", "grok", "json"])
  })

  it("points HTTP clients at the URL with the token", () => {
    const url = "http://127.0.0.1:7421/mcp"
    const s = httpSnippets(url, "oah_abc")
    expect(s.map((x) => x.id)).toEqual(["claude", "codex", "opencode", "antigravity", "grok", "json"])
    expect(s[0].text).toBe('claude mcp add --transport http oah http://127.0.0.1:7421/mcp --header "Authorization: Bearer oah_abc"')
    expect(s[1].text).toBe('[mcp_servers.oah]\nurl = "http://127.0.0.1:7421/mcp"\nhttp_headers = { Authorization = "Bearer oah_abc" }')
    expect(JSON.parse(s[2].text).mcp.oah).toEqual({ type: "remote", url, headers: { Authorization: "Bearer oah_abc" } })
    // agy wants its flags before the name.
    expect(s[3].text).toBe('agy mcp add --header "Authorization: Bearer oah_abc" oah http://127.0.0.1:7421/mcp')
    expect(s[4].text).toBe('grok mcp add --header "Authorization: Bearer oah_abc" oah http://127.0.0.1:7421/mcp')
    expect(JSON.parse(s[5].text).mcpServers.oah).toEqual({ type: "http", url, headers: { Authorization: "Bearer oah_abc" } })
  })

  it("hides the token but its prefix and last four", () => {
    expect(maskToken("oah_0123456789abcdef0123456789abcdef")).toBe("oah_••••••••••••cdef")
    expect(maskToken("short")).toBe("•••••")
  })
})
