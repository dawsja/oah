// Non-interactive mode: `oah -p "fix the tests"`. Runs the same engine as the
// TUI (sessions, checkpoints, cost, compaction), streams the answer to stdout and
// progress to stderr, then exits. Nobody is there to approve anything, so tool
// calls that would ask are denied unless you pass --permission allow.
import type { Settings } from "./config"
import { displayName } from "./catalog"
import { Mcp } from "./mcp"
import { Memory } from "./memory"
import { saveSession, type SessionData } from "./session"
import { Skills } from "./skills"
import { Store } from "./ui/store"
import { fmtCost } from "./ui/theme"

export interface PrintOptions {
  prompt: string
  settings: Settings
  session?: SessionData
  cwd: string
  json: boolean
  quiet: boolean
  /** `provider/slug` from -m, applied after a resumed session's model. */
  model?: string
}

const dim = (s: string) => (process.stderr.isTTY ? `\x1b[2m${s}\x1b[0m` : s)
const red = (s: string) => (process.stderr.isTTY ? `\x1b[31m${s}\x1b[0m` : s)

export async function runPrint(o: PrintOptions): Promise<number> {
  ;(globalThis as any).AI_SDK_LOG_WARNINGS = false
  // `oah -p … | head` closes stdout early: stop quietly like other CLI tools.
  process.stdout.on("error", (e: any) => {
    if (e?.code === "EPIPE") process.exit(0)
  })
  const err = (line: string) => {
    if (!o.quiet) process.stderr.write(line + "\n")
  }
  const mcp = new Mcp(o.cwd, () => {})
  const store = new Store(o.settings, o.session, o.cwd, new Skills(o.cwd), new Memory(o.cwd), mcp, () => false)
  if (o.model) store.settings.model = o.model
  const denied: string[] = []
  store.headless = {
    ask(req) {
      denied.push(req.title)
      err(red(`✗ ${req.title} — needs approval (${req.kind}); denied. Rerun with --permission allow to let it run.`))
      return "deny"
    },
    question(q) {
      err(dim(`? ${q.question} — no one to answer; the model will choose`))
      return "(This is a non-interactive run and no one can answer. Pick the most reasonable option, say which one you chose, and continue.)"
    },
  }
  mcp.start()
  await Promise.all([mcp.settled(), store.refreshLocal()])
  if (o.model) store.settings.model = o.model

  // Stream new transcript entries: answer text to stdout, everything else to stderr.
  const start = store.session.transcript.length
  const printed = new Map<number, number>()
  let failed = false
  let lastOut: "text" | "other" | null = null
  const flush = (final = false) => {
    const t = store.session.transcript
    for (let i = start; i < t.length; i++) {
      const e = t[i]!
      if (e.kind === "assistant") {
        if (o.json) continue
        const done = printed.get(i) ?? -1
        if (done < 0 && lastOut === "text") process.stdout.write("\n\n")
        const next = e.text.slice(Math.max(0, done))
        if (next) process.stdout.write(next)
        printed.set(i, e.text.length)
        lastOut = "text"
      } else if (!printed.has(i)) {
        if (e.kind === "tool") {
          if (e.status === "running" && !final) break // keep order: wait for it to finish
          const mark = e.status === "ok" ? "✓" : e.status === "denied" ? "⊘" : "✗"
          if (lastOut === "text") process.stdout.write("\n")
          err(dim(`${mark} ${e.title}${e.summary ? ` — ${e.summary}` : ""}`))
        } else if (e.kind === "error") {
          failed = true
          if (lastOut === "text") process.stdout.write("\n")
          process.stderr.write(red(`oah: ${e.text}`) + "\n")
        } else if (e.kind === "info") err(dim(`※ ${e.text.replace(/\*\*/g, "")}`))
        else if (e.kind === "user" && i > start) err(dim(`› ${e.text}`))
        printed.set(i, 1)
        if (e.kind !== "reasoning" && e.kind !== "user") lastOut = "other"
      }
    }
  }
  store.subscribe(() => flush())

  let interrupted = false
  const onSigint = () => {
    interrupted = true
    store.busy?.abort.abort()
    process.stderr.write("\n" + red("oah: interrupted") + "\n")
  }
  process.on("SIGINT", onSigint)
  store.sendUser(o.prompt)
  await store.whenIdle()
  process.off("SIGINT", onSigint)
  flush(true)
  if (lastOut === "text" && !o.json) process.stdout.write("\n")

  saveSession(store.session)
  await mcp.close().catch(() => {})
  const cost = store.session.cost ?? 0
  const answer = store.session.transcript.slice(start).filter((e) => e.kind === "assistant").map((e) => (e.kind === "assistant" ? e.text : "")).join("\n\n")
  if (o.json) {
    const errors = store.session.transcript.slice(start).filter((e) => e.kind === "error").map((e) => (e.kind === "error" ? e.text : ""))
    process.stdout.write(
      JSON.stringify({ result: answer, session: store.session.id, model: store.settings.model, cost: Math.round(cost * 1e6) / 1e6, contextTokens: store.session.contextTokens, totalTokens: store.session.totalTokens, usage: store.session.usage, lastUsage: store.session.lastUsage, usageScope: "main-agent", verification: store.session.verification, denied, errors }, null, 2) + "\n",
    )
  } else err(dim(`— ${displayName(store.settings.model)}${cost ? ` · ${fmtCost(cost)}` : ""} · session ${store.session.id} · continue with: oah -c -p "…"`))
  return interrupted ? 130 : failed ? 1 : store.session.verification?.status === "failed" || store.session.verification?.status === "unverified" ? 2 : 0
}

/**
 * Read piped stdin. With a prompt already given, stdin is optional context: if
 * nothing arrives within `waitMs` (an idle pipe in CI, say) it's ignored rather
 * than waited on forever.
 */
export async function readStdin(waitMs?: number): Promise<string> {
  if (process.stdin.isTTY) return ""
  const reader = Bun.stdin.stream().getReader()
  const chunks: Uint8Array[] = []
  const first = reader.read()
  const got = waitMs === undefined ? await first : await Promise.race([first, new Promise<null>((r) => setTimeout(() => r(null), waitMs))])
  if (!got) {
    reader.releaseLock?.()
    return ""
  }
  if (got.done) return ""
  chunks.push(got.value)
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    chunks.push(value)
  }
  return Buffer.concat(chunks).toString("utf8")
}
