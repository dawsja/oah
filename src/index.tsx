#!/usr/bin/env bun
// oah — a full-screen AI coding agent for the terminal (OpenTUI + AI SDK).
import { createCliRenderer } from "@opentui/core"
import { createRoot } from "@opentui/react"
import * as auth from "./auth"
import { refreshCatalog } from "./catalog"
import { ensureDir, homeDir, joinModel, loadSettings, shortPath, splitModel, VERSION, type Effort, type Permission } from "./config"
import { Mcp } from "./mcp"
import { Memory } from "./memory"
import { latestSession, listSessions, loadSession, relativeTime, saveSession } from "./session"
import { Skills } from "./skills"
import { readStdin, runPrint } from "./print"
import { App } from "./ui/App"
import { Store } from "./ui/store"
import { checkForUpdate, cleanupOldBinary, compareVersions, fetchRelease, installedBinary, keptConfigNote, uninstall, update } from "./update"

const USAGE = `oah — Open Agent Harness

Usage:
  oah [options] [prompt]   Full-screen app (an optional first prompt)
  oah -p "prompt"         Run once, print the answer, exit (also: echo … | oah -p)
  oah sessions            List saved sessions
  oah login               Continue with ChatGPT (no TUI)
  oah logout              Sign out of ChatGPT
  oah models              Refresh the model catalog from models.dev
  oah update [version]    Update to the latest release (--check: only check)
  oah uninstall           Remove oah (--purge: also settings, sessions, credentials; --yes: don't ask)

Options:
  -r, --resume [id]       Resume a session (default: most recent here)
  -c, --continue          Continue the most recent session here
  -m, --model <p/model>   e.g. chatgpt/gpt-5.6, anthropic/claude-opus-5-5, google/gemini-4-argon
  -e, --effort <level>    minimal | low | medium | high | xhigh | max
  -p, --print             Non-interactive: stream the answer to stdout, progress to stderr
      --json              With -p: print one JSON result (answer, session, cost, errors)
  -q, --quiet             With -p: no progress on stderr
      --permission <lvl>  auto | ask | allow (with -p, anything that would ask is denied)
      --yolo              Same as --permission allow
      --plan              Start in plan mode
  -h, --help · -V, --version

Config lives in ${shortPath(homeDir())}`

interface Args {
  resume?: string | true
  model?: string
  effort?: Effort
  permission?: Permission
  plan?: boolean
  print?: boolean
  json?: boolean
  quiet?: boolean
  prompt?: string
  sub?: string
  /** Arguments after `update` / `uninstall`. */
  subArgs?: string[]
}

function parseArgs(argv: string[]): Args {
  const a: Args = {}
  const rest: string[] = []
  if (argv[0] === "update" || argv[0] === "uninstall") return { sub: argv[0], subArgs: argv.slice(1) }
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
      case "--print":
        a.print = true
        break
      case "--json":
        a.json = true
        a.print = true
        break
      case "-q":
      case "--quiet":
        a.quiet = true
        break
      case "--yolo":
        a.permission = "allow"
        break
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
          ["clip.exe"], // WSL: the Windows clipboard
        ]
  for (const cmd of candidates) {
    try {
      const p = Bun.spawn(cmd, { stdin: "pipe", stdout: "ignore", stderr: "ignore" })
      // clip.exe reads UTF-16LE (with a BOM) correctly; anything else garbles non-ASCII.
      p.stdin.write(cmd[0] === "clip.exe" ? Buffer.concat([Buffer.from([0xff, 0xfe]), Buffer.from(text, "utf16le")]) : text)
      p.stdin.end()
      p.unref()
      return true
    } catch {}
  }
  return false
}

async function updateCommand(a: string[]): Promise<number> {
  const unknown = a.find((x) => x.startsWith("-") && x !== "--check")
  if (unknown) return console.error(`oah update: unknown option ${unknown}`), 2
  const version = a.find((x) => !x.startsWith("-"))
  try {
    if (a.includes("--check")) {
      const rel = await fetchRelease(version)
      const newer = compareVersions(rel.version, VERSION) > 0
      console.log(newer ? `oah ${rel.version} is available (you have ${VERSION}) — run: oah update` : `oah ${VERSION} is up to date.`)
      return 0
    }
    const r = await update({ version, onStatus: (s) => process.stderr.write(`\x1b[2m${s}…\x1b[0m\n`) })
    if (r.status === "current") console.log(`oah ${r.version} is already the latest version.`)
    else console.log(`Updated oah ${r.from} → ${r.to} (${shortPath(r.path)}).`)
    return 0
  } catch (e: any) {
    console.error(`oah update: ${e?.message ?? e}`)
    return 1
  }
}

async function uninstallCommand(a: string[]): Promise<number> {
  const unknown = a.find((x) => !["--purge", "--yes", "-y"].includes(x))
  if (unknown) return console.error(`oah uninstall: unknown option ${unknown}`), 2
  const purge = a.includes("--purge")
  const exe = installedBinary()
  if (!exe) return console.error("oah uninstall: oah is running from source — nothing to uninstall"), 1
  if (!a.includes("--yes") && !a.includes("-y")) {
    if (!process.stdin.isTTY) return console.error("oah uninstall: pass --yes to confirm when not running in a terminal"), 2
    process.stdout.write(`Remove ${shortPath(exe)}${purge ? ` and everything in ${shortPath(homeDir())} (settings, sessions, credentials)` : ""}? [y/N] `)
    const answer = await new Promise<string>((res) => process.stdin.once("data", (d) => res(String(d).trim().toLowerCase())))
    process.stdin.pause()
    if (answer !== "y" && answer !== "yes") return console.log("Cancelled."), 1
  }
  try {
    for (const line of uninstall({ purge })) console.log(`  ${line}`)
    console.log(`oah is uninstalled.${purge ? "" : ` ${keptConfigNote()}`}`)
    return 0
  } catch (e: any) {
    console.error(`oah uninstall: ${e?.message ?? e}`)
    return 1
  }
}

async function main() {
  cleanupOldBinary() // before anything that can exit early (--version, --help)
  let args: Args
  try {
    args = parseArgs(process.argv.slice(2))
  } catch (e: any) {
    console.error(`oah: ${e.message}\n\n${USAGE}`)
    process.exit(2)
  }
  ensureDir(homeDir())

  if (args.sub === "update") process.exit(await updateCommand(args.subArgs ?? []))
  if (args.sub === "uninstall") process.exit(await uninstallCommand(args.subArgs ?? []))
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

  const headless = !!args.print || !process.stdin.isTTY || !process.stdout.isTTY
  let prompt = args.prompt ?? ""
  if (headless) {
    const piped = (await readStdin(prompt ? 400 : undefined)).trim()
    if (piped) prompt = prompt ? `${prompt}\n\n<stdin>\n${piped}\n</stdin>` : piped
    if (!prompt.trim()) {
      console.error(args.print ? 'oah: -p needs a prompt, e.g. oah -p "fix the failing tests" (or pipe one in)' : "oah needs an interactive terminal (or use -p \"prompt\" to run once and print the answer).")
      process.exit(2)
    }
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
  const model = args.model ? joinModel(splitModel(args.model).provider, splitModel(args.model).slug) : undefined

  if (headless) process.exit(await runPrint({ prompt, settings, session, cwd, json: !!args.json, quiet: !!args.quiet, model }))

  // Anything written to stderr lands on top of the full-screen UI: silence the AI
  // SDK's warning logger and Node-style process warnings while the TUI runs.
  ;(globalThis as any).AI_SDK_LOG_WARNINGS = false
  process.emitWarning = () => {}

  const renderer = await createCliRenderer({ exitOnCtrlC: false, useMouse: true, targetFps: 60, useConsole: false } as any)
  const copy = (text: string) => {
    const osc = renderer.copyToClipboardOSC52(text)
    return copyNative(text) || osc
  }
  let store: Store
  const mcp = new Mcp(cwd, () => store?.emit())
  store = new Store(settings, session, cwd, new Skills(cwd), new Memory(cwd), mcp, copy)
  if (model) store.settings.model = model
  if (session) store.info(`Resumed session ${session.id}`)
  mcp.start()
  void checkForUpdate().then((v) => {
    if (!v) return
    store.updateAvailable = v
    store.notify(`oah ${v} is available — /update`)
  })
  void mcp.settled().then(() => {
    const need = mcp.servers.filter((x) => x.state === "needs-auth").map((x) => x.name)
    if (need.length) store.notify(`MCP: ${need.join(", ")} need${need.length === 1 ? "s" : ""} authentication — /mcp to sign in`)
  })

  let exiting = false
  const exit = async () => {
    if (exiting) return
    exiting = true
    store.shutdown()
    renderer.destroy()
    // A slow MCP server mustn't hold up exiting (or the resume hint).
    await Promise.race([mcp.close().catch(() => {}), new Promise((r) => setTimeout(r, 2000))])
    if (store.session.messages.length) {
      saveSession(store.session)
      process.stdout.write(`\x1b[2mSession saved.\x1b[0m Resume with:\n\n  \x1b[1moah --resume ${store.session.id}\x1b[0m\n\n`)
    } else {
      // Nothing new to save: point at the latest conversation in this folder instead.
      const last = latestSession(store.cwd)
      if (last?.messages.length)
        process.stdout.write(`\x1b[2mNothing new to save. Your last session here — “${last.title || "untitled"}” (${relativeTime(last.updated)}):\x1b[0m\n\n  \x1b[1moah -c\x1b[0m   \x1b[2mor\x1b[0m   \x1b[1moah --resume ${last.id}\x1b[0m\n\n`)
    }
    process.exit(0)
  }
  process.on("SIGTERM", exit)
  process.on("SIGHUP", exit)
  createRoot(renderer).render(<App store={store} onExit={exit} />)
  if (prompt) store.submit(prompt)
}

main()
