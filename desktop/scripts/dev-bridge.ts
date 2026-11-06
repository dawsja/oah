// Development only: run the desktop UI in a plain browser (`bun run web`) against
// the real engine. This stands in for the Tauri shell (src-tauri/src/lib.rs):
// it spawns `oah serve` from this repo's source and answers the same calls over
// a WebSocket on 127.0.0.1:1430.
//
//   bun run web:bridge     # terminal 1 (OAH_HOME=… to use a scratch config)
//   bun run web            # terminal 2, then open http://localhost:1420
import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, statSync, writeFileSync } from "node:fs"
import { homedir } from "node:os"
import { join, resolve } from "node:path"
import type { ServerWebSocket } from "bun"

const ENTRY = resolve(import.meta.dir, "../../src/index.tsx")
const PORT = Number(process.env.OAH_BRIDGE_PORT ?? 1430)

const home = () => process.env.OAH_HOME || join(process.env.XDG_CONFIG_HOME || join(homedir(), ".config"), "oah")
const prefsFile = () => join(home(), "desktop.json")
const short = (p: string) => (p.startsWith(homedir()) ? "~" + p.slice(homedir().length) : p)
const readPrefs = (): Record<string, unknown> => {
  try {
    return JSON.parse(readFileSync(prefsFile(), "utf8"))
  } catch {
    return {}
  }
}
const writePrefs = (patch: Record<string, unknown>) => {
  const p = readPrefs()
  for (const [k, v] of Object.entries(patch)) v === null ? delete p[k] : (p[k] = v)
  mkdirSync(home(), { recursive: true })
  writeFileSync(prefsFile() + ".tmp", JSON.stringify(p, null, 2))
  renameSync(prefsFile() + ".tmp", prefsFile())
}

function sessions() {
  const dir = join(home(), "sessions")
  if (!existsSync(dir)) return []
  const out = []
  for (const f of readdirSync(dir)) {
    if (!f.endsWith(".json")) continue
    try {
      const s = JSON.parse(readFileSync(join(dir, f), "utf8"))
      out.push({ id: s.id, title: s.title ?? "", cwd: s.cwd ?? "", cwdShort: short(s.cwd ?? ""), created: s.created ?? 0, updated: s.updated ?? 0, model: s.model ?? "", mode: s.mode ?? "build", entries: s.transcript?.length ?? 0, hasGoal: !!s.goal && !s.goal.done, detailed: true })
    } catch {}
  }
  return out.sort((a, b) => b.updated - a.updated)
}

let engine: ReturnType<typeof Bun.spawn> | null = null
let client: ServerWebSocket<unknown> | null = null
let gen = 0

async function start(opts: { cwd?: string; resume?: string }) {
  const last = readPrefs().lastCwd as string | undefined
  const cwd = [opts.cwd, last, homedir()].find((d) => d && existsSync(d) && statSync(d).isDirectory())!
  const old = engine
  if (old) {
    old.stdin && (old.stdin as any).end?.()
    setTimeout(() => old.kill(), 5000)
  }
  const mine = ++gen
  const p = Bun.spawn(["bun", ENTRY, "serve", "--cwd", cwd, ...(opts.resume ? ["--resume", opts.resume] : [])], { cwd, stdin: "pipe", stdout: "pipe", stderr: "pipe" })
  engine = p
  const relay = (line: string) => {
    if (process.env.OAH_BRIDGE_DEBUG) console.log(new Date().toISOString().slice(17, 23), "→", mine === gen ? (client ? "sent" : "no client") : "stale", line.slice(0, 80))
    if (mine === gen) client?.send(JSON.stringify({ line }))
  }
  ;(async () => {
    const dec = new TextDecoder()
    let buf = ""
    for await (const chunk of p.stdout as unknown as AsyncIterable<Uint8Array>) {
      buf += dec.decode(chunk)
      for (let nl = buf.indexOf("\n"); nl >= 0; nl = buf.indexOf("\n")) {
        const l = buf.slice(0, nl).trim()
        buf = buf.slice(nl + 1)
        if (l) relay(l)
      }
    }
    const stderr = await new Response(p.stderr as ReadableStream).text()
    relay(JSON.stringify({ t: "engine-exit", code: await p.exited, stderr: stderr.split("\n").slice(-60).join("\n") }))
  })()
  writePrefs({ lastCwd: cwd })
  return { cwd, cwdShort: short(cwd), engine: `source at ${ENTRY} (dev bridge)` }
}

const methods: Record<string, (p: any) => unknown> = {
  start,
  send({ op }) {
    const stdin = engine?.stdin as any
    if (!stdin) throw new Error("The engine isn't running")
    stdin.write(JSON.stringify(op) + "\n")
    stdin.flush?.()
  },
  sessions,
  info: () => ({ version: "dev", bundle: null, os: process.platform === "darwin" ? "macos" : process.platform === "win32" ? "windows" : "linux", home: home(), homeShort: short(home()), homeDir: homedir(), sessionsDir: join(home(), "sessions"), oahHomeEnv: !!process.env.OAH_HOME, xdgEnv: !!process.env.XDG_CONFIG_HOME }),
  prefs: readPrefs,
  setPrefs: ({ patch }) => writePrefs(patch),
}

Bun.serve({
  hostname: "127.0.0.1",
  port: PORT,
  async fetch(req, server) {
    // `?shot=<ms>` in the UI loads this image so headless Chrome's --screenshot
    // (taken at the load event) waits until the UI is ready.
    const url = new URL(req.url)
    if (url.pathname === "/hold") {
      await Bun.sleep(Math.min(30_000, Number(url.searchParams.get("ms")) || 0))
      return new Response(Buffer.from("R0lGODlhAQABAAAAACw=", "base64"), { headers: { "content-type": "image/gif" } })
    }
    // Only the local dev UI may connect.
    const origin = req.headers.get("origin") ?? ""
    if (!/^http:\/\/(localhost|127\.0\.0\.1):1420$/.test(origin)) return new Response("forbidden", { status: 403 })
    return server.upgrade(req) ? undefined : new Response("websocket only", { status: 400 })
  },
  websocket: {
    open(ws) {
      client = ws
    },
    async message(ws, raw) {
      const { id, method, params } = JSON.parse(String(raw))
      if (process.env.OAH_BRIDGE_DEBUG) console.log(new Date().toISOString().slice(17, 23), "←", method, JSON.stringify(params ?? {}).slice(0, 120))
      try {
        const fn = methods[method]
        if (!fn) throw new Error(`unknown method ${method}`)
        ws.send(JSON.stringify({ id, result: (await fn(params ?? {})) ?? null }))
      } catch (e: any) {
        ws.send(JSON.stringify({ id, error: e?.message ?? String(e) }))
      }
    },
    close(ws) {
      if (client === ws) client = null
    },
  },
})
console.log(`oah dev bridge on ws://127.0.0.1:${PORT} · config ${short(home())} · engine ${short(ENTRY)}`)
