// Paid opt-in smoke benchmark. Uses isolated source/state; never edits the caller's repo.
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { homeDir, loadSettings, paths } from "../src/config"

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..")
export const tasks = [
  { prompt: "Read math.ts and fix add so it adds rather than subtracts. Run bun test to verify.", check: (s: string) => /return a \+ b/.test(s) },
  { prompt: "Add export function multiply(a: number, b: number) to math.ts. Add a test in math.test.ts and run bun test.", check: (s: string) => /export function multiply/.test(s) },
  { prompt: "Add export function divide(a: number, b: number) to math.ts. Throw Error('division by zero') for zero divisor. Add tests and run bun test.", check: (s: string) => /export function divide/.test(s) },
]

async function bench() {
  if (!process.argv.includes("--run")) {
    console.log("Usage: bun scripts/bench.ts --run [--model provider/model] [--runs 3] [--stress] [--elision-threshold 0]\nRuns 3 sequential coding prompts per repetition. Uses your credentials (may incur charges). Scratch files and JSON results are retained for inspection. --self-test checks the harness without model requests.")
    if (!process.argv.includes("--self-test")) return
  }
  const selfTest = process.argv.includes("--self-test")
  const value = (flag: string) => { const i = process.argv.indexOf(flag); return i < 0 ? undefined : process.argv[i + 1] }
  const runs = Number(value("--runs") ?? 1)
  if (!Number.isInteger(runs) || runs < 1 || runs > 20) throw new Error("--runs must be 1–20")
  const model = value("--model") ?? loadSettings().model
  const threshold = Number(value("--elision-threshold") ?? settingsThreshold())
  if (!Number.isFinite(threshold) || threshold < 0) throw new Error("--elision-threshold must be nonnegative")
  const cases = [...tasks, ...(process.argv.includes("--stress") ? [{ prompt: "Synthetic context stress task: use read_file with limit 200 on each fixtures/f1.ts through fixtures/f6.ts (six separate reads). Then create fixture-total.ts exporting const total equal to the sum of their six weight constants. Do not change fixtures. Run bun test.", check: (_s: string) => true }] : [])]
  const sourceHome = homeDir()
  const authFile = paths.auth
  const settings = loadSettings()
  const results: unknown[] = []
  let failed = false
  for (let run = 0; run < runs; run++) {
    const work = mkdtempSync(join(tmpdir(), "oah-bench-"))
    const home = join(work, ".oah-home")
    mkdirSync(home)
    try {
    // Copy credentials only, never project MCP/skills/settings. Keep auth files private.
    if (!selfTest) try { writeFileSync(join(home, "auth.json"), readFileSync(authFile), { mode: 0o600 }) } catch {}
    try { writeFileSync(join(home, "models.json"), readFileSync(paths.modelsCache)) } catch {}
    writeFileSync(join(home, "settings.json"), JSON.stringify({ ...settings, model, mode: "build", permission: "allow", autoCompact: false, toolResultThreshold: threshold }))
    writeFileSync(join(work, ".gitignore"), ".oah-home/\nresults.json\n")
    writeFileSync(join(work, "math.ts"), "export function add(a: number, b: number) { return a - b }\n")
    writeFileSync(join(work, "math.test.ts"), 'import { expect, test } from "bun:test"\nimport { add } from "./math"\ntest("adds", () => expect(add(2, 3)).toBe(5))\n')
    mkdirSync(join(work, "fixtures"))
    for (let i = 1; i <= 6; i++) writeFileSync(join(work, "fixtures", `f${i}.ts`), `export const weight = ${i}\n` + Array.from({ length: 199 }, (_, n) => `// fixture ${i} line ${n}: ${"context ".repeat(10)}`).join("\n"))
    await Bun.$`git -C ${work} init -q`.quiet()
    let session: string | undefined
    let previous = { cost: 0, totalTokens: 0, input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }
    for (const [i, task] of cases.entries()) {
      const started = performance.now()
      const args = [process.execPath, join(root, "src/index.tsx"), "-p", task.prompt, "--json", "--quiet", "--permission", "allow", ...(model ? ["-m", model] : []), ...(session ? ["-r", session] : [])]
      let data: any
      let exitCode = 0
      if (selfTest) {
        const source = ['export function add(a: number, b: number) { return a + b }', ...(i >= 1 ? ['export function multiply(a: number, b: number) { return a * b }'] : []), ...(i >= 2 ? ["export function divide(a: number, b: number) { if (!b) throw Error('division by zero'); return a / b }"] : [])].join("\n")
        writeFileSync(join(work, "math.ts"), source)
        if (i === 3) writeFileSync(join(work, "fixture-total.ts"), "export const total = 21\n")
        data = { session: "self-test", cost: 0, totalTokens: 0 }
      } else {
        const p = Bun.spawn(args, { cwd: work, env: { ...process.env, OAH_HOME: home }, stdin: "ignore", stdout: "pipe", stderr: "pipe", timeout: 300_000 })
        const [out, err, code] = await Promise.all([new Response(p.stdout).text(), new Response(p.stderr).text(), p.exited])
        exitCode = code
        try { data = JSON.parse(out) } catch { data = { error: err || out || `exit ${code}` } }
      }
      session = data.session
      // Independent checks are outside the agent's tests; models cannot satisfy them by editing a test.
      const verify = Bun.spawn([process.execPath, "-e", `const m=await import('./math.ts'); if(m.add(2,3)!==5)throw Error('add'); ${i >= 1 ? "if(m.multiply(3,4)!==12)throw Error('multiply');" : ""} ${i >= 2 ? "if(m.divide(12,3)!==4)throw Error('divide'); let threw=false;try{m.divide(1,0)}catch{threw=true}if(!threw)throw Error('zero');" : ""} ${i === 3 ? "if((await import('./fixture-total.ts')).total!==21)throw Error('fixture sum');" : ""}`], { cwd: work, stdout: "ignore", stderr: "ignore" })
      const assertionsPassed = await verify.exited === 0 && task.check(readFileSync(join(work, "math.ts"), "utf8"))
      const passed = assertionsPassed && exitCode === 0 && !data.errors?.length && !data.denied?.length
      const totals = { cost: data.cost ?? 0, totalTokens: data.totalTokens ?? 0, input: data.usage?.input ?? 0, output: data.usage?.output ?? 0, cacheRead: data.usage?.cacheRead ?? 0, cacheWrite: data.usage?.cacheWrite ?? 0 }
      const delta = Object.fromEntries(Object.entries(totals).map(([k, v]) => [k, v - previous[k as keyof typeof totals]]))
      previous = totals
      const row = { run: run + 1, task: i + 1, assertionsPassed, exitCode, verification: data.verification?.status, temperature: i ? "warm-session" : "new-session", passed, wallMs: Math.round(performance.now() - started), ...delta, model: data.model ?? model, usageScope: "main-agent; cost includes auxiliary requests", elisionThreshold: threshold, work, error: data.error }
      results.push(row)
      console.log(JSON.stringify(row))
      failed ||= !passed
      if (!session) break
    }
    writeFileSync(join(work, "results.json"), JSON.stringify(results, null, 2))
    } finally { rmSync(join(home, "auth.json"), { force: true }) } // never retain copied credentials
  }
  console.error(`Benchmark ${selfTest ? "SELF-TEST (not model measurements)" : "complete"}; credential source: ${sourceHome}; results retained in scratch directories.`)
  if (failed) process.exitCode = 1
}
function settingsThreshold() { return loadSettings().toolResultThreshold ?? 60_000 }
if (import.meta.main) await bench()
