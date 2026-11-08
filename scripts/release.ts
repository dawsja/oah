#!/usr/bin/env bun
// Cut a release of only the apps that changed.
//
//   bun scripts/release.ts <version> [--desktop] [--android] [--push] [--dry-run]
//
// The desktop app and the phone app share one version number (app/package.json, the Cargo
// workspace), but each has its own tag, and each tag builds only that app:
//
//   desktop-v<version>  → .github/workflows/release.yml  (installers, the updater's latest.json)
//   android-v<version>  → .github/workflows/android.yml  (the APK)
//
// With no --desktop/--android, this compares HEAD with each app's last tag and tags only the
// apps whose shipped code changed (it prints why). Version bumps alone don't count, nor do tests,
// docs or CI. Then it bumps the version, commits "oah <version>", tags, and (with --push) pushes
// main and the tags. An app skipped this time simply skips that version number.
//
// Old releases were plain v<version> tags, which built both; they still count as the last
// release of each app.

import { $ } from "bun"
import { existsSync, readFileSync, statSync, writeFileSync } from "node:fs"
import { dirname, join, normalize } from "node:path"

type App = "desktop" | "android"
const APPS: App[] = ["desktop", "android"]

const root = (await $`git rev-parse --show-toplevel`.text()).trim()
process.chdir(root)

// ---- Arguments -------------------------------------------------------------------------------

const args = process.argv.slice(2)
const version = args.find((a) => !a.startsWith("--"))
const flag = (f: string) => args.includes(f)
const forced = APPS.filter((a) => flag(`--${a}`))
const push = flag("--push")
const dryRun = flag("--dry-run")
const unknown = args.filter((a) => a.startsWith("--") && !["--desktop", "--android", "--push", "--dry-run"].includes(a))
if (unknown.length || (!version && !dryRun)) {
  console.error("usage: bun scripts/release.ts <version> [--desktop] [--android] [--push] [--dry-run]")
  process.exit(2)
}
if (version && !/^\d+\.\d+\.\d+$/.test(version)) fail(`"${version}" isn't a version like 0.3.4`)

function fail(msg: string): never {
  console.error(`release: ${msg}`)
  process.exit(1)
}

// ---- What changed for each app ---------------------------------------------------------------

/** The newest tag on HEAD's history that released `app` (its own tag, or an old v* one). */
async function lastTag(app: App): Promise<string | null> {
  const patterns = ["--format=%(refname:short)", `refs/tags/${app}-v*`, "refs/tags/v*"]
  const out = await $`git for-each-ref --merged HEAD --sort=-creatordate ${patterns}`.text()
  return out.split("\n").find(Boolean) ?? null
}

/** Files under app/src reachable from an HTML entry's module script (static and dynamic imports). */
function reachable(entryHtml: string): Set<string> {
  const html = readFileSync(entryHtml, "utf8")
  const src = html.match(/<script[^>]+type="module"[^>]+src="\/([^"]+)"/)?.[1]
  if (!src) fail(`no module script in ${entryHtml}`)
  const seen = new Set<string>()
  const queue = [join("app", src)]
  const resolve = (from: string, spec: string): string | null => {
    let base: string
    if (spec.startsWith("@/")) base = join("app/src", spec.slice(2))
    else if (spec.startsWith(".")) base = normalize(join(dirname(from), spec))
    else return null // a package
    const exts = ["", ".ts", ".tsx", ".js", ".jsx", ".css", "/index.ts", "/index.tsx"]
    return exts.map((e) => base + e).find((f) => existsSync(f) && statSync(f).isFile()) ?? null
  }
  while (queue.length) {
    const f = queue.pop()!
    if (seen.has(f)) continue
    seen.add(f)
    const text = readFileSync(f, "utf8")
    for (const m of text.matchAll(/(?:from\s+|import\s*\(\s*|import\s+|@import\s+)["']([^"']+)["']/g)) {
      const r = resolve(f, m[1])
      if (r && !seen.has(r)) queue.push(r)
    }
  }
  return seen
}

const desktopFrontend = reachable("app/index.html")
const mobileFrontend = reachable("app/mobile.html")

const NOTHING: App[] = []
const BOTH: App[] = ["desktop", "android"]

/** Lines a diff adds or removes, minus the version bumps every release makes. */
async function realChanges(tag: string, file: string): Promise<string[]> {
  const diff = await $`git diff ${tag}..HEAD -- ${file}`.text()
  return diff
    .split("\n")
    .filter((l) => /^[+-]/.test(l) && !/^(\+\+\+|---)/.test(l))
    .filter((l) => !/^[+-]\s*("version":\s*"[^"]*",?|version = "[^"]*")\s*$/.test(l))
}

/** Which apps a changed file ships in. */
async function appsFor(file: string, tag: string): Promise<App[]> {
  // The phone's native voice build runs these through android/.cargo/config.toml.
  if (/^scripts\/android-/.test(file)) return ["android"]
  // Never shipped: docs, tests, CI and the release pipeline itself.
  if (/(^|\/)(README|CHANGELOG|LICENSE)[^/]*$|\.md$/.test(file)) return NOTHING
  if (/^\.github\/|^scripts\/|^rustfmt\.toml$|^\.gitignore$|^assets\//.test(file)) return NOTHING
  if (/(^|\/)tests?\/|\.test\.tsx?$|^app\/vitest\.config\.ts$/.test(file)) return NOTHING

  // Version bumps alone don't count.
  if (file === "Cargo.toml" || file === "app/package.json" || file === "Cargo.lock") {
    const lines = await realChanges(tag, file)
    if (file === "Cargo.lock") {
      // A workspace crate's version line changes with no checksum line: that's just the bump.
      const other = lines.filter((l) => !/^[+-]version = "/.test(l))
      return other.length ? BOTH : NOTHING
    }
    return lines.length ? BOTH : NOTHING
  }

  if (file.startsWith("android/")) return ["android"]
  if (file === "app/mobile.html" || file === "app/vite.mobile.config.ts") return ["android"]
  if (file === "app/index.html" || file === "app/vite.config.ts") return ["desktop"]
  if (file.startsWith("app/src-tauri/") || file.startsWith("app/scripts/") || file.startsWith("app/public/")) return ["desktop"]
  if (file === "app/bun.lock" || file === "app/tsconfig.json" || file === "app/components.json") return BOTH

  if (file.startsWith("app/src/")) {
    if (!existsSync(file)) return BOTH // deleted: can't tell who imported it
    const apps: App[] = []
    if (desktopFrontend.has(file)) apps.push("desktop")
    if (mobileFrontend.has(file)) apps.push("android")
    return apps
  }

  // The phone shares voice and core paths, and uses the client half of oah-remote.
  if (file.startsWith("crates/oah-voice/") || file.startsWith("crates/oah-core/")) return BOTH
  if (file.startsWith("crates/oah-remote/")) {
    if (file === "crates/oah-remote/src/client.rs") return ["android"]
    if (file === "crates/oah-remote/src/server.rs") return ["desktop"]
    return BOTH
  }
  if (file.startsWith("crates/")) return ["desktop"]

  console.warn(`  (don't know who ships ${file}; counting it for both)`)
  return BOTH
}

// ---- Decide ----------------------------------------------------------------------------------

const status = (await $`git status --porcelain`.text()).trim()
if (status && !dryRun) fail(`the working tree isn't clean:\n${status}`)
const branch = (await $`git rev-parse --abbrev-ref HEAD`.text()).trim()
if (branch !== "main" && !dryRun) fail(`releases are cut from main (on ${branch})`)

const release: App[] = []
for (const app of APPS) {
  const tag = await lastTag(app)
  if (!tag) {
    console.log(`${app}: never released`)
    if (!forced.length || forced.includes(app)) release.push(app)
    continue
  }
  const files = (await $`git diff --name-only ${tag}..HEAD`.text()).split("\n").filter(Boolean)
  const why: string[] = []
  for (const f of files) if ((await appsFor(f, tag)).includes(app)) why.push(f)
  console.log(`${app}: last released at ${tag}; ${why.length ? `${why.length} shipped file(s) changed` : "nothing shipped changed"}`)
  for (const f of why) console.log(`    ${f}`)
  if (forced.length ? forced.includes(app) : why.length > 0) release.push(app)
}

if (!release.length) fail("nothing to release (pass --desktop or --android to release anyway)")
const tags = release.map((a) => `${a}-v${version ?? "<version>"}`)
console.log(`\nreleasing: ${tags.join(", ")}`)
if (dryRun) process.exit(0)

// ---- Bump, commit, tag, push -----------------------------------------------------------------

for (const t of tags) {
  if ((await $`git tag -l ${t}`.text()).trim()) fail(`${t} already exists`)
}

const cargoToml = readFileSync("Cargo.toml", "utf8")
const bumped = cargoToml.replace(/(\[workspace\.package\][^[]*?\nversion = ")[^"]+"/, `$1${version}"`)
if (bumped === cargoToml) fail("couldn't find [workspace.package] version in Cargo.toml")
writeFileSync("Cargo.toml", bumped)
const pkg = readFileSync("app/package.json", "utf8")
writeFileSync("app/package.json", pkg.replace(/"version": "[^"]+"/, `"version": "${version}"`))

const cargo = existsSync(join(process.env.HOME ?? "", ".cargo/bin/cargo")) ? join(process.env.HOME!, ".cargo/bin/cargo") : "cargo"
await $`${cargo} update -w --quiet`
await $`git add Cargo.toml Cargo.lock app/package.json`
await $`git commit -q -m ${`oah ${version}`} -m ${`Releases: ${release.join(", ")}.`}`
for (const t of tags) await $`git tag -a ${t} -m ${`oah ${t.replace(/-v/, " ")}`}`
console.log(`committed oah ${version}, tagged ${tags.join(", ")}`)

if (push) {
  await $`git push origin main`
  for (const t of tags) await $`git push origin ${t}`
  console.log("pushed")
} else {
  console.log(`push with: git push origin main ${tags.join(" ")}`)
}
