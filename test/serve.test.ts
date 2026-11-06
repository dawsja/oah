import { expect, test } from "bun:test"
import { mkdtempSync, readdirSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"

/** Drive `oah serve` like the desktop app does: JSON lines over stdio. */
async function serve() {
  const home = mkdtempSync(join(tmpdir(), "oah-serve-home-"))
  const cwd = mkdtempSync(join(tmpdir(), "oah-serve-cwd-"))
  const p = Bun.spawn(["bun", join(import.meta.dir, "../src/index.tsx"), "serve", "--cwd", cwd], {
    // An API key so a model is selected; no request is made in these tests.
    env: { ...process.env, OAH_HOME: home, ANTHROPIC_API_KEY: "test-key", OAH_NO_UPDATE_CHECK: "1" },
    stdin: "pipe",
    stdout: "pipe",
    stderr: "ignore",
  })
  const events: any[] = []
  const waiters: { pred: (e: any) => boolean; resolve: (e: any) => void }[] = []
  ;(async () => {
    const dec = new TextDecoder()
    let buf = ""
    for await (const chunk of p.stdout as unknown as AsyncIterable<Uint8Array>) {
      buf += dec.decode(chunk)
      for (let nl = buf.indexOf("\n"); nl >= 0; nl = buf.indexOf("\n")) {
        const e = JSON.parse(buf.slice(0, nl))
        buf = buf.slice(nl + 1)
        events.push(e)
        for (const w of waiters.splice(0)) (w.pred(e) ? w.resolve(e) : waiters.push(w))
      }
    }
  })()
  const next = (pred: (e: any) => boolean, ms = 10_000) =>
    new Promise<any>((resolve, reject) => {
      const hit = events.find(pred)
      if (hit) return resolve(hit)
      const timer = setTimeout(() => reject(new Error("timed out waiting for an event")), ms)
      waiters.push({ pred, resolve: (e) => (clearTimeout(timer), resolve(e)) })
    })
  const send = (op: object) => {
    p.stdin.write(JSON.stringify(op) + "\n")
    p.stdin.flush()
  }
  const clear = () => events.splice(0)
  return { p, home, cwd, next, send, clear, events }
}

test("serve: hello, state, commands, pickers and a clean exit", async () => {
  const s = await serve()
  const hello = await s.next((e) => e.t === "hello")
  expect(hello.protocol).toBe(1)
  expect(hello.commands.some((c: any) => c.name === "model")).toBe(true)
  const first = await s.next((e) => e.t === "state")
  expect(first.s.cwd).toBe(s.cwd)
  expect(first.s.settings.mode).toBe("build")

  s.clear()
  s.send({ op: "submit", text: "/plan" })
  const plan = await s.next((e) => e.t === "state" && e.s.settings.mode === "plan")
  expect(plan.s.toast.text).toContain("Plan mode")

  s.send({ op: "submit", text: "/help" })
  const help = await s.next((e) => e.t === "transcript" && e.changes.some(([, x]: any) => x.kind === "info"))
  expect(help.changes.at(-1)[1].text).toContain("Keys")

  // Pickers are state: open one, then pick an item by value.
  s.send({ op: "submit", text: "/permissions" })
  const picker = await s.next((e) => e.t === "state" && e.s.picker?.kind === "permission")
  expect(picker.s.picker.items.map((i: any) => i.value)).toEqual(["auto", "ask", "allow"])
  s.send({ op: "pick", value: "ask" })
  await s.next((e) => e.t === "state" && !e.s.picker && e.s.settings.permission === "ask")

  // An unknown op is reported, not fatal.
  s.send({ op: "nope" })
  await s.next((e) => e.t === "error")

  s.send({ op: "shutdown" })
  await s.next((e) => e.t === "exit")
  expect(await s.p.exited).toBe(0)
  // Settings changed through the engine are the CLI's settings file.
  expect(readdirSync(s.home)).toContain("settings.json")
}, 30_000)

test("serve: resuming a session doesn't add a note to it", async () => {
  const s = await serve()
  await s.next((e) => e.t === "state")
  // Save a session to switch to.
  s.send({ op: "submit", text: "#remember this" })
  const id = "feedfacecafe"
  await Bun.write(join(s.home, "sessions", `${id}.json`), JSON.stringify({ id, created: 1, updated: 2, cwd: s.cwd, title: "saved", model: "", effort: "medium", mode: "build", messages: [{ role: "user", content: "hi" }], transcript: [{ kind: "user", text: "hi" }], contextTokens: 0, totalTokens: 0, todos: [], compactions: 0 }))
  s.clear()
  s.send({ op: "resume", id })
  const t = await s.next((e) => e.t === "transcript" && e.session === id)
  expect(t.length).toBe(1)
  expect(t.changes.map(([, x]: any) => x.kind)).toEqual(["user"])
  s.send({ op: "shutdown" })
  expect(await s.p.exited).toBe(0)
}, 30_000)

test("serve: closing stdin saves and exits", async () => {
  const s = await serve()
  await s.next((e) => e.t === "hello")
  s.p.stdin.end()
  expect(await s.p.exited).toBe(0)
}, 30_000)
