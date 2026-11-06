import { afterAll, beforeAll, describe, expect, test } from "bun:test"
import { mkdirSync, mkdtempSync, rmSync, utimesSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { Memory } from "../src/memory"
import { Skills } from "../src/skills"
import { parseAge, parseSize, TOOLS, type ToolCtx } from "../src/tools"

let dir: string
let ctx: ToolCtx
const run = (name: string, args: any) => TOOLS.find((t) => t.name === name)!.run(args, ctx)

beforeAll(() => {
  dir = mkdtempSync(join(tmpdir(), "nur-search-"))
  mkdirSync(join(dir, "src/deep/er"), { recursive: true })
  mkdirSync(join(dir, "build"))
  mkdirSync(join(dir, ".hidden"))
  writeFileSync(join(dir, ".gitignore"), "build/\n")
  writeFileSync(join(dir, "src/app.ts"), "const a = 1\nfunction hello() {}\n// TODO: fix\nconst b = 2\n")
  writeFileSync(join(dir, "src/app.test.ts"), "test('hello', () => {})\n")
  writeFileSync(join(dir, "src/deep/er/x.rs"), "fn hello() {}\n".repeat(200))
  writeFileSync(join(dir, "build/out.js"), "hello\n")
  writeFileSync(join(dir, ".hidden/secret.txt"), "hello\n")
  writeFileSync(join(dir, "README.md"), "Hello world\n")
  const old = new Date(Date.now() - 10 * 864e5)
  utimesSync(join(dir, "README.md"), old, old)
  ctx = { cwd: dir, signal: new AbortController().signal, skills: new Skills(dir), memory: new Memory(dir), mode: "build", onTodos() {}, onGoalComplete() {} }
})
afterAll(() => rmSync(dir, { recursive: true, force: true }))

describe("ls", () => {
  test("lists directories first, hides dotfiles by default", async () => {
    const o = await run("ls", {})
    expect(o.output.split("\n")).toEqual(["build/", "src/", "README.md"])
    const all = await run("ls", { all: true })
    expect(all.output).toContain(".gitignore")
    expect(all.output).toContain(".hidden/")
  })
  test("long and depth", async () => {
    const o = await run("ls", { path: "src", long: true, depth: 2 })
    expect(o.output).toMatch(/\d+B\s+\d{4}-\d\d-\d\d \d\d:\d\d\s+app\.ts/)
    expect(o.output).toContain("  er/")
  })
  test("missing directory fails", async () => {
    expect((await run("ls", { path: "nope" })).ok).toBe(false)
  })
})

describe("find", () => {
  test("by name, respecting .gitignore and hidden", async () => {
    const o = await run("find", { name: "*.ts" })
    expect(o.output.split("\n")).toEqual(["src/app.test.ts", "src/app.ts"])
    expect((await run("find", { name: "out.js" })).output).toBe("Nothing found.")
    expect((await run("find", { name: "out.js", no_ignore: true })).output).toBe("build/out.js")
    expect((await run("find", { name: "secret.txt" })).output).toBe("Nothing found.")
    expect((await run("find", { name: "secret.txt", hidden: true })).output).toBe(".hidden/secret.txt")
  })
  test("type, depth, size, age, path pattern", async () => {
    expect((await run("find", { type: "dir" })).output.split("\n")).toEqual(["src/", "src/deep/", "src/deep/er/"])
    expect((await run("find", { max_depth: 1 })).output.split("\n")).toEqual(["README.md", "src/"])
    expect((await run("find", { min_size: "1k" })).output).toBe("src/deep/er/x.rs")
    expect((await run("find", { type: "file", modified_within: "1d", name: "*.md" })).output).toBe("Nothing found.")
    expect((await run("find", { path_pattern: "src/**/*.rs" })).output).toBe("src/deep/er/x.rs")
    expect((await run("find", { min_size: "lots" })).ok).toBe(false)
  })
  test("limit", async () => {
    const o = await run("find", { type: "file", limit: 1 })
    expect(o.output).toContain("stopped at 1 results")
  })
  test("size and age parsing", () => {
    expect(parseSize("10k")).toBe(10240)
    expect(parseSize("2M")).toBe(2 * 1024 ** 2)
    expect(parseAge("2h")).toBe(2 * 36e5)
    expect(parseAge("soon")).toBeNull()
  })
})

for (const engine of ["ripgrep", "js"] as const) {
  describe(`grep (${engine})`, () => {
    beforeAll(() => {
      if (engine === "js") process.env.NUR_NO_RG = "1"
      else delete process.env.NUR_NO_RG
    })
    afterAll(() => delete process.env.NUR_NO_RG)
    test("content matches with line numbers, gitignore respected", async () => {
      const o = await run("grep", { pattern: "hello", glob: "*.ts" })
      expect(o.output.split("\n").sort()).toEqual(["src/app.test.ts:1: test('hello', () => {})", "src/app.ts:2: function hello() {}"])
      expect(o.output).not.toContain("build/")
    })
    test("case insensitive and fixed strings", async () => {
      expect((await run("grep", { pattern: "HELLO WORLD", case_insensitive: true })).output).toBe("README.md:1: Hello world")
      expect((await run("grep", { pattern: "hello()", fixed_strings: true, glob: "*.ts" })).output).toBe("src/app.ts:2: function hello() {}")
    })
    test("files and count modes", async () => {
      expect((await run("grep", { pattern: "hello", output: "files" })).output.split("\n").sort()).toEqual(["src/app.test.ts", "src/app.ts", "src/deep/er/x.rs"])
      expect((await run("grep", { pattern: "hello", output: "count", glob: "*.rs" })).output).toBe("src/deep/er/x.rs:200")
    })
    test("context lines", async () => {
      const o = await run("grep", { pattern: "TODO", context: 1, path: "src/app.ts" })
      expect(o.output.split("\n").map((l) => l.replace(/^src\/app\.ts/, ""))).toEqual(["-2- function hello() {}", ":3: // TODO: fix", "-4- const b = 2"])
    })
    test("limit and no matches", async () => {
      const o = await run("grep", { pattern: "hello", glob: "*.rs", limit: 5 })
      expect(o.output).toContain("195 more lines")
      expect((await run("grep", { pattern: "zzz-nothing" })).output).toBe("No matches.")
    })
  })
}
