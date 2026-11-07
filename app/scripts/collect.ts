// After `tauri build`, gather one target's installers and update artifacts into
// app/release/ under stable names, each with a .sha256 (and the updater's
// .sig where Tauri made one):
//
//   oah-desktop-<version>-linux-x64.AppImage | .deb | .rpm
//   oah-desktop-<version>-windows-x64-setup.exe
//
//   bun scripts/collect.ts --target x86_64-unknown-linux-gnu
import { createHash } from "node:crypto"
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs"
import { join, resolve } from "node:path"

const desktop = resolve(import.meta.dir, "..")
const version: string = JSON.parse(readFileSync(join(desktop, "package.json"), "utf8")).version
const arg = process.argv.indexOf("--target")
const triple = arg > 0 ? process.argv[arg + 1]! : ""

const os = ["linux", "windows"].find((o) => triple.includes(o))
const arch = triple.startsWith("aarch64") ? "arm64" : triple.startsWith("x86_64") ? "x64" : ""
if (!os || !arch) throw new Error(`usage: bun scripts/collect.ts --target <rust target triple> (got '${triple}')`)

// tauri build --target <triple> writes to target/<triple>/release/bundle; without it, target/release/bundle.
// The Cargo workspace is the repo root, so bundles land in ../target.
const bundle = [join(desktop, "..", "target", triple, "release", "bundle"), join(desktop, "..", "target", "release", "bundle")].find(existsSync)
if (!bundle) throw new Error("no bundle folder — run tauri build first")

/** Bundle subfolder, file suffix, and its name in the release. */
const KINDS: [string, RegExp, string][] = [
  ["appimage", /\.AppImage$/, ".AppImage"],
  ["deb", /\.deb$/, ".deb"],
  ["rpm", /\.rpm$/, ".rpm"],
  ["nsis", /-setup\.exe$/, "-setup.exe"],
]

const out = join(desktop, "release")
mkdirSync(out, { recursive: true })
let n = 0
for (const [dir, match, suffix] of KINDS) {
  const d = join(bundle, dir)
  if (!existsSync(d)) continue
  const file = readdirSync(d).find((f) => match.test(f) && f.includes(version))
  if (!file) continue
  const name = `oah-desktop-${version}-${os}-${arch}${suffix}`
  copyFileSync(join(d, file), join(out, name))
  const sum = createHash("sha256").update(readFileSync(join(out, name))).digest("hex")
  writeFileSync(join(out, `${name}.sha256`), `${sum}  ${name}\n`)
  if (existsSync(join(d, `${file}.sig`))) copyFileSync(join(d, `${file}.sig`), join(out, `${name}.sig`))
  console.log(`  ${name}  ${(statSync(join(out, name)).size / 1048576).toFixed(1)} MB${existsSync(join(out, `${name}.sig`)) ? " (signed for the updater)" : ""}`)
  n++
}
if (!n) throw new Error(`no installers for ${version} in ${bundle}`)
