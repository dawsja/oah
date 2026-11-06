// Self-update and uninstall for the compiled binary. Releases are described by
// <base>/latest.json (written by scripts/build.ts, uploaded by scripts/publish-r2.sh):
//   { "version": "0.2.0", "published": "…", "files": { "linux-x64": { "path": "v0.2.0/oah-linux-x64.tar.gz", "sha256": "…", "size": 123 } } }
import { chmodSync, existsSync, mkdtempSync, readFileSync, renameSync, rmSync, unlinkSync, writeFileSync } from "node:fs"
import { homedir, tmpdir } from "node:os"
import { basename, dirname, join } from "node:path"
import { homeDir, readJson, shortPath, VERSION, writeFileAtomic } from "./config"
import { isWindows } from "./platform"

export const DOWNLOAD_BASE = (process.env.OAH_DOWNLOAD_URL || "https://dl.oah.dev").replace(/\/+$/, "")

export interface ReleaseFile {
  path: string
  sha256: string
  size: number
}
export interface Release {
  version: string
  published?: string
  files: Record<string, ReleaseFile>
}

/** This machine's release target, e.g. linux-x64, darwin-arm64, windows-x64. */
export function target(): string {
  const os = process.platform === "darwin" ? "darwin" : isWindows ? "windows" : "linux"
  return `${os}-${process.arch === "arm64" ? "arm64" : "x64"}`
}

/** -1 / 0 / 1, numeric per dotted part ("0.10.0" > "0.9.3"). */
export function compareVersions(a: string, b: string): number {
  const pa = a.replace(/^v/, "").split(/[.-]/).map((x) => parseInt(x, 10) || 0)
  const pb = b.replace(/^v/, "").split(/[.-]/).map((x) => parseInt(x, 10) || 0)
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0)
    if (d) return Math.sign(d)
  }
  return 0
}

/** The latest release (or a specific version), from latest.json; falls back to latest.txt + .sha256 files. */
export async function fetchRelease(version?: string, signal?: AbortSignal): Promise<Release> {
  const get = (path: string) => fetch(`${DOWNLOAD_BASE}/${path}`, { signal: signal ?? AbortSignal.timeout(20_000), headers: { "User-Agent": `oah/${VERSION}` } })
  // GitHub releases serve the newest release's files from …/releases/latest/download/.
  const latest = (name: string) => (/github\.com\/[^/]+\/[^/]+\/releases\/download$/.test(DOWNLOAD_BASE) ? fetch(`${DOWNLOAD_BASE.replace(/\/download$/, "/latest/download")}/${name}`, { signal: signal ?? AbortSignal.timeout(20_000) }) : get(name))
  if (!version) {
    const r = await latest("latest.json").catch(() => null)
    if (r?.ok) {
      const rel = (await r.json()) as Release
      if (rel?.version && rel.files) return rel
    }
    const t = await latest("latest.txt")
    if (!t.ok) throw new Error(`couldn't reach ${DOWNLOAD_BASE} (HTTP ${t.status})`)
    version = (await t.text()).trim()
  }
  version = version.replace(/^v/, "")
  const file = `v${version}/oah-${target()}.tar.gz`
  const sum = await get(`${file}.sha256`)
  if (!sum.ok) throw new Error(sum.status === 404 ? `oah ${version} has no build for ${target()}` : `couldn't fetch ${file}.sha256 (HTTP ${sum.status})`)
  return { version, files: { [target()]: { path: file, sha256: (await sum.text()).trim().split(/\s+/)[0]!, size: 0 } } }
}

/** The running oah binary, or null when running from source (bun src/index.tsx). */
export function installedBinary(): string | null {
  const exe = process.execPath
  const name = basename(exe).toLowerCase()
  if (!name.startsWith("oah")) return null
  return exe
}

/** Remove the old binary left by a Windows self-update (it can't be deleted while running). */
export function cleanupOldBinary() {
  const exe = installedBinary()
  if (exe && existsSync(`${exe}.old`)) {
    try {
      unlinkSync(`${exe}.old`)
    } catch {}
  }
}

export type UpdateResult = { status: "updated"; from: string; to: string; path: string } | { status: "current"; version: string }

/** Download, verify and install a release over the running binary. */
export async function update(o: { version?: string; force?: boolean; onStatus?: (s: string) => void; signal?: AbortSignal } = {}): Promise<UpdateResult> {
  const say = o.onStatus ?? (() => {})
  const exe = installedBinary()
  if (!exe) throw new Error("oah is running from source — update with git pull instead")
  say("checking for updates")
  const rel = await fetchRelease(o.version, o.signal)
  if (!o.version && !o.force && compareVersions(rel.version, VERSION) <= 0) return { status: "current", version: VERSION }
  const file = rel.files[target()]
  if (!file) throw new Error(`oah ${rel.version} has no build for ${target()}`)

  const tmp = mkdtempSync(join(tmpdir(), "oah-update-"))
  try {
    say(`downloading oah ${rel.version}`)
    const r = await fetch(`${DOWNLOAD_BASE}/${file.path}`, { signal: o.signal, headers: { "User-Agent": `oah/${VERSION}` } })
    if (!r.ok) throw new Error(`download failed: HTTP ${r.status}`)
    const bytes = new Uint8Array(await r.arrayBuffer())
    const got = new Bun.CryptoHasher("sha256").update(bytes).digest("hex")
    if (got !== file.sha256.toLowerCase()) throw new Error(`checksum mismatch (expected ${file.sha256}, got ${got}) — nothing was changed`)
    const archive = join(tmp, basename(file.path))
    writeFileSync(archive, bytes)
    const tar = Bun.spawnSync(["tar", "-xzf", archive, "-C", tmp], { stderr: "pipe" })
    if (tar.exitCode !== 0) throw new Error(`couldn't extract the archive: ${tar.stderr.toString().trim()}`)
    const fresh = join(tmp, isWindows ? "oah.exe" : "oah")
    if (!existsSync(fresh)) throw new Error("the archive doesn't contain the oah binary")

    say("installing")
    // Stage next to the binary (same filesystem), then swap with renames.
    const staged = `${exe}.new`
    writeFileSync(staged, readFileSync(fresh))
    if (!isWindows) chmodSync(staged, 0o755)
    if (isWindows) {
      // A running .exe can be renamed but not overwritten.
      rmSync(`${exe}.old`, { force: true })
      renameSync(exe, `${exe}.old`)
      try {
        renameSync(staged, exe)
      } catch (e) {
        renameSync(`${exe}.old`, exe)
        throw e
      }
    } else renameSync(staged, exe)
    return { status: "updated", from: VERSION, to: rel.version, path: exe }
  } finally {
    rmSync(tmp, { recursive: true, force: true })
  }
}

// ---- background check (TUI) -------------------------------------------------------

const CHECK_EVERY = 24 * 60 * 60 * 1000
const checkFile = () => join(homeDir(), "update-check.json")

/** At most once a day: the newer version available, if any (cached between runs). */
export async function checkForUpdate(): Promise<string | null> {
  if (process.env.OAH_NO_UPDATE_CHECK || !installedBinary()) return null
  const cached = readJson<{ at: number; latest: string } | null>(checkFile(), null)
  let latest = cached?.latest
  if (!cached || Date.now() - cached.at > CHECK_EVERY) {
    try {
      latest = (await fetchRelease(undefined, AbortSignal.timeout(8000))).version
      writeFileAtomic(checkFile(), JSON.stringify({ at: Date.now(), latest }))
    } catch {
      return null
    }
  }
  return latest && compareVersions(latest, VERSION) > 0 ? latest : null
}

// ---- uninstall ---------------------------------------------------------------------

/** Shell profiles install.sh may have added a PATH line to. */
function profiles(): string[] {
  const h = homedir()
  return [join(process.env.ZDOTDIR || h, ".zshrc"), join(h, ".bashrc"), join(h, ".bash_profile"), join(h, ".profile"), join(process.env.XDG_CONFIG_HOME || join(h, ".config"), "fish", "config.fish")]
}

/** Drop the PATH lines install.sh added for `binDir` (always right after a "# oah" comment). Returns the files changed. */
function removePathLines(binDir: string): string[] {
  const changed: string[] = []
  for (const f of profiles()) {
    let text: string
    try {
      text = readFileSync(f, "utf8")
    } catch {
      continue
    }
    const lines = text.split("\n")
    const out: string[] = []
    for (let i = 0; i < lines.length; i++) {
      const l = lines[i]!
      if (out.at(-1) === "# oah" && l.includes(binDir) && /PATH|fish_add_path/.test(l)) {
        out.pop()
        if (out.at(-1) === "") out.pop()
        continue
      }
      out.push(l)
    }
    if (out.length !== lines.length) {
      writeFileSync(f, out.join("\n"))
      changed.push(f)
    }
  }
  return changed
}

/** Remove the oah binary and its PATH entry; with `purge`, also config, sessions and credentials. */
export function uninstall(o: { purge: boolean }): string[] {
  const exe = installedBinary()
  if (!exe) throw new Error("oah is running from source — nothing to uninstall")
  const binDir = dirname(exe)
  const done: string[] = []
  if (isWindows) {
    // Edit the raw registry value (keeps %VARS% unexpanded) and only when oah's folder is in it.
    const ps = [
      `$d = '${binDir.replace(/'/g, "''")}'`,
      "$k = Get-Item 'HKCU:\\Environment'",
      "$raw = $k.GetValue('Path', '', 'DoNotExpandEnvironmentNames')",
      "$parts = $raw -split ';'",
      "if ($parts -contains $d) {",
      "  Set-ItemProperty -Path 'HKCU:\\Environment' -Name Path -Value ((($parts | Where-Object { $_ -and $_ -ne $d })) -join ';') -Type ExpandString",
      "  [Environment]::SetEnvironmentVariable('OAH_UNINSTALL_REFRESH', '1', 'User'); [Environment]::SetEnvironmentVariable('OAH_UNINSTALL_REFRESH', $null, 'User')",
      "  'removed'",
      "}",
    ].join("\n")
    const r = Bun.spawnSync(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", ps], { stdout: "pipe", stderr: "ignore" })
    if (r.stdout.toString().includes("removed")) done.push(`removed ${binDir} from your user PATH`)
    // A running .exe can't delete itself, and Bun's child processes die with it on Windows, so the
    // cleanup is started through WMI: a hidden cmd.exe that waits a moment, then deletes.
    const del = [`del /f /q "${exe}"`, `del /f /q "${exe}.old"`, ...(o.purge ? [`rmdir /s /q "${homeDir()}"`] : [])].join(" & ")
    const cmdline = `cmd.exe /d /c ping -n 3 127.0.0.1 >nul & ${del}`
    const cleanup = `$si = New-CimInstance -ClassName Win32_ProcessStartup -ClientOnly -Property @{ ShowWindow = [uint16]0 }; Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = '${cmdline.replace(/'/g, "''")}'; ProcessStartupInformation = $si } | Out-Null`
    Bun.spawnSync(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", cleanup], { stdout: "ignore", stderr: "ignore" })
    done.push(`removing ${exe}`)
  } else {
    unlinkSync(exe)
    done.push(`removed ${shortPath(exe)}`)
    for (const f of removePathLines(binDir)) done.push(`removed the PATH line from ${shortPath(f)}`)
  }
  if (o.purge && !isWindows) {
    rmSync(homeDir(), { recursive: true, force: true })
    done.push(`removed ${shortPath(homeDir())} (settings, sessions, credentials)`)
  } else if (o.purge) done.push(`removing ${shortPath(homeDir())} (settings, sessions, credentials)`)
  return done
}

export const keptConfigNote = () => `Your settings, sessions and credentials are still in ${shortPath(homeDir())} — run \`oah uninstall --purge\` to remove them too.`
