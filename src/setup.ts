// oah configuring itself: installing skills, adding MCP servers, and importing
// MCP servers / skills / sessions from Claude Code. Used by /import and by the
// `oah_setup` tool.
import { cpSync, existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { homedir, tmpdir } from "node:os"
import { basename, dirname, join, resolve } from "node:path"
import type { ModelMessage } from "ai"
import { ensureDir, paths, readJson, shortPath, writeFileAtomic, type Effort, type Mode } from "./config"
import { writeServer, type ServerConfig } from "./mcp"
import type { Entry, SessionData } from "./session"
import { parseFrontMatter } from "./skills"

const oneLine = (s: string, max: number) => {
  const t = s.replace(/\s+/g, " ").trim()
  return t.length > max ? t.slice(0, max - 1) + "…" : t
}

// ---- skills ---------------------------------------------------------------------------

export const skillRoot = (scope: "global" | "project", cwd: string) => (scope === "global" ? paths.skills : join(cwd, ".oah", "skills"))

interface FoundSkill {
  name: string
  dir: string
}

/** Folders containing a SKILL.md under `root` (depth ≤ 5, skipping .git/node_modules). */
export function findSkills(root: string): FoundSkill[] {
  const out: FoundSkill[] = []
  const visit = (d: string, depth: number) => {
    if (depth > 5 || out.length > 500) return
    let entries
    try {
      entries = readdirSync(d, { withFileTypes: true })
    } catch {
      return
    }
    if (entries.some((e) => e.isFile() && e.name === "SKILL.md")) {
      const { meta } = parseFrontMatter(readFileSync(join(d, "SKILL.md"), "utf8"))
      out.push({ name: sanitizeName(meta.name ?? basename(d)), dir: d })
      return
    }
    for (const e of entries) if (e.isDirectory() && e.name !== ".git" && e.name !== "node_modules") visit(join(d, e.name), depth + 1)
  }
  visit(root, 0)
  return out
}

const sanitizeName = (n: string) => n.trim().replace(/[^A-Za-z0-9._-]+/g, "-").replace(/^-+|-+$/g, "") || "skill"

function copySkill(s: FoundSkill, root: string, overwrite = true): string | null {
  const dest = join(root, s.name)
  if (existsSync(dest)) {
    if (!overwrite) return null
    rmSync(dest, { recursive: true, force: true })
  }
  ensureDir(root)
  cpSync(s.dir, dest, { recursive: true, filter: (src) => !/(^|\/)(\.git|node_modules)(\/|$)/.test(src.slice(s.dir.length)) })
  return dest
}

/** "owner/repo", "https://github.com/owner/repo[/tree/<ref>/<path>]", or any git URL → clone info. */
function gitSource(src: string): { url: string; ref?: string; sub?: string } | null {
  const short = /^([A-Za-z0-9_.-]+)\/([A-Za-z0-9_.-]+)(?:\/(.+))?$/.exec(src)
  if (short && !src.startsWith(".") && !existsSync(src)) return { url: `https://github.com/${short[1]}/${short[2]!.replace(/\.git$/, "")}.git`, sub: short[3] }
  const gh = /^https?:\/\/github\.com\/([^/]+)\/([^/#?]+)(?:\/(?:tree|blob)\/([^/]+)(?:\/(.*))?)?/.exec(src)
  if (gh) {
    let sub = gh[4]?.replace(/\/$/, "")
    if (sub?.endsWith("SKILL.md")) sub = dirname(sub)
    return { url: `https://github.com/${gh[1]}/${gh[2]!.replace(/\.git$/, "")}.git`, ref: gh[3], sub: sub === "." ? undefined : sub }
  }
  if (/^(git@|https?:\/\/).+\.git$/.test(src)) return { url: src }
  return null
}

export interface InstallResult {
  installed: { name: string; path: string }[]
  /** Set when nothing was installed: why, or which names to choose from. */
  message?: string
}

/**
 * Install skills from a GitHub repo ("owner/repo", a /tree/ URL), a git URL, a raw
 * SKILL.md URL, or a local folder. With several skills in the source, `name`
 * picks one (or `all`); otherwise the choices are returned.
 */
export async function installSkill(source: string, o: { name?: string; all?: boolean; scope: "global" | "project"; cwd: string; signal?: AbortSignal }): Promise<InstallResult> {
  const root = skillRoot(o.scope, o.cwd)
  source = source.trim()
  // Raw SKILL.md over HTTP.
  if (/^https?:\/\//.test(source) && !gitSource(source)) {
    const r = await fetch(source, { signal: o.signal, headers: { "User-Agent": "oah" } })
    if (!r.ok) return { installed: [], message: `HTTP ${r.status} fetching ${source}` }
    const text = await r.text()
    const { meta } = parseFrontMatter(text)
    if (!meta.name && !o.name) return { installed: [], message: "that file has no `name:` front matter — pass a name" }
    const name = sanitizeName(o.name ?? meta.name!)
    const dest = join(root, name)
    rmSync(dest, { recursive: true, force: true })
    ensureDir(dest)
    writeFileSync(join(dest, "SKILL.md"), text)
    return { installed: [{ name, path: dest }] }
  }
  let base: string
  let tmp: string | null = null
  const git = gitSource(source)
  try {
    if (git) {
      tmp = mkdtempSync(join(tmpdir(), "oah-skill-"))
      const args = ["git", "clone", "--depth", "1", "--quiet", ...(git.ref ? ["--branch", git.ref] : []), git.url, tmp]
      const p = Bun.spawn(args, { stdout: "ignore", stderr: "pipe", env: { ...process.env, GIT_TERMINAL_PROMPT: "0" } })
      o.signal?.addEventListener("abort", () => p.kill(), { once: true })
      const err = await new Response(p.stderr).text()
      if ((await p.exited) !== 0) return { installed: [], message: `git clone ${git.url} failed: ${oneLine(err, 300)}` }
      base = git.sub ? join(tmp, git.sub) : tmp
    } else {
      base = resolve(o.cwd, source.replace(/^~(?=\/)/, homedir()))
      if (base.endsWith("SKILL.md")) base = dirname(base)
    }
    if (!existsSync(base)) return { installed: [], message: `${source}: not found` }
    let found = findSkills(base)
    if (!found.length) return { installed: [], message: `no SKILL.md found in ${source}` }
    if (o.name) {
      const want = o.name.toLowerCase()
      const exact = found.filter((f) => f.name.toLowerCase() === want || basename(f.dir).toLowerCase() === want)
      found = exact.length ? exact : found.filter((f) => f.name.toLowerCase().includes(want))
      if (!found.length) return { installed: [], message: `no skill named '${o.name}' in ${source}` }
    }
    if (found.length > 1 && !o.all) return { installed: [], message: `${source} has ${found.length} skills — choose one with name, or pass all: true:\n${found.map((f) => `- ${f.name}`).join("\n")}` }
    return { installed: found.map((f) => ({ name: f.name, path: copySkill(f, root)! })) }
  } finally {
    if (tmp) rmSync(tmp, { recursive: true, force: true })
  }
}

/** Remove a skill installed in oah's own folders (never touches .claude/.agents). */
export function removeSkill(name: string, cwd: string): string | null {
  for (const root of [skillRoot("project", cwd), skillRoot("global", cwd)]) {
    const hit = findSkills(root).find((s) => s.name.toLowerCase() === name.toLowerCase() || basename(s.dir).toLowerCase() === name.toLowerCase())
    if (hit && dirname(hit.dir) === root) {
      rmSync(hit.dir, { recursive: true, force: true })
      return hit.dir
    }
  }
  return null
}

// ---- MCP ------------------------------------------------------------------------------

export const mcpFile = (scope: "global" | "project", cwd: string) => (scope === "global" ? paths.mcp : join(cwd, ".oah", "mcp.json"))

export function addMcpServer(name: string, cfg: ServerConfig, scope: "global" | "project", cwd: string): string {
  const file = mcpFile(scope, cwd)
  writeServer(file, name, cfg)
  return file
}

/** Remove a server from oah's config files; returns the files it was removed from. */
export function removeMcpServer(name: string, cwd: string): string[] {
  const out: string[] = []
  for (const f of [paths.mcp, join(cwd, ".oah", "mcp.json"), join(cwd, ".mcp.json")]) {
    const raw = readJson<any>(f, null)
    if (raw?.mcpServers?.[name] || raw?.servers?.[name]) {
      writeServer(f, name, null)
      out.push(f)
    }
  }
  return out
}

/** Claude Code server entry → oah's (drops `type: "stdio"`, keeps the rest). */
function fromClaude(c: any, pluginRoot?: string): ServerConfig | null {
  const sub = (v: any): any =>
    typeof v === "string" ? (pluginRoot ? v.replaceAll("${CLAUDE_PLUGIN_ROOT}", pluginRoot) : v) : Array.isArray(v) ? v.map(sub) : v && typeof v === "object" ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, sub(x)])) : v
  c = sub(c)
  if (c?.url) return { url: c.url, ...(c.type === "sse" ? { type: "sse" as const } : {}), ...(c.headers ? { headers: c.headers } : {}) }
  if (c?.command) return { command: c.command, ...(c.args?.length ? { args: c.args } : {}), ...(c.env && Object.keys(c.env).length ? { env: c.env } : {}), ...(c.cwd ? { cwd: c.cwd } : {}) }
  return null
}

// ---- Claude Code import ---------------------------------------------------------------

const claudeDir = () => process.env.CLAUDE_CONFIG_DIR || join(homedir(), ".claude")

function plugins(): { name: string; root: string }[] {
  const list = readJson<any>(join(claudeDir(), "plugins", "installed_plugins.json"), null)
  const out: { name: string; root: string }[] = []
  for (const [id, installs] of Object.entries<any[]>(list?.plugins ?? {})) {
    const root = installs?.at(-1)?.installPath
    if (root && existsSync(root)) out.push({ name: id.split("@")[0]!, root })
  }
  return out
}

function pluginServers(root: string): Record<string, any> {
  const manifest = readJson<any>(join(root, ".claude-plugin", "plugin.json"), {})
  let m = manifest?.mcpServers
  if (typeof m === "string") m = readJson<any>(resolve(root, m), {})
  if (!m) m = readJson<any>(join(root, ".mcp.json"), {})
  return m?.mcpServers ?? m ?? {}
}

export interface ImportReport {
  lines: string[]
  mcpChanged: boolean
  skillsChanged: boolean
}

export function importFromClaude(o: { cwd: string; what: Set<"mcp" | "skills" | "sessions">; model: string; effort: Effort; mode: Mode }): ImportReport {
  const lines: string[] = []
  let mcpChanged = false
  let skillsChanged = false
  const cdir = claudeDir()
  if (!existsSync(cdir)) return { lines: [`No Claude Code config found at ${shortPath(cdir)}.`], mcpChanged, skillsChanged }
  const claudeJson = readJson<any>(process.env.CLAUDE_CONFIG_DIR ? join(cdir, ".claude.json") : join(homedir(), ".claude.json"), {})

  if (o.what.has("mcp")) {
    const have = readJson<any>(paths.mcp, {})?.mcpServers ?? {}
    const haveProject = readJson<any>(join(o.cwd, ".oah", "mcp.json"), {})?.mcpServers ?? {}
    const added: string[] = []
    const skipped: string[] = []
    const add = (name: string, raw: any, scope: "global" | "project", root?: string) => {
      const cfg = fromClaude(raw, root)
      if (!cfg) return
      if ((scope === "global" ? have : haveProject)[name]) return void skipped.push(name)
      addMcpServer(name, cfg, scope, o.cwd)
      added.push(scope === "project" ? `${name} (this project)` : name)
    }
    for (const [name, c] of Object.entries<any>(claudeJson.mcpServers ?? {})) add(name, c, "global")
    for (const p of plugins()) for (const [name, c] of Object.entries<any>(pluginServers(p.root))) add(name, c, "global", p.root)
    const local = claudeJson.projects?.[o.cwd]?.mcpServers ?? {}
    for (const [name, c] of Object.entries<any>(local)) add(name, c, "project")
    const elsewhere = Object.entries<any>(claudeJson.projects ?? {}).filter(([p, v]) => p !== o.cwd && Object.keys(v?.mcpServers ?? {}).length).length
    mcpChanged = added.length > 0
    lines.push(`**MCP servers:** ${added.length ? `added ${added.join(", ")}` : "nothing new"}${skipped.length ? ` · already set up: ${skipped.join(", ")}` : ""}`)
    if (elsewhere) lines.push(`  ${elsewhere} other project${elsewhere === 1 ? " has" : "s have"} project-only servers — run /import mcp from ${elsewhere === 1 ? "that folder" : "those folders"}.`)
    if (existsSync(join(o.cwd, ".mcp.json"))) lines.push("  This project's `.mcp.json` is read by oah directly.")
  }

  if (o.what.has("skills")) {
    const sources: FoundSkill[] = []
    for (const p of plugins()) sources.push(...findSkills(join(p.root, "skills")))
    const synced = join(cdir, "skills", "synced")
    if (existsSync(synced)) sources.push(...findSkills(synced))
    const added: string[] = []
    const skipped: string[] = []
    for (const s of sources) {
      if (copySkill(s, paths.skills, false)) added.push(s.name)
      else skipped.push(s.name)
    }
    skillsChanged = added.length > 0
    const direct = findSkills(join(cdir, "skills")).filter((s) => !s.dir.includes("/synced/")).length
    lines.push(`**Skills:** ${added.length ? `copied ${added.length} from Claude plugins and synced skills to ${shortPath(paths.skills)}` : "nothing new"}${skipped.length ? ` · ${skipped.length} already there` : ""}`)
    if (direct) lines.push(`  ${direct} skill${direct === 1 ? "" : "s"} in ${shortPath(join(cdir, "skills"))} ${direct === 1 ? "is" : "are"} read by oah directly.`)
  }

  if (o.what.has("sessions")) {
    let imported = 0
    let skipped = 0
    let empty = 0
    const projects = join(cdir, "projects")
    let dirs: string[] = []
    try {
      dirs = readdirSync(projects)
    } catch {}
    for (const d of dirs) {
      let files: string[] = []
      try {
        files = readdirSync(join(projects, d)).filter((f) => f.endsWith(".jsonl"))
      } catch {
        continue
      }
      for (const f of files) {
        const uuid = f.slice(0, -6)
        const id = `cc${uuid.replace(/-/g, "").slice(0, 10)}`
        if (existsSync(join(paths.sessions, `${id}.json`))) {
          skipped++
          continue
        }
        try {
          const s = convertSession(join(projects, d, f), id, uuid, o)
          if (!s) {
            empty++
            continue
          }
          // Written directly (not saveSession) to keep the original `updated` time for /session ordering.
          writeFileAtomic(join(paths.sessions, `${id}.json`), JSON.stringify(s), true)
          imported++
        } catch {
          empty++
        }
      }
    }
    lines.push(`**Sessions:** imported ${imported}${skipped ? ` · ${skipped} already imported` : ""}${empty ? ` · ${empty} empty or unreadable skipped` : ""} — open them with /session`)
  }
  return { lines, mcpChanged, skillsChanged }
}

const REMINDER = /<system-reminder>[\s\S]*?<\/system-reminder>/g

function userText(c: string): string | null {
  if (/^<local-command-(stdout|stderr|caveat)>/.test(c.trim())) return null
  const name = /<command-name>([\s\S]*?)<\/command-name>/.exec(c)?.[1]
  if (name) {
    const args = /<command-args>([\s\S]*?)<\/command-args>/.exec(c)?.[1] ?? ""
    return `${name.trim()} ${args.trim()}`.trim()
  }
  const t = c.replace(REMINDER, "").trim()
  return t || null
}

const blockText = (c: any): string =>
  typeof c === "string" ? c : Array.isArray(c) ? c.map((b) => (b?.type === "text" ? b.text : b?.type === "image" ? "[image]" : "")).filter(Boolean).join("\n") : ""

function toolTitle(name: string, input: any): string {
  const v = input?.command ?? input?.file_path ?? input?.path ?? input?.pattern ?? input?.url ?? input?.description ?? input?.prompt ?? ""
  return `${name} ${oneLine(String(v), 100)}`.trim()
}

/**
 * Claude Code session (.jsonl) → oah session. Tool calls and results are kept as
 * text in the model history (other providers can't replay Claude's tool blocks),
 * and as tool rows in the transcript.
 */
function convertSession(file: string, id: string, uuid: string, o: { model: string; effort: Effort; mode: Mode }): SessionData | null {
  const transcript: Entry[] = []
  const msgs: { role: "user" | "assistant"; text: string }[] = []
  const toolNames = new Map<string, string>()
  let title = ""
  let firstPrompt = ""
  let cwd = ""
  let first = 0
  let last = 0
  const say = (role: "user" | "assistant", text: string) => {
    if (!text.trim()) return
    const prev = msgs.at(-1)
    if (prev?.role === role) prev.text += `\n\n${text}`
    else msgs.push({ role, text })
  }
  for (const line of readFileSync(file, "utf8").split("\n")) {
    if (!line) continue
    let j: any
    try {
      j = JSON.parse(line)
    } catch {
      continue
    }
    if (j.type === "ai-title" && j.aiTitle) title = j.aiTitle
    if (j.type === "summary" && j.summary && !title) title = j.summary
    if ((j.type !== "user" && j.type !== "assistant") || j.isSidechain) continue
    const at = Date.parse(j.timestamp ?? "") || 0
    if (at) {
      first ||= at
      last = at
    }
    cwd ||= j.cwd ?? ""
    const content = j.message?.content
    if (j.type === "user") {
      if (j.isMeta) continue
      if (typeof content === "string") {
        const t = userText(content)
        if (!t) continue
        firstPrompt ||= t
        transcript.push({ kind: "user", text: t, at })
        say("user", t)
        continue
      }
      for (const b of Array.isArray(content) ? content : []) {
        if (b.type === "text") {
          const t = userText(b.text ?? "")
          if (!t) continue
          firstPrompt ||= t
          transcript.push({ kind: "user", text: t, at })
          say("user", t)
        } else if (b.type === "image") {
          say("user", "[image]")
        } else if (b.type === "tool_result") {
          const out = blockText(b.content)
          const row = transcript.findLast((e) => e.kind === "tool" && e.callId === b.tool_use_id)
          if (row && row.kind === "tool") {
            row.status = b.is_error ? "failed" : "ok"
            row.summary = oneLine(out.split("\n").find((l) => l.trim()) ?? "", 80)
            row.ms = Math.max(0, at - row.started)
          }
          const name = toolNames.get(b.tool_use_id) ?? "tool"
          say("user", `[${name} result${b.is_error ? " (error)" : ""}]\n${out.length > 2000 ? out.slice(0, 2000) + "\n…(truncated)" : out}`)
        }
      }
    } else {
      const model = `anthropic/${j.message?.model ?? "claude"}`
      for (const b of Array.isArray(content) ? content : typeof content === "string" ? [{ type: "text", text: content }] : []) {
        if (b.type === "text" && b.text?.trim()) {
          const prev = transcript.at(-1)
          if (prev?.kind === "assistant") prev.text += `\n\n${b.text}`
          else transcript.push({ kind: "assistant", text: b.text, model, at })
          say("assistant", b.text)
        } else if (b.type === "tool_use") {
          toolNames.set(b.id, b.name)
          const t = toolTitle(b.name, b.input)
          transcript.push({ kind: "tool", callId: b.id, name: b.name, title: t, summary: "", detail: [], status: "ok", started: at, at })
          say("assistant", `[called ${b.name}: ${oneLine(JSON.stringify(b.input ?? {}), 400)}]`)
        }
      }
    }
  }
  if (!msgs.some((m) => m.role === "user") || !msgs.some((m) => m.role === "assistant")) return null
  if (msgs[0]!.role !== "user") msgs.unshift({ role: "user", text: "(continuing)" })
  msgs[0]!.text = `[This conversation was imported from Claude Code. Tool calls and their results appear as text; the tools named there were Claude Code's, not yours.]\n\n${msgs[0]!.text}`
  const messages: ModelMessage[] = msgs.map((m) => ({ role: m.role, content: m.text }))
  const chars = msgs.reduce((n, m) => n + m.text.length, 0)
  transcript.unshift({ kind: "info", text: `Imported from Claude Code session ${uuid}` })
  return {
    id,
    created: first || Date.now(),
    updated: last || Date.now(),
    cwd: cwd || process.cwd(),
    title: oneLine(title || firstPrompt || "Imported session", 60),
    model: o.model,
    effort: o.effort,
    mode: o.mode,
    messages,
    transcript,
    contextTokens: Math.round(chars / 4),
    totalTokens: 0,
    todos: [],
    compactions: 0,
  }
}
