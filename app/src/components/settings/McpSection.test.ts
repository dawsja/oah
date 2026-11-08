import { describe, expect, it } from "vitest"
import { snippets, wslPath } from "./McpSection"

describe("MCP client snippets", () => {
  it("maps a Windows path into WSL", () => {
    expect(wslPath("C:\\Users\\me\\AppData\\Local\\oah\\oah.exe")).toBe("/mnt/c/Users/me/AppData/Local/oah/oah.exe")
    expect(wslPath("/usr/bin/oah")).toBe("/usr/bin/oah")
  })

  it("runs `<exe> mcp` in every client", () => {
    const s = snippets("C:\\Program Files\\oah\\oah.exe", true)
    expect(s.map((x) => x.id)).toEqual(["claude", "codex", "json", "wsl"])
    expect(s[0].text).toBe('claude mcp add oah -- "C:\\Program Files\\oah\\oah.exe" mcp')
    expect(s[1].text).toContain("command = 'C:\\Program Files\\oah\\oah.exe'")
    expect(JSON.parse(s[2].text).mcpServers.oah).toEqual({ command: "C:\\Program Files\\oah\\oah.exe", args: ["mcp"] })
    expect(s[3].text).toBe('claude mcp add oah -- "/mnt/c/Program Files/oah/oah.exe" mcp')
    expect(snippets("/usr/bin/oah", false).map((x) => x.id)).toEqual(["claude", "codex", "json"])
  })
})
