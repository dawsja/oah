// Build the engine the desktop app bundles: the oah CLI compiled for one target,
// saved as src-tauri/binaries/oah-engine-<target triple>[.exe] (Tauri's sidecar
// naming; tauri.bundle.conf.json lists it). Same flags as ../scripts/build.ts.
//
//   bun scripts/engine.ts                          # this machine's target
//   bun scripts/engine.ts --target x86_64-pc-windows-msvc
import { $ } from "bun"
import { existsSync, mkdirSync, mkdtempSync, readFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"

const root = resolve(import.meta.dir, "../..")
const TARGETS: Record<string, { bun: string; pkg: string }> = {
  "x86_64-unknown-linux-gnu": { bun: "bun-linux-x64", pkg: "@opentui/core-linux-x64" },
  "aarch64-unknown-linux-gnu": { bun: "bun-linux-arm64", pkg: "@opentui/core-linux-arm64" },
  "x86_64-pc-windows-msvc": { bun: "bun-windows-x64", pkg: "@opentui/core-win32-x64" },
  "aarch64-pc-windows-msvc": { bun: "bun-windows-arm64", pkg: "@opentui/core-win32-arm64" },
}
const PLATFORM_PKGS = ["linux-x64", "linux-arm64", "linux-x64-musl", "linux-arm64-musl", "darwin-x64", "darwin-arm64", "win32-x64", "win32-arm64"].map((p) => `@opentui/core-${p}`)

const arg = process.argv.indexOf("--target")
const triple = arg > 0 ? process.argv[arg + 1]! : (await $`rustc -vV`.text()).match(/^host: (\S+)/m)?.[1]
const t = triple ? TARGETS[triple] : undefined
if (!triple || !t) throw new Error(`unsupported target ${triple ?? "(rustc not found)"} — one of: ${Object.keys(TARGETS).join(", ")}`)

// The target's prebuilt OpenTUI library must be on disk to be embedded.
const { dependencies } = JSON.parse(readFileSync(join(root, "package.json"), "utf8"))
const pkgDir = join(root, "node_modules", t.pkg)
if (!existsSync(pkgDir)) {
  const tmp = mkdtempSync(join(tmpdir(), "oah-engine-"))
  await $`npm pack ${t.pkg}@${dependencies["@opentui/core"]} --pack-destination ${tmp}`.quiet()
  mkdirSync(pkgDir, { recursive: true })
  await $`tar -xzf ${tmp}/*.tgz -C ${pkgDir} --strip-components=1`
}

const outDir = join(import.meta.dir, "../src-tauri/binaries")
mkdirSync(outDir, { recursive: true })
const out = join(outDir, `oah-engine-${triple}${triple.includes("windows") ? ".exe" : ""}`)
const external = PLATFORM_PKGS.filter((p) => p !== t.pkg).flatMap((p) => ["--external", p])
await $`bun build ${join(root, "src/index.tsx")} --compile --minify --sourcemap=none --target=${t.bun} --define process.env.NODE_ENV='"production"' ${external} --outfile ${out}`.cwd(root)
console.log(`engine → ${out}`)
