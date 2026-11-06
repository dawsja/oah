import { expect, test } from "bun:test"
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { tmpdir } from "node:os"
import { applyPatch, replaceFile } from "../src/tools/patch"

function fixture() {
  const dir = mkdtempSync(join(tmpdir(), "oah-patch-"))
  writeFileSync(join(dir, "a"), "one\n")
  writeFileSync(join(dir, "b"), "keep\n")
  return { dir, run: (body: string, commit?: (p: string, v: string | null) => void) => applyPatch(`*** Begin Patch\n${body}\n*** End Patch`, (p) => join(dir, p), (p) => p, commit) }
}
test("repeated path updates compose", () => {
  const f = fixture()
  f.run("*** Update File: a\n-one\n+two\n*** Update File: a\n-two\n+three")
  expect(readFileSync(join(f.dir, "a"), "utf8")).toBe("three\n")
})
test("move to self and destination overwrite fail before writes", () => {
  const f = fixture()
  for (const dest of ["a", "b"]) expect(() => f.run(`*** Update File: a\n*** Move to: ${dest}\n-one\n+two`)).toThrow("no files changed")
  expect(readFileSync(join(f.dir, "a"), "utf8")).toBe("one\n")
  expect(readFileSync(join(f.dir, "b"), "utf8")).toBe("keep\n")
})
test("commit failure rolls back earlier writes", () => {
  const f = fixture()
  let n = 0
  expect(() => f.run("*** Update File: a\n-one\n+two\n*** Update File: b\n-keep\n+changed", (p, v) => {
    if (++n === 2) throw Error("injected I/O failure")
    replaceFile(p, v!)
  })).toThrow("rolled back")
  expect(readFileSync(join(f.dir, "a"), "utf8")).toBe("one\n")
  expect(readFileSync(join(f.dir, "b"), "utf8")).toBe("keep\n")
})
test("ambiguous hunks and missing anchors fail", () => {
  const f = fixture()
  writeFileSync(join(f.dir, "a"), "one\none\n")
  expect(() => f.run("*** Update File: a\n-one\n+two")).toThrow("ambiguous")
  expect(() => f.run("*** Update File: a\n@@ absent\n-one\n+two")).toThrow("anchor not found")
})

test("patches reject dangling symlinks and compose directory aliases", async () => {
  const { symlinkSync } = await import("node:fs")
  const f = fixture()
  symlinkSync("missing", join(f.dir, "dangling"))
  expect(() => f.run("*** Add File: dangling\n+not allowed")).toThrow("regular file")
  symlinkSync(f.dir, join(f.dir, "alias"), "dir")
  f.run("*** Update File: a\n-one\n+two\n*** Update File: alias/a\n-two\n+three")
  expect(readFileSync(join(f.dir, "a"), "utf8")).toBe("three\n")
})

test("rollback restores deleted binary assets byte-for-byte", async () => {
  const { rmSync } = await import("node:fs")
  const f = fixture()
  const bytes = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0, 255, 254, 10, 1])
  writeFileSync(join(f.dir, "image.png"), bytes)
  let n = 0
  expect(() => f.run("*** Delete File: image.png\n*** Update File: b\n-keep\n+changed", (p, value) => {
    if (++n === 2) throw Error("injected failure")
    if (value === null) rmSync(p); else replaceFile(p, value)
  })).toThrow("rolled back")
  expect(readFileSync(join(f.dir, "image.png"))).toEqual(bytes)
  expect(() => f.run("*** Update File: image.png\n-irrelevant\n+replacement")).toThrow("binary files")
})
