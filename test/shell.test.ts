import { describe, expect, test } from "bun:test"
import { existsSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { TOOLS, type ToolCtx } from "../src/tools"

const bash = TOOLS.find((t) => t.name === "bash")!
const ctx = (signal = new AbortController().signal) => ({ cwd: tmpdir(), signal }) as unknown as ToolCtx
const alive = (pid: number) => {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

describe.skipIf(process.platform === "win32")("bash tool process handling", () => {
  test("a timeout stops grandchildren that hold the output pipe", async () => {
    const pidFile = join(tmpdir(), `oah-grandchild-${process.pid}`)
    const t0 = Date.now()
    // sh → sleep (grandchild, inherits stdout); the shell itself waits on it.
    const r = await bash.run({ command: `sh -c 'sleep 30 & echo $! > ${pidFile}; wait'`, timeout_ms: 1500 }, ctx())
    expect(Date.now() - t0).toBeLessThan(8000)
    expect(r.ok).toBe(false)
    expect(r.summary).toContain("timed out")
    const pid = Number(await Bun.file(pidFile).text())
    await Bun.sleep(300)
    expect(alive(pid)).toBe(false)
  }, 15000)

  test("a finished command that leaves something running doesn't hang", async () => {
    const t0 = Date.now()
    const r = await bash.run({ command: "sleep 20 & echo started", timeout_ms: 60000 }, ctx())
    expect(Date.now() - t0).toBeLessThan(6000)
    expect(r.output).toContain("started")
    expect(r.output).toContain("exit code: 0")
  }, 15000)

  test("esc (abort) stops the whole group", async () => {
    const ac = new AbortController()
    const t0 = Date.now()
    setTimeout(() => ac.abort(), 500)
    const r = await bash.run({ command: "sh -c 'sleep 30' | cat", timeout_ms: 60000 }, ctx(ac.signal))
    expect(Date.now() - t0).toBeLessThan(5000)
    expect(r.summary).toBe("interrupted")
  }, 15000)

  test("normal commands still report output and exit codes", async () => {
    const r = await bash.run({ command: "echo hi; exit 3" }, ctx())
    expect(r.output).toContain("hi")
    expect(r.output).toContain("exit code: 3")
    expect(existsSync(tmpdir())).toBe(true)
  })
})
