// Build standalone `oah` binaries (Bun runtime + OpenTUI's native Zig library
// embedded) and release tarballs for install.sh:
//   bun scripts/build.ts                  → host target only
//   bun scripts/build.ts --all            → every supported target
//   bun scripts/build.ts --os darwin,windows  → those OSes (both architectures)
// Windows archives hold oah.exe (extract with the built-in tar.exe; see scripts/install.ps1).
// Output: dist/oah-<os>-<arch>.tar.gz (+ .sha256), dist/latest.txt and dist/latest.json
// (version, publish time, and each target's path/sha256/size — what `oah update` reads).
import { $ } from "bun"
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"

const { version, dependencies } = JSON.parse(readFileSync("package.json", "utf8"))
const OPENTUI_VERSION: string = dependencies["@opentui/core"]
const TARGETS = [
  { bun: "bun-linux-x64", name: "linux-x64", pkg: "@opentui/core-linux-x64" },
  { bun: "bun-linux-arm64", name: "linux-arm64", pkg: "@opentui/core-linux-arm64" },
  { bun: "bun-darwin-x64", name: "darwin-x64", pkg: "@opentui/core-darwin-x64" },
  { bun: "bun-darwin-arm64", name: "darwin-arm64", pkg: "@opentui/core-darwin-arm64" },
  { bun: "bun-windows-x64", name: "windows-x64", pkg: "@opentui/core-win32-x64" },
  { bun: "bun-windows-arm64", name: "windows-arm64", pkg: "@opentui/core-win32-arm64" },
]
const PLATFORM_PKGS = [
  "@opentui/core-linux-x64",
  "@opentui/core-linux-arm64",
  "@opentui/core-linux-x64-musl",
  "@opentui/core-linux-arm64-musl",
  "@opentui/core-darwin-x64",
  "@opentui/core-darwin-arm64",
  "@opentui/core-win32-x64",
  "@opentui/core-win32-arm64",
]
const host = `${process.platform === "darwin" ? "darwin" : process.platform === "win32" ? "windows" : "linux"}-${process.arch === "arm64" ? "arm64" : "x64"}`
const osArg = process.argv.indexOf("--os")
const oses = osArg > 0 ? (process.argv[osArg + 1] ?? "").split(",").map((s) => s.trim()) : null
const targets = process.argv.includes("--all") ? TARGETS : oses ? TARGETS.filter((t) => oses.includes(t.name.split("-")[0]!)) : TARGETS.filter((t) => t.name === host)
if (!targets.length) throw new Error(`no targets for ${oses ? oses.join(",") : host}`)

rmSync("dist", { recursive: true, force: true })
mkdirSync("dist", { recursive: true })
const files: Record<string, { path: string; sha256: string; size: number }> = {}
for (const t of targets) {
  // Each target needs its platform's prebuilt OpenTUI library (built with Zig) on disk.
  const pkgDir = `node_modules/${t.pkg}`
  if (!existsSync(pkgDir)) {
    const tmp = mkdtempSync(join(tmpdir(), "oah-"))
    await $`npm pack ${t.pkg}@${OPENTUI_VERSION} --pack-destination ${tmp}`.quiet()
    mkdirSync(pkgDir, { recursive: true })
    await $`tar -xzf ${tmp}/*.tgz -C ${pkgDir} --strip-components=1`
  }
  const dir = `dist/oah-${t.name}`
  mkdirSync(dir, { recursive: true })
  // Only this target's native library is embedded; other platforms stay external.
  const external = PLATFORM_PKGS.filter((p) => p !== t.pkg).flatMap((p) => ["--external", p])
  const exe = t.name.startsWith("windows") ? "oah.exe" : "oah"
  await $`bun build src/index.tsx --compile --minify --sourcemap=none --target=${t.bun} --define process.env.NODE_ENV='"production"' ${external} --outfile ${dir}/${exe}`
  await $`tar -C ${dir} -czf dist/oah-${t.name}.tar.gz ${exe}`
  const sum = (await $`shasum -a 256 dist/oah-${t.name}.tar.gz`.text()).split(" ")[0]
  writeFileSync(`dist/oah-${t.name}.tar.gz.sha256`, `${sum}\n`)
  files[t.name] = { path: `v${version}/oah-${t.name}.tar.gz`, sha256: sum!, size: statSync(`dist/oah-${t.name}.tar.gz`).size }
  console.log(`built dist/oah-${t.name}.tar.gz`)
}
writeFileSync("dist/latest.txt", `${version}\n`)
writeFileSync("dist/latest.json", JSON.stringify({ version, published: new Date().toISOString(), files }, null, 2) + "\n")
