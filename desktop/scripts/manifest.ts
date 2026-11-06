// Write desktop/release/latest.json (and latest.txt) from the collected release
// files (scripts/collect.ts, one run per target). One manifest serves both:
//
//   platforms  the app's background updater (tauri-plugin-updater static format):
//              each install finds `{os}-{arch}-{installer}`, then `{os}-{arch}`.
//   files      the install scripts and download links: what to download per platform.
//
//   bun scripts/manifest.ts [--base https://dl.oah.dev] [--notes "…"]
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs"
import { join, resolve } from "node:path"

export interface Manifest {
  version: string
  notes: string
  pub_date: string
  platforms: Record<string, { url: string; signature: string }>
  files: Record<string, { path: string; url: string; sha256: string; size: number }>
}

/** Release suffix → updater installer name (null: not an update artifact). */
const UPDATE: [string, string | null][] = [
  [".AppImage", "appimage"],
  [".deb", "deb"],
  [".rpm", "rpm"],
  ["-setup.exe", "nsis"],
  [".msi", "msi"],
  [".app.tar.gz", "app"],
  [".dmg", null],
]
/** The update an install falls back to when its own kind isn't listed. */
const FALLBACK: Record<string, string> = { linux: "appimage", windows: "nsis", darwin: "app" }
const ARCH: Record<string, string> = { x64: "x86_64", arm64: "aarch64" }

export function buildManifest(dir: string, version: string, base: string, notes = "", now = new Date()): Manifest {
  const m: Manifest = { version, notes, pub_date: now.toISOString().replace(/\.\d{3}Z$/, "Z"), platforms: {}, files: {} }
  const prefix = `oah-desktop-${version}-`
  for (const f of readdirSync(dir).sort()) {
    if (!f.startsWith(prefix) || f.endsWith(".sig") || f.endsWith(".sha256")) continue
    const rest = f.slice(prefix.length) // e.g. linux-x64.deb, windows-x64-setup.exe
    const kind = UPDATE.find(([suffix]) => rest.endsWith(suffix))
    if (!kind) continue
    const [suffix, installer] = kind
    const [os, arch] = rest.slice(0, -suffix.length).split("-") as [string, string]
    const path = `desktop/v${version}/${f}`
    const url = `${base}/${path}`
    const sha = existsSync(join(dir, `${f}.sha256`)) ? readFileSync(join(dir, `${f}.sha256`), "utf8").split(/\s/)[0]! : ""
    // Download key, e.g. windows-x64-setup.exe → "windows-x64-nsis"; darwin-arm64.dmg → "darwin-arm64-dmg".
    m.files[`${os}-${arch}-${installer ?? suffix.slice(1)}`] = { path, url, sha256: sha, size: statSync(join(dir, f)).size }
    if (!installer || !existsSync(join(dir, `${f}.sig`))) continue
    const signature = readFileSync(join(dir, `${f}.sig`), "utf8").trim()
    const target = `${os}-${ARCH[arch] ?? arch}`
    m.platforms[`${target}-${installer}`] = { url, signature }
    if (FALLBACK[os] === installer) m.platforms[target] = { url, signature }
  }
  return m
}

if (import.meta.main) {
  const desktop = resolve(import.meta.dir, "..")
  const arg = (name: string) => {
    const i = process.argv.indexOf(name)
    return i > 0 ? process.argv[i + 1] : undefined
  }
  const version: string = JSON.parse(readFileSync(join(desktop, "package.json"), "utf8")).version
  const base = (arg("--base") ?? process.env.OAH_PUBLIC_URL ?? "https://dl.oah.dev").replace(/\/$/, "")
  const dir = join(desktop, "release")
  const m = buildManifest(dir, version, base, arg("--notes") ?? `oah desktop ${version}`)
  if (!Object.keys(m.files).length) throw new Error(`no oah-desktop-${version}-* files in ${dir} — run scripts/collect.ts first`)
  writeFileSync(join(dir, "latest.json"), JSON.stringify(m, null, 2) + "\n")
  writeFileSync(join(dir, "latest.txt"), `${version}\n`)
  console.log(`latest.json: ${Object.keys(m.files).length} downloads, updates for ${Object.keys(m.platforms).join(", ") || "nothing (no .sig files)"}`)
}
