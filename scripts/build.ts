// Build standalone `nur` binaries (Bun runtime + OpenTUI's native Zig library
// embedded) and release tarballs for install.sh:
//   bun scripts/build.ts            → host target only
//   bun scripts/build.ts --all      → every supported target
// Output: dist/nur-<os>-<arch>.tar.gz (+ .sha256) and dist/latest.txt
import { $ } from "bun"
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"

const { version, dependencies } = JSON.parse(readFileSync("package.json", "utf8"))
const OPENTUI_VERSION: string = dependencies["@opentui/core"]
const TARGETS = [
  { bun: "bun-linux-x64", name: "linux-x64", pkg: "@opentui/core-linux-x64" },
  { bun: "bun-linux-arm64", name: "linux-arm64", pkg: "@opentui/core-linux-arm64" },
  { bun: "bun-darwin-x64", name: "darwin-x64", pkg: "@opentui/core-darwin-x64" },
  { bun: "bun-darwin-arm64", name: "darwin-arm64", pkg: "@opentui/core-darwin-arm64" },
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
const host = `${process.platform === "darwin" ? "darwin" : "linux"}-${process.arch === "arm64" ? "arm64" : "x64"}`
const targets = process.argv.includes("--all") ? TARGETS : TARGETS.filter((t) => t.name === host)

rmSync("dist", { recursive: true, force: true })
mkdirSync("dist", { recursive: true })
for (const t of targets) {
  // Each target needs its platform's prebuilt OpenTUI library (built with Zig) on disk.
  const pkgDir = `node_modules/${t.pkg}`
  if (!existsSync(pkgDir)) {
    const tmp = mkdtempSync(join(tmpdir(), "nur-"))
    await $`npm pack ${t.pkg}@${OPENTUI_VERSION} --pack-destination ${tmp}`.quiet()
    mkdirSync(pkgDir, { recursive: true })
    await $`tar -xzf ${tmp}/*.tgz -C ${pkgDir} --strip-components=1`
  }
  const dir = `dist/nur-${t.name}`
  mkdirSync(dir, { recursive: true })
  // Only this target's native library is embedded; other platforms stay external.
  const external = PLATFORM_PKGS.filter((p) => p !== t.pkg).flatMap((p) => ["--external", p])
  await $`bun build src/index.tsx --compile --minify --sourcemap=none --target=${t.bun} --define process.env.NODE_ENV='"production"' ${external} --outfile ${dir}/nur`
  await $`tar -C ${dir} -czf dist/nur-${t.name}.tar.gz nur`
  const sum = (await $`shasum -a 256 dist/nur-${t.name}.tar.gz`.text()).split(" ")[0]
  writeFileSync(`dist/nur-${t.name}.tar.gz.sha256`, `${sum}\n`)
  console.log(`built dist/nur-${t.name}.tar.gz`)
}
writeFileSync("dist/latest.txt", `${version}\n`)
