// Background shells: long-running commands (dev servers, watchers, slow test
// runs) that the model starts with `bash background: true`, then polls with
// `bash_output` and stops with `bash_kill`.
import { stripAnsi } from "./index"

const MAX_BUFFER = 1 << 20

export interface Shell {
  id: string
  command: string
  cwd: string
  started: number
  ended?: number
  exitCode: number | null
  output: string
  /** Offset already returned by bash_output. */
  readPos: number
  /** Characters dropped from the front when the buffer overflowed. */
  dropped: number
  proc: ReturnType<typeof Bun.spawn>
}

const shells = new Map<string, Shell>()
let counter = 0
let onChange: () => void = () => {}

export function setShellListener(fn: () => void) {
  onChange = fn
}

export function startShell(command: string, cwd: string): Shell {
  const id = `bg${++counter}`
  const proc = Bun.spawn(["bash", "-c", command], {
    cwd,
    stdin: "ignore",
    stdout: "pipe",
    stderr: "pipe",
    env: { ...process.env, NUR: "1", TERM: "dumb", PAGER: "cat", GIT_PAGER: "cat" },
  })
  const sh: Shell = { id, command, cwd, started: Date.now(), exitCode: null, output: "", readPos: 0, dropped: 0, proc }
  shells.set(id, sh)
  const pump = async (stream: ReadableStream<Uint8Array>) => {
    const dec = new TextDecoder()
    const reader = stream.getReader()
    for (;;) {
      const { done, value } = await reader.read()
      if (done) break
      append(sh, dec.decode(value, { stream: true }))
    }
  }
  Promise.all([pump(proc.stdout as ReadableStream<Uint8Array>), pump(proc.stderr as ReadableStream<Uint8Array>)]).catch(() => {})
  proc.exited.then((code) => {
    sh.exitCode = code
    sh.ended = Date.now()
    onChange()
  })
  onChange()
  return sh
}

function append(sh: Shell, text: string) {
  sh.output += stripAnsi(text)
  if (sh.output.length > MAX_BUFFER) {
    const cut = sh.output.length - MAX_BUFFER
    sh.output = sh.output.slice(cut)
    sh.dropped += cut
    sh.readPos = Math.max(0, sh.readPos - cut)
  }
}

export function getShell(id: string): Shell | undefined {
  return shells.get(id)
}

/** Output produced since the last read (optionally only lines matching `filter`). */
export function readShell(sh: Shell, filter?: RegExp): string {
  let text = sh.output.slice(sh.readPos)
  sh.readPos = sh.output.length
  if (filter) text = text.split("\n").filter((l) => filter.test(l)).join("\n")
  return text
}

export function killShell(sh: Shell) {
  if (sh.exitCode === null) sh.proc.kill("SIGTERM")
  setTimeout(() => sh.exitCode === null && sh.proc.kill("SIGKILL"), 2000)
}

export function runningShells(): Shell[] {
  return [...shells.values()].filter((s) => s.exitCode === null)
}

export function allShells(): Shell[] {
  return [...shells.values()]
}

export function killAllShells() {
  for (const s of runningShells()) {
    try {
      s.proc.kill("SIGKILL")
    } catch {}
  }
}

export function shellStatus(sh: Shell): string {
  const secs = Math.round(((sh.ended ?? Date.now()) - sh.started) / 1000)
  return sh.exitCode === null ? `running for ${secs}s` : `exited with code ${sh.exitCode} after ${secs}s`
}
