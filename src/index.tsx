#!/usr/bin/env bun
// oah — a full-screen AI coding agent for the terminal (OpenTUI + AI SDK).
import { createCliRenderer } from "@opentui/core"
import { createRoot } from "@opentui/react"
import * as auth from "./auth"
import { refreshCatalog } from "./catalog"
import { ensureDir, homeDir, loadSettings, PROVIDERS, shortPath, VERSION, type Effort, type Permission } from "./config"
import { Mcp } from "./mcp"
import { Memory } from "./memory"
import { latestSession, listSessions, loadSession, relativeTime, saveSession } from "./session"
import { Skills } from "./skills"
import { App } from "./ui/App"
import { Store } from "./ui/store"

const USAGE = `oah — an AI coding agent for your terminal

Usage:
  oah [options] [prompt]
  oah sessions            List saved sessions
  oah login               Continue with ChatGPT (no TUI)
  oah logout              Sign out of ChatGPT
  oah models              Refresh the model catalog from models.dev

Options:
  -r, --resume [id]       Resume a session (default: most recent here)
  -c, --continue          Continue the most recent session here
  -m, --model <p/model>   e.g. chatgpt/gpt-5.6, anthropic/claude-opus-5-5, openai/gpt-5.6
  -e, --effort <level>    minimal | low | medium | high | xhigh | max
  -p, --permission <lvl>  auto | ask | allow
      --plan              Start in plan mode
  -h, --help · -V, --version

Config lives in ${shortPath(homeDir())}`

interface Args {
  resume?: string | true
  model?: string
  effort?: Effort
  permission?: Permission
  plan?: boolean
  prompt?: string
  sub?: string
}

function parseArgs(argv: string[]): Args {
  const a: Args = {}
  const rest: string[] = []
  for (let i = 0; i < argv.length; i++) {
    const s = argv[i]!
    const next = () => {
      const v = argv[++i]
      if (v === undefined) throw new Error(`${s} needs a value`)
      return v
    }
    switch (s) {
      case "-h":
      case "--help":
        console.log(USAGE)
        process.exit(0)
      case "-V":
      case "--version":
        console.log(`oah ${VERSION}`)
        process.exit(0)
      case "-r":
      case "--resume": {
        const v = argv[i + 1]
        if (v && !v.startsWith("-")) (a.resume = v), i++
        else a.resume = true
        break
      }
      case "-c":
      case "--continue":
        a.resume = true
        break
      case "-m":
      case "--model":
        a.model = next()
        break
      case "-e":
      case "--effort":
        a.effort = next() as Effort
        break
      case "-p":
      case "--permission":
        a.permission = ({ ask: "ask", "always-ask": "ask", allow: "allow", "always-allow": "allow", auto: "auto" } as Record<string, Permission>)[next()]
        break
      case "--plan":
        a.plan = true
        break
      default:
        if (s.startsWith("-")) throw new Error(`unknown option ${s}`)
        if (!rest.length && !a.sub && ["sessions", "login", "logout", "models"].includes(s)) a.sub = s
        else rest.push(s)
    }
  }
  if (rest.length) a.prompt = rest.join(" ")
  return a
}

function copyNative(text: string): boolean {
  const candidates: string[][] =
    process.platform === "darwin"
      ? [["pbcopy"]]
      : [
          ...(process.env.WAYLAND_DISPLAY ? [["wl-copy"]] : []),
          ...(process.env.DISPLAY ? [["xclip", "-selection", "clipboard"], ["xsel", "--clipboard", "--input"]] : []),
        ]
  for (const cmd of candidates) {
    try {
      const p = Bun.spawn(cmd, { stdin: "pipe", stdout: "ignore", stderr: "ignore" })
      p.stdin.write(text)
      p.stdin.end()
      p.unref()
      return true
    } catch {}
  }
  return false
}

async function main() {
  let args: Args
  try {
    args = parseArgs(process.argv.slice(2))
  } catch (e: any) {
    console.error(`oah: ${e.message}\n\n${USAGE}`)
    process.exit(2)
  }
  ensureDir(homeDir())

  if (args.sub === "sessions") {
    const list = listSessions()
    if (!list.length) console.log("No saved sessions.")
    for (const s of list.slice(0, 50)) console.log(`${s.id}  ${relativeTime(s.updated).padStart(8)}  ${shortPath(s.cwd)}  ${s.title || "(untitled)"}`)
    return
  }
  if (args.sub === "login") {
    const p = await auth.beginChatGptLogin()
    console.log(`Opening your browser to continue with ChatGPT. If it doesn't open, visit:\n\n  ${p.displayUrl}\n`)
    auth.openBrowser(p.url)
    try {
      const s = await p.done
      console.log(s.chatgpt?.planUsage ? `Signed in${s.chatgpt.email ? ` as ${s.chatgpt.email}` : ""}. Using your ChatGPT plan.` : "Signed in, but plan usage wasn't granted.")
    } catch (e: any) {
      console.error(`Sign-in failed: ${e.message}`)
      process.exit(1)
    }
    return
  }
  if (args.sub === "logout") return console.log(await auth.logoutChatGpt())
  if (args.sub === "models") return console.log((await refreshCatalog(true)) ? "Model catalog updated from models.dev." : "Couldn't reach models.dev.")

  if (!process.stdin.isTTY || !process.stdout.isTTY) {
    console.error("oah needs an interactive terminal.")
    process.exit(1)
  }

  const settings = loadSettings()
  if (args.effort) settings.effort = args.effort
  if (args.permission) settings.permission = args.permission
  if (args.plan) settings.mode = "plan"

  let session = undefined
  if (args.resume) {
    session = typeof args.resume === "string" ? loadSession(args.resume) : latestSession(process.cwd())
    if (!session && typeof args.resume === "string") {
      console.error(`oah: no session matches '${args.resume}'. Run \`oah sessions\` to list them.`)
      process.exit(1)
    }
    if (session?.cwd) {
      try {
        process.chdir(session.cwd)
      } catch {}
    }
  }
  const cwd = process.cwd()

  const renderer = await createCliRenderer({ exitOnCtrlC: false, useMouse: true, targetFps: 60, useConsole: false } as any)
  const copy = (text: string) => {
    const osc = renderer.copyToClipboardOSC52(text)
    return copyNative(text) || osc
  }
  let store: Store
  const mcp = new Mcp(cwd, () => store?.emit())
  store = new Store(settings, session, cwd, new Skills(cwd), new Memory(cwd), mcp, copy)
  if (args.model) store.settings.model = args.model.includes("/") ? args.model : `${PROVIDERS.find((p) => args.model!.startsWith("claude") === (p === "anthropic")) ?? "chatgpt"}/${args.model}`
  if (session) store.info(`Resumed session ${session.id}`)
  mcp.start()

  let exiting = false
  const exit = async () => {
    if (exiting) return
    exiting = true
    store.shutdown()
    renderer.destroy()
    await mcp.close().catch(() => {})
    if (store.session.messages.length) {
      saveSession(store.session)
      process.stdout.write(`\x1b[2mSession saved.\x1b[0m Resume with:\n\n  \x1b[1moah --resume ${store.session.id}\x1b[0m\n\n`)
    }
    process.exit(0)
  }
  process.on("SIGTERM", exit)
  process.on("SIGHUP", exit)
  createRoot(renderer).render(<App store={store} onExit={exit} />)
  if (args.prompt) store.submit(args.prompt)
}

main()
