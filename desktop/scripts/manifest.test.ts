import { expect, test } from "bun:test"
import { mkdtempSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { buildManifest } from "./manifest"

test("every install type finds its own signed update; each OS has a fallback", () => {
  const dir = mkdtempSync(join(tmpdir(), "oah-manifest-"))
  const v = "0.2.0"
  const put = (rest: string, sig = true) => {
    const f = `oah-desktop-${v}-${rest}`
    writeFileSync(join(dir, f), "x".repeat(10))
    writeFileSync(join(dir, `${f}.sha256`), `abc123  ${f}\n`)
    if (sig) writeFileSync(join(dir, `${f}.sig`), `sig-of-${rest}\n`)
  }
  for (const r of ["linux-x64.AppImage", "linux-x64.deb", "linux-x64.rpm", "windows-x64-setup.exe"]) put(r)
  put("windows-x64.msi") // not built: ignored
  writeFileSync(join(dir, "oah-desktop-0.1.0-linux-x64.deb"), "old") // another version: ignored

  const m = buildManifest(dir, v, "https://dl.oah.dev", "notes", new Date("2026-11-06T12:00:00.123Z"))
  expect(m.version).toBe(v)
  expect(m.pub_date).toBe("2026-11-06T12:00:00Z") // RFC 3339, as the updater parses it
  expect(Object.keys(m.platforms).sort()).toEqual([
    "linux-x86_64", "linux-x86_64-appimage", "linux-x86_64-deb", "linux-x86_64-rpm",
    "windows-x86_64", "windows-x86_64-nsis",
  ])
  expect(m.platforms["linux-x86_64-deb"]).toEqual({ url: `https://dl.oah.dev/desktop/v${v}/oah-desktop-${v}-linux-x64.deb`, signature: "sig-of-linux-x64.deb" })
  expect(m.platforms["linux-x86_64"]!.url).toEndWith(".AppImage")
  expect(m.platforms["windows-x86_64"]!.url).toEndWith("-setup.exe")
  expect(m.files["linux-x64-deb"]).toEqual({ path: `desktop/v${v}/oah-desktop-${v}-linux-x64.deb`, url: `https://dl.oah.dev/desktop/v${v}/oah-desktop-${v}-linux-x64.deb`, sha256: "abc123", size: 10 })
  expect(Object.keys(m.files).sort()).toEqual(["linux-x64-appimage", "linux-x64-deb", "linux-x64-rpm", "windows-x64-nsis"])
  expect(m.files["windows-x64-nsis"]!.path).toBe(`desktop/v${v}/oah-desktop-${v}-windows-x64-setup.exe`)
  expect(Object.keys(m.files)).not.toContain("linux-x64-deb-0.1.0")
})
