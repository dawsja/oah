import { expect, test } from "bun:test"
import { existsSync, mkdtempSync, readFileSync, symlinkSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { tmpdir } from "node:os"
import { buildTools, planDenied, promptParts, type TurnOptions } from "../src/agent"
import { defaultSettings } from "../src/config"
import { Memory } from "../src/memory"
import { Skills } from "../src/skills"
import { isSafeCommand, risk } from "../src/tools"
import { Verification } from "../src/verification"

function options(): TurnOptions {
  const cwd = mkdtempSync(join(tmpdir(), "oah-policy-"))
  return {
    model: "openai/gpt-5.6", effort: "low", permission: "allow", mode: "build", goalActive: true,
    instructions: "test", messages: [], settings: { ...defaultSettings },
    ctx: { cwd, skills: new Skills(cwd), memory: new Memory(cwd) },
    mcp: { tools: () => ({}) } as any, grants: new Set(), signal: new AbortController().signal,
    hooks: { text() {}, reasoning() {}, reasoningEnd() {}, toolStart() {}, toolEnd() {}, ask: async () => "deny", todos() {}, goalComplete() {}, usage() {}, spend() {}, toolProgress() {}, question: async () => "", beforeChange: async () => {} },
  }
}
async function call(tools: ReturnType<typeof buildTools>, name: string, args: any, id = name) {
  return (tools[name]!.execute as any)(args, { toolCallId: id, messages: [] }) as Promise<string>
}
test("plan schemas stay stable but mutations are denied even with allow/grants", async () => {
  const o = options()
  const build = buildTools(o)
  const plan = buildTools({ ...o, mode: "plan", grants: new Set(["write", "memory", "oah_setup"]) })
  expect(Object.keys(plan)).toEqual(Object.keys(build))
  expect(plan.bash!.description).toBe(build.bash!.description)
  for (const [name, args] of [
    ["write_file", { path: "no", content: "bad" }], ["edit_file", { path: "no", old_string: "a", new_string: "b" }],
    ["memory", { action: "add", text: "bad" }], ["oah_setup", { action: "reload" }], ["bash", { command: "env touch no" }],
  ] as const) expect(await call(plan, name, args)).toContain("Plan mode")
  expect(existsSync(join(o.ctx.cwd, "no"))).toBe(false)
  expect(planDenied("mcp__server__write", {})).toBeDefined()
  expect(planDenied("memory", { action: "list" })).toBeUndefined()
})
test("runtime changes do not rewrite the static system prefix", () => {
  const base = { cwd: "/repo", mode: "build" as const, skills: "one", memory: "old", mcp: ["a"] }
  const a = promptParts(base), b = promptParts({ ...base, mode: "plan", memory: "new", goal: "finish", skills: "two" })
  expect(a.system).toBe(b.system)
  expect(a.context).not.toBe(b.context)
})
test("conservative shell policy rejects mutation syntax and attached output flags", () => {
  for (const cmd of ["env touch no", "ls\ntouch no", "git tag -d old", "git remote set-url origin x", "git branch new", "cargo check", "sort -ofile input", "sort -o file input", "tree -ofile", "file -C -m magic", "uniq input output", "rg --pre=evil x", "find . -fprint out", "npm testevil"]) expect(isSafeCommand(cmd, ["npm test"])).toBe(false)
})
test("symlink targets outside the workspace are not local writes", () => {
  const o = options(), outside = mkdtempSync(join(tmpdir(), "oah-outside-"))
  symlinkSync(outside, join(o.ctx.cwd, "link"), "dir")
  expect(risk("write_file", { path: "link/new" }, o.ctx.cwd, [])).toBe("write-outside")
})
test("verification and mutations serialize through a delayed checkpoint", async () => {
  const o = options()
  let release!: () => void
  const wait = new Promise<void>((r) => { release = r })
  o.hooks.beforeChange = () => wait
  o.runtime = new Verification()
  const tools = buildTools(o)
  // Initialize a tiny git repo so diff --check can verify the written tree.
  await Bun.$`git -C ${o.ctx.cwd} init -q`.quiet()
  const write = call(tools, "write_file", { path: "a", content: "hello\n" })
  const check = call(tools, "bash", { command: "git diff --check" })
  await Bun.sleep(10)
  expect(o.runtime.state.status).toBe("unverified")
  expect(existsSync(join(o.ctx.cwd, "a"))).toBe(false)
  release()
  await Promise.all([write, check])
  expect(readFileSync(join(o.ctx.cwd, "a"), "utf8")).toBe("hello\n")
  expect(o.runtime.state.status).toBe("checks-passed")
})
test("goal completion requires post-mutation checks", async () => {
  const o = options()
  let done = false
  o.hooks.goalComplete = () => { done = true }
  const tools = buildTools(o)
  await call(tools, "write_file", { path: "a", content: "x" })
  expect(await call(tools, "goal_complete", { summary: "done" })).toContain("Completion refused")
  expect(done).toBe(false)
})

test("MCP mutations invalidate checks while declared read-only calls do not", async () => {
  const o = options()
  o.runtime = new Verification()
  o.runtime.checked("bun test", true, 0)
  o.mcp = { tools: () => ({ mcp__test__write: { execute: async () => ({ content: [{ type: "text", text: "changed" }] }) }, mcp__test__read: { metadata: { annotations: { readOnlyHint: true } }, execute: async () => ({ content: [{ type: "text", text: "read" }] }) } }) } as any
  const tools = buildTools(o)
  await call(tools, "mcp__test__read", {})
  expect(o.runtime.state.status).toBe("checks-passed")
  await call(tools, "mcp__test__write", {})
  expect(o.runtime.state.status).toBe("unverified")
  expect(await call(tools, "goal_complete", { summary: "done" })).toContain("Completion refused")
})
test("a batch verdict cannot authorize changed arguments", async () => {
  const o = options()
  o.permission = "auto"
  let executed = false
  o.mcp = { tools: () => ({ mcp__test__write: { execute: async () => { executed = true; return "bad" } } }) } as any
  o.reviewed = new Map([["id", { name: "mcp__test__write", args: "{}", intent: "", verdict: { allow: true, reason: "old arguments only" } }]])
  const result = await call(buildTools(o), "mcp__test__write", { value: "x".repeat(60_001) }, "id")
  expect(executed).toBe(false)
  expect(result).toContain("denied")
})

test("sensitive configuration and credentials require review, not ordinary local-write grants", () => {
  const o = options()
  for (const p of [".git/config", ".mcp.json", "AGENTS.md", ".env", ".oah/skills/custom/SKILL.md"]) {
    expect(risk("write_file", { path: p }, o.ctx.cwd, [])).toBe("protected")
  }
  expect(risk("read_file", { path: ".env.local" }, o.ctx.cwd, [])).toBe("protected")
  expect(isSafeCommand("cat .env")).toBe(false)
})

test("ordinary local-write grants do not approve protected writes", async () => {
  const o = options()
  o.permission = "ask"
  o.grants.add("write")
  expect(await call(buildTools(o), "write_file", { path: "AGENTS.md", content: "untrusted rules" })).toContain("denied")
  expect(existsSync(join(o.ctx.cwd, "AGENTS.md"))).toBe(false)
})
