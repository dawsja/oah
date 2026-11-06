import { afterAll, beforeAll, describe, expect, test } from "bun:test"
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { Memory } from "../src/memory"
import { Skills } from "../src/skills"
import { permissionPreview, TOOLS, type ToolCtx } from "../src/tools"
import { applyHunks, parsePatch } from "../src/tools/patch"
import { allShells, killAllShells, runningShells } from "../src/tools/shells"

let dir: string
let ctx: ToolCtx
const run = (name: string, args: any) => TOOLS.find((t) => t.name === name)!.run(args, ctx)
const read = (p: string) => readFileSync(join(dir, p), "utf8")

beforeAll(() => {
  dir = mkdtempSync(join(tmpdir(), "oah-edits-"))
  ctx = { cwd: dir, signal: new AbortController().signal, skills: new Skills(dir), memory: new Memory(dir), mode: "build", onTodos() {}, onGoalComplete() {} }
})
afterAll(() => {
  killAllShells()
  rmSync(dir, { recursive: true, force: true })
})

describe("edit_file", () => {
  test("several edits in one call", async () => {
    writeFileSync(join(dir, "a.ts"), "const a = 1\nconst b = 2\nconst c = 3\n")
    const o = await run("edit_file", { path: "a.ts", edits: [{ old_string: "a = 1", new_string: "a = 10" }, { old_string: "c = 3", new_string: "c = 30" }] })
    expect(o.ok).toBe(true)
    expect(read("a.ts")).toBe("const a = 10\nconst b = 2\nconst c = 30\n")
    expect(o.summary).toContain("2 edits")
  })
  test("all-or-nothing when one edit fails", async () => {
    writeFileSync(join(dir, "b.ts"), "x\ny\n")
    const o = await run("edit_file", { path: "b.ts", edits: [{ old_string: "x", new_string: "X" }, { old_string: "missing", new_string: "?" }] })
    expect(o.ok).toBe(false)
    expect(o.output).toContain("edit 2")
    expect(read("b.ts")).toBe("x\ny\n")
  })
  test("single edit still works; preview covers every edit", async () => {
    writeFileSync(join(dir, "c.ts"), "hello\n")
    expect((await run("edit_file", { path: "c.ts", old_string: "hello", new_string: "bye" })).ok).toBe(true)
    expect(read("c.ts")).toBe("bye\n")
    expect(permissionPreview("edit_file", { edits: [{ old_string: "a", new_string: "b" }, { old_string: "c", new_string: "d" }] })).toEqual(["-a", "+b", "-c", "+d"])
  })
})

describe("apply_patch", () => {
  test("parses add / update / move / delete", () => {
    const ops = parsePatch("*** Begin Patch\n*** Add File: n.txt\n+hi\n*** Update File: a.ts\n*** Move to: b.ts\n@@ fn\n-x\n+y\n*** Delete File: c.ts\n*** End Patch")
    expect(ops.map((o) => o.kind)).toEqual(["add", "update", "delete"])
    expect(ops[1]).toMatchObject({ moveTo: "b.ts", hunks: [{ anchor: "fn", lines: ["-x", "+y"] }] })
  })
  test("hunks tolerate trailing whitespace and use anchors", () => {
    const src = "function a() {\n  return 1\n}\nfunction b() {\n  return 1\n}\n"
    // The anchor skips the first identical "return 1"; the removed line has trailing spaces in the patch.
    const out = applyHunks(src, [{ anchor: "function b() {", lines: ["-  return 1   ", "+  return 2"] }], "f")
    expect(out).toBe("function a() {\n  return 1\n}\nfunction b() {\n  return 2\n}\n")
  })
  test("applies a multi-file patch", async () => {
    writeFileSync(join(dir, "p1.ts"), "one\ntwo\nthree\n")
    writeFileSync(join(dir, "gone.ts"), "bye\n")
    const patch = ["*** Begin Patch", "*** Update File: p1.ts", " one", "-two", "+TWO", " three", "*** Add File: sub/new.ts", "+fresh", "*** Delete File: gone.ts", "*** End Patch"].join("\n")
    const o = await run("apply_patch", { patch })
    expect(o.ok).toBe(true)
    expect(read("p1.ts")).toBe("one\nTWO\nthree\n")
    expect(read("sub/new.ts")).toBe("fresh\n")
    expect(existsSync(join(dir, "gone.ts"))).toBe(false)
    expect(o.summary).toBe("3 files · +2 -2")
  })
  test("changes nothing if any hunk fails", async () => {
    writeFileSync(join(dir, "p2.ts"), "keep\n")
    const patch = ["*** Begin Patch", "*** Add File: should-not-exist.ts", "+x", "*** Update File: p2.ts", "-not here", "+y", "*** End Patch"].join("\n")
    const o = await run("apply_patch", { patch })
    expect(o.ok).toBe(false)
    expect(o.output).toContain("no files changed")
    expect(existsSync(join(dir, "should-not-exist.ts"))).toBe(false)
    expect(read("p2.ts")).toBe("keep\n")
  })
})

describe("background shells", () => {
  test("start, read incrementally, filter, kill", async () => {
    const o = await run("bash", { command: "for i in 1 2 3; do echo line$i; sleep 0.2; done; echo ready; sleep 30", background: true })
    expect(o.ok).toBe(true)
    const id = /background (bg\d+)/.exec(o.summary)![1]!
    expect(runningShells().some((s) => s.id === id)).toBe(true)
    await Bun.sleep(1000)
    const r1 = await run("bash_output", { id })
    expect(r1.output).toContain("ready")
    expect(r1.output).toContain("running")
    const r2 = await run("bash_output", { id })
    expect(r2.output).toContain("(no new output)")
    expect((await run("bash_kill", { id })).summary).toBe(`stopped ${id}`)
    await Bun.sleep(300)
    expect(runningShells().some((s) => s.id === id)).toBe(false)
    expect((await run("bash_output", { id })).output).toContain("exited")
  })
  test("unknown ids fail clearly", async () => {
    expect((await run("bash_output", { id: "bg999" })).ok).toBe(false)
    expect(allShells().length).toBeGreaterThan(0)
  })
})
