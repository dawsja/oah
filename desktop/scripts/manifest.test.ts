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
  for (const r of ["linux-x64.AppImage", "linux-x64.deb", "linux-x64.rpm", "windows-x64-setup.exe", "windows-x64.msi", "darwin-arm64.app.tar.gz", "darwin-x64.app.tar.gz"]) put(r)
  put("darwin-arm64.dmg", false)
  writeFileSync(join(dir, "oah-desktop-0.1.0-linux-x64.deb"), "old") // another version: ignored

  const m = buildManifest(dir, v, "https://dl.oah.dev", "notes", new Date("2026-11-06T12:00:00.123Z"))
  expect(m.version).toBe(v)
  expect(m.pub_date).toBe("2026-11-06T12:00:00Z") // RFC 3339, as the updater parses it
  expect(Object.keys(m.platforms).sort()).toEqual([
    "darwin-aarch64", "darwin-aarch64-app", "darwin-x86_64", "darwin-x86_64-app",
    "linux-x86_64", "linux-x86_64-appimage", "linux-x86_64-deb", "linux-x86_64-rpm",
    "windows-x86_64", "windows-x86_64-msi", "windows-x86_64-nsis",
  ])
  expect(m.platforms["linux-x86_64-deb"]).toEqual({ url: `https://dl.oah.dev/desktop/v${v}/oah-desktop-${v}-linux-x64.deb`, signature: "sig-of-linux-x64.deb" })
  expect(m.platforms["linux-x86_64"]!.url).toEndWith(".AppImage")
  expect(m.platforms["windows-x86_64"]!.url).toEndWith("-setup.exe")
  // The dmg is a download, not an update.
  expect(m.files["darwin-arm64-dmg"]).toEqual({ path: `desktop/v${v}/oah-desktop-${v}-darwin-arm64.dmg`, url: `https://dl.oah.dev/desktop/v${v}/oah-desktop-${v}-darwin-arm64.dmg`, sha256: "abc123", size: 10 })
  expect(m.files["windows-x64-nsis"]!.path).toBe(`desktop/v${v}/oah-desktop-${v}-windows-x64-setup.exe`)
  expect(Object.keys(m.files)).not.toContain("linux-x64-deb-0.1.0")
})
