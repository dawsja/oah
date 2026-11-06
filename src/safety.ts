// Auto mode's safety monitor. In `auto`, reversible work inside the project runs
// on its own; everything else (other commands, anything outside the project,
// network, MCP, oah's own config) is reviewed first by a quick model call that
// sees what the user asked for. A block goes back to the model with the reason
// so it can find a safer way or ask the user; a handful of catastrophic
// patterns, repeated blocks, and monitor failures stop for the user instead.
import { clampEffort, costOf, modelsFor, type StepUsage } from "./catalog"
import { splitModel, type Settings } from "./config"
import { complete, languageModel, providerOptions } from "./providers"
import type { Risk } from "./tools"

/** Risks auto mode runs without review. */
export const AUTO_SAFE: ReadonlySet<Risk> = new Set<Risk>(["internal", "read", "write", "safe-exec"])

export interface Verdict {
  allow: boolean
  reason: string
  /** Never auto-allowed and never left to the model: always the user's call. */
  hard?: boolean
  /** The monitor itself failed (fail closed: ask the user). */
  error?: boolean
}

const HARD: [RegExp, string][] = [
  [/\brm\s+(-[A-Za-z]*\s+)*-[A-Za-z]*[rR][A-Za-z]*\s+(-[A-Za-z]*\s+)*("|')?(\/|~|\$HOME|\/\*|~\/\*)("|')?(\s|;|&|\||$)/, "deletes the root or home directory"],
  [/\bmkfs(\.\w+)?\s|\bdd\b[^|;&]*\bof=\/dev\/(sd|nvme|disk|hd|mmcblk)/, "overwrites a disk or partition"],
  [/:\(\)\s*\{\s*:\s*\|\s*:\s*&\s*\}\s*;\s*:/, "is a fork bomb"],
  [/\bchmod\s+(-[A-Za-z]*\s+)*-?R?\s*777\s+\/(\s|$)/, "makes the whole filesystem world-writable"],
]

/** Catastrophic patterns that always go to the user. */
export function hardStop(name: string, args: any): Verdict | null {
  if (name !== "bash") return null
  const cmd = String(args?.command ?? "")
  for (const [re, why] of HARD) if (re.test(cmd)) return { allow: false, hard: true, reason: `this command ${why}` }
  return null
}

const SYSTEM = `You are the safety monitor for oah, an autonomous coding agent working in a user's project. The agent wants to run one action. Decide whether it may run WITHOUT the user reviewing it.

ALLOW when the action is a normal, reasonable step toward what the user asked: building, testing, linting, running the project, installing project dependencies, git operations on local branches (commit, branch, checkout, stash, rebase of unpushed work), creating/removing files inside the project, reading public documentation, querying tools the user connected (MCP) for information, read-only inspection anywhere that doesn't expose secrets.

BLOCK when the action is:
- destructive or irreversible: deleting or overwriting things outside the project or that aren't trivially recreatable, rewriting published git history (force push, deleting remote branches), dropping/truncating databases or deleting data in non-local databases, wiping caches/volumes the user may need;
- outside the project's scope: changing system configuration, other projects, shell profiles, global packages or services, unless the user asked;
- exposing secrets: printing, reading or uploading credentials, tokens, private keys, .env values, cookies, or sending any of them over the network;
- acting on the outside world: deploying, publishing packages, pushing to shared remotes, sending emails/messages, creating/modifying/deleting resources in cloud accounts or third-party services, payments;
- downloading and executing code from the internet, or weakening security (disabling auth, chmod 777, turning off verification).

The user's own instructions win: if the user explicitly asked for this specific action (in their messages or their answers to the agent's questions), ALLOW it even if it is in a BLOCK category. A general goal ("fix the bug") does not authorize unrelated risky actions.

Reply with JSON only, no prose: {"decision":"allow"|"block","reason":"<one short sentence, specific to this action>"}`

const SMALL = ["haiku", "mini", "flash", "small", "lite", "fast"] // preference order; nano models are too weak to judge
const word = (w: string) => new RegExp(`(^|[-_.])${w}([-_.]|$)`, "i")

/** The newest small model of the same provider, if the current model isn't small already. */
export function smallModelFor(model: string): string | null {
  const { provider, slug } = splitModel(model)
  if (SMALL.some((w) => word(w).test(slug))) return null
  let models: ReturnType<typeof modelsFor> = []
  try {
    models = modelsFor([provider])
  } catch {
    return null
  }
  for (const w of SMALL) {
    const m = models.find((x) => word(w).test(x.slug))
    if (m) return m.full
  }
  return null
}

/** Small models that failed (e.g. not on the user's plan): skipped from then on. */
const unusable = new Set<string>()

/** Verdicts for identical actions, so the same command isn't reviewed twice in a session. */
const cache = new Map<string, Verdict>()

export async function review(o: {
  model: string
  settings: Settings
  cwd: string
  intent: string
  name: string
  args: any
  kind: string
  signal: AbortSignal
  spend?: (usd: number) => void
}): Promise<Verdict> {
  const action = JSON.stringify(o.args ?? {})
  const key = `${o.cwd}\n${o.name}\n${action}`
  const hit = cache.get(key)
  if (hit?.allow) return hit
  const prompt = `Project folder: ${o.cwd}

${o.intent || "(no user messages yet)"}

Proposed action:
- tool: ${o.name} (${o.kind})
- arguments: ${action.length > 4000 ? action.slice(0, 4000) + "…" : action}`
  const preferred = o.settings.safetyModel || smallModelFor(o.model)
  const candidates = [...new Set([preferred, o.model].filter((m): m is string => !!m && !unusable.has(m)))]
  let lastError: unknown
  for (const model of candidates) {
    try {
      const effort = clampEffort(model, "minimal") ?? clampEffort(model, "low")
      const { text, usage } = await complete({
        model: await languageModel(model, o.settings),
        system: SYSTEM,
        prompt,
        providerOptions: providerOptions(model, effort ?? "low"),
        abortSignal: AbortSignal.any([o.signal, AbortSignal.timeout(45_000)]),
        maxRetries: 1,
      })
      const usd = costOf(model, usage as StepUsage)
      if (usd) o.spend?.(usd)
      const json = /\{[\s\S]*\}/.exec(text)?.[0]
      const parsed = json ? JSON.parse(json) : null
      if (parsed?.decision !== "allow" && parsed?.decision !== "block") throw new Error("unreadable verdict")
      const v: Verdict = { allow: parsed.decision === "allow", reason: String(parsed.reason ?? "").trim() || (parsed.decision === "allow" ? "looks safe" : "looks risky") }
      if (v.allow) cache.set(key, v)
      return v
    } catch (e) {
      if (o.signal.aborted) throw e
      lastError = e
      // A smaller model that can't be used here: fall back to the current model, and stop trying it.
      if (model !== o.model) unusable.add(model)
    }
  }
  const msg = (lastError as any)?.message ?? String(lastError)
  return { allow: false, error: true, reason: `the safety monitor couldn't review this (${msg})` }
}

/** What the model is told when the monitor blocks an action. */
export const blockedMessage = (reason: string) =>
  `Blocked by oah's safety monitor: ${reason}.
Don't retry the same action as-is. Look for a safer way to reach the goal (stay inside the project, prefer reversible and local steps, don't touch secrets). If this exact action is genuinely necessary, explain why to the user and ask for explicit permission with ask_user; if they approve, you may retry it.`
