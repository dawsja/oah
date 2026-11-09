import { describe, expect, it } from "vitest"
import { baseName, inTerminal, locationKey, parseShellOsc, selectionBlock, shellLabel, terminalTheme } from "./terminal"

describe("terminal helpers", () => {
  it("reads the shell a remote start script names", () => {
    expect(parseShellOsc("shell=bash")).toBe("bash")
    expect(parseShellOsc("shell=zsh\n")).toBe("zsh")
    expect(parseShellOsc("shell=")).toBeNull()
    expect(parseShellOsc("shell=a;b")).toBeNull()
    expect(parseShellOsc("title=x")).toBeNull()
  })

  it("names shells and folders", () => {
    expect(shellLabel("pwsh")).toBe("PowerShell")
    expect(shellLabel("powershell")).toBe("Windows PowerShell")
    expect(shellLabel("bash")).toBe("bash")
    expect(shellLabel(null)).toBe("Terminal")
    expect(baseName("/home/u/code/oah-wt/fix-it/")).toBe("fix-it")
    expect(baseName("C:\\Users\\u\\oah")).toBe("oah")
  })

  it("keys a shell by the system it runs on", () => {
    expect(locationKey({ kind: "local" })).toBe("local")
    expect(locationKey({ kind: "wsl", distro: "Ubuntu" })).toBe("wsl:Ubuntu")
    expect(locationKey({ kind: "ssh", host: "devbox" })).toBe("ssh:devbox")
  })

  it("leaves non-elements alone", () => {
    expect(inTerminal(null)).toBe(false)
    expect(inTerminal({ closest: (sel: string) => (sel === ".xterm" ? {} : null) } as unknown as EventTarget)).toBe(true)
    expect(inTerminal({ closest: () => null } as unknown as EventTarget)).toBe(false)
  })

  it("themes from oah's colors", () => {
    const t = terminalTheme(true, "#0e0e10", "#ececef")
    expect(t.background).toBe("#0e0e10")
    expect(t.cursor).toBe("#ececef")
    expect(terminalTheme(false, "#eef0f3", "#16181d").white).not.toBe("#ffffff")
  })
})

describe("selectionBlock", () => {
  it("fences terminal output, tidied", () => {
    expect(selectionBlock("\r\n$ cargo test   \r\ntest a ... FAILED\r\n\r\n")).toBe("```\n$ cargo test\ntest a ... FAILED\n```\n")
  })
  it("fences past backticks inside", () => {
    expect(selectionBlock("a ```b``` c")).toBe("````\na ```b``` c\n````\n")
  })
  it("has nothing for whitespace", () => {
    expect(selectionBlock("  \n \r\n")).toBeNull()
  })
})
