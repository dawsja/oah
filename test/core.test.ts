import { describe, expect, test } from "bun:test"
import { grantKey, needsPermission } from "../src/agent"
import { clampEffort, contextWindow, effortsFor } from "../src/catalog"
import { joinModel, nextPermission, splitModel } from "../src/config"
import { adaptiveThinking, providerOptions } from "../src/providers"
import { parseFrontMatter } from "../src/skills"
import { diffLines, globToRegex, isSafeCommand, permissionPreview, risk, stripAnsi } from "../src/tools"

describe("models", () => {
  test("provider/model ids", () => {
    expect(splitModel("anthropic/claude-opus-5-5")).toEqual({ provider: "anthropic", slug: "claude-opus-5-5" })
    expect(splitModel("claude-sonnet-5-5").provider).toBe("anthropic")
    expect(splitModel("gpt-5.6").provider).toBe("chatgpt")
    expect(joinModel("openai", "gpt-5.6")).toBe("openai/gpt-5.6")
  })
  test("catalog lookups", () => {
    expect(contextWindow("anthropic/claude-opus-5-5")).toBeGreaterThan(100_000)
    expect(effortsFor("anthropic/claude-opus-5-5")).toContain("max")
    expect(clampEffort("anthropic/claude-opus-5-5", "minimal")).toBe("low")
  })
  test("anthropic options: adaptive thinking, effort, fallbacks", () => {
    expect(adaptiveThinking("claude-opus-5-5")).toBe(true)
    expect(adaptiveThinking("claude-opus-4-6")).toBe(true)
    expect(adaptiveThinking("claude-haiku-4-5")).toBe(false)
    const o = providerOptions("anthropic/claude-opus-5-5", "high").anthropic
    expect(o.thinking).toEqual({ type: "adaptive", display: "summarized" })
    expect(o.effort).toBe("high")
    expect(o.fallbacks).toBe("default")
  })
  test("openai options: stateless with encrypted reasoning", () => {
    const o = providerOptions("chatgpt/gpt-5.6", "medium").openai
    expect(o.store).toBe(false)
    expect(o.reasoningSummary).toBe("auto")
    expect(o.include).toContain("reasoning.encrypted_content")
  })
})

describe("permissions", () => {
  test("matrix", () => {
    expect(needsPermission("auto", "read")).toBe(false)
    expect(needsPermission("auto", "safe-exec")).toBe(false)
    expect(needsPermission("auto", "write")).toBe(true)
    expect(needsPermission("auto", "exec")).toBe(true)
    expect(needsPermission("ask", "read")).toBe(true)
    expect(needsPermission("ask", "internal")).toBe(false)
    expect(needsPermission("allow", "exec")).toBe(false)
    expect(nextPermission("auto")).toBe("ask")
    expect(nextPermission("allow")).toBe("auto")
  })
  test("grant keys and risk", () => {
    expect(grantKey("bash", { command: "cargo test --all" })).toBe("bash:cargo")
    expect(grantKey("edit_file", {})).toBe("write")
    expect(risk("read_file", { path: "/etc/passwd" }, "/home/x", [])).toBe("read-outside")
    expect(risk("read_file", { path: "src/a.ts" }, "/home/x", [])).toBe("read")
  })
  test("safe commands", () => {
    expect(isSafeCommand("ls -la")).toBe(true)
    expect(isSafeCommand("git status && git diff")).toBe(true)
    expect(isSafeCommand("cat a | grep x | wc -l")).toBe(true)
    expect(isSafeCommand("rm -rf /")).toBe(false)
    expect(isSafeCommand("echo hi > file")).toBe(false)
    expect(isSafeCommand("find . -delete")).toBe(false)
    expect(isSafeCommand("git push")).toBe(false)
    expect(isSafeCommand("npm test", ["npm test"])).toBe(true)
  })
  test("previews", () => {
    expect(permissionPreview("edit_file", { old_string: "a\nb", new_string: "a\nB" })).toEqual(["-b", "+B"])
  })
})

describe("helpers", () => {
  test("diff, glob, ansi, front matter", () => {
    expect(diffLines("a\nb\nc", "a\nB\nc", 10)).toEqual(["-b", "+B"])
    expect(globToRegex("src/**/*.ts").test("src/ui/App.ts")).toBe(true)
    expect(globToRegex("*.{ts,tsx}").test("a.tsx")).toBe(true)
    expect(stripAnsi("\x1b[31mred\x1b[0m\r\n")).toBe("red\n")
    const { meta, body } = parseFrontMatter("---\nname: pdf\ndescription: >\n  folded\n  text\n---\n# Body")
    expect(meta).toEqual({ name: "pdf", description: "folded text" })
    expect(body.startsWith("# Body")).toBe(true)
  })
})
