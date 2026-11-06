import { expect, test } from "bun:test"
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { assertFresh, fileVersion, rememberVersion } from "../src/tools/versions"
import { BoundedText } from "../src/tools/text"

test("file-to-directory and disappearance versions are explicit, never thrown", () => {
  const path = join(mkdtempSync(join(tmpdir(), "oah-versions-")), "source")
  const versions = new Map<string, string | null>()
  writeFileSync(path, "text")
  rememberVersion(path, versions)
  rmSync(path)
  mkdirSync(path)
  expect(fileVersion(path)).toBe("!non-file")
  expect(() => assertFresh(path, versions)).toThrow("changed on disk")
  rmSync(path, { recursive: true })
  expect(fileVersion(path)).toBeNull()
  rememberVersion(path, versions)
  expect(() => assertFresh(path, versions)).not.toThrow()
})
test("bounded output retains exact small text and honest large-stream count", () => {
  const small = new BoundedText(1000)
  small.append("a".repeat(600)); small.append("b".repeat(200))
  expect(small.toString()).toBe("a".repeat(600) + "b".repeat(200))
  small.append("c".repeat(4000))
  expect(small.toString().length).toBeLessThanOrEqual(1000)
  expect(small.toString()).toContain("4800 characters total")
  expect(small.toString()).toStartWith("aaa")
  expect(small.toString()).toEndWith("ccc")
})
