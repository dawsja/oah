// OS differences in one place: which shell runs commands, opening URLs, and
// stopping a command together with everything it started.
import { existsSync } from "node:fs"

export const isWindows = process.platform === "win32"

let shell: { argv: string[]; name: "PowerShell" | "bash" | "sh" } | undefined

/** The shell that runs the `bash` tool's commands: PowerShell on Windows (pwsh 7 when installed), else bash (sh as a fallback). */
export function commandShell() {
  if (!shell) {
    if (isWindows) {
      const exe = Bun.which("pwsh") ?? Bun.which("powershell") ?? "powershell.exe"
      shell = { argv: [exe, "-NoLogo", "-NoProfile", "-NonInteractive", "-Command"], name: "PowerShell" }
    } else if (Bun.which("bash") || existsSync("/bin/bash")) shell = { argv: [Bun.which("bash") ?? "/bin/bash", "-c"], name: "bash" }
    else shell = { argv: ["/bin/sh", "-c"], name: "sh" }
  }
  return shell
}

/** argv for running `command` in the command shell. */
export function shellArgv(command: string): string[] {
  const s = commandShell()
  if (s.name !== "PowerShell") return [...s.argv, command]
  // PowerShell: UTF-8 output, no progress bars, and a failing command gives a non-zero exit code like `bash -c`.
  const script = [
    "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8",
    "$ProgressPreference = 'SilentlyContinue'",
    "if ($PSStyle) { $PSStyle.OutputRendering = 'PlainText' }",
    command,
    "$__ok = $?; $__code = $LASTEXITCODE",
    "if ($__code) { exit $__code }",
    "if (-not $__ok) { exit 1 }",
  ].join("\n")
  return [...s.argv, script]
}

/**
 * Run a shell command. On POSIX it gets its own process group (setsid), so
 * `killTree` can stop everything it started — not just the shell, whose
 * children would otherwise keep running and hold the output pipes open.
 */
export function spawnCommand(command: string, o: { cwd: string; env: Record<string, string | undefined> }) {
  return Bun.spawn(shellArgv(command), { cwd: o.cwd, env: o.env, stdin: "ignore", stdout: "pipe", stderr: "pipe", detached: !isWindows })
}

/** Stop a command and everything it started. */
export function killTree(proc: { pid: number; kill: (sig?: any) => void }, signal: "SIGTERM" | "SIGKILL" = "SIGKILL") {
  if (isWindows) {
    try {
      Bun.spawn(["taskkill", "/PID", String(proc.pid), "/T", "/F"], { stdout: "ignore", stderr: "ignore" })
      return
    } catch {}
  }
  try {
    process.kill(-proc.pid, signal) // the whole process group (spawnCommand made it the leader)
    return
  } catch {}
  try {
    proc.kill(signal)
  } catch {}
}

/** Open a URL in the default browser. Returns false when no opener is available. */
export function openUrl(url: string): boolean {
  const cmds: string[][] =
    process.platform === "darwin"
      ? [["open", url]]
      : isWindows
        ? [["rundll32", "url.dll,FileProtocolHandler", url]]
        : [["xdg-open", url], ["wslview", url], ["sensible-browser", url]]
  for (const c of cmds) {
    try {
      Bun.spawn(c, { stdout: "ignore", stderr: "ignore", stdin: "ignore" }).unref()
      return true
    } catch {}
  }
  return false
}
