import { expect, test } from "bun:test"
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { Snapshots } from "../src/checkpoints"

test("restore puts edited, deleted and created files back, leaving ignored files and .git alone", async () => {
  const prevHome = process.env.OAH_HOME
  process.env.OAH_HOME = mkdtempSync(join(tmpdir(), "oah-home-"))
  const dir = mkdtempSync(join(tmpdir(), "oah-proj-"))
  try {
    writeFileSync(join(dir, ".gitignore"), "build/\n")
    writeFileSync(join(dir, "a.txt"), "original a\n")
    mkdirSync(join(dir, "src"))
    writeFileSync(join(dir, "src", "b.ts"), "export const b = 1\n")
    writeFileSync(join(dir, "gone.txt"), "will be deleted\n")
    mkdirSync(join(dir, ".git"))
    writeFileSync(join(dir, ".git", "HEAD"), "ref: refs/heads/main\n")

    const snaps = new Snapshots(dir, "s1")
    const tree = await snaps.snapshot()
    expect(tree).toMatch(/^[0-9a-f]{40}$/)

    // The "AI" changes things.
    writeFileSync(join(dir, "a.txt"), "changed a\n")
    writeFileSync(join(dir, "src", "new.ts"), "new file\n")
    rmSync(join(dir, "gone.txt"))
    mkdirSync(join(dir, "build"))
    writeFileSync(join(dir, "build", "out.js"), "ignored output\n")
    writeFileSync(join(dir, ".git", "HEAD"), "ref: refs/heads/other\n")

    const after = await snaps.snapshot()
    expect((await snaps.diff(tree!, after!)).sort()).toEqual(["a.txt", "gone.txt", "src/new.ts"])

    const touched = await snaps.restore(tree!)
    expect(touched.sort()).toEqual(["a.txt", "gone.txt", "src/new.ts"])
    expect(readFileSync(join(dir, "a.txt"), "utf8")).toBe("original a\n")
    expect(readFileSync(join(dir, "gone.txt"), "utf8")).toBe("will be deleted\n")
    expect(existsSync(join(dir, "src", "new.ts"))).toBe(false)
    expect(readFileSync(join(dir, "src", "b.ts"), "utf8")).toBe("export const b = 1\n")
    // Ignored files and the project's own .git are never touched.
    expect(readFileSync(join(dir, "build", "out.js"), "utf8")).toBe("ignored output\n")
    expect(readFileSync(join(dir, ".git", "HEAD"), "utf8")).toBe("ref: refs/heads/other\n")
  } finally {
    rmSync(dir, { recursive: true, force: true })
    if (prevHome === undefined) delete process.env.OAH_HOME
    else process.env.OAH_HOME = prevHome
  }
})
