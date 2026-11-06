// Built-in tools. Each returns text for the model plus a short summary and
// preview lines for the transcript. Permission gating happens in the agent.
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs"
import { dirname, isAbsolute, join, normalize, relative, resolve } from "node:path"
import { homedir } from "node:os"
import type { Mode } from "../config"
import type { Memory } from "../memory"
import type { Skills } from "../skills"

export type Risk = "internal" | "read" | "read-outside" | "write" | "safe-exec" | "exec" | "external"

export interface Outcome {
  output: string
  summary: string
  detail: string[]
  ok: boolean
}

export interface ToolCtx {
  cwd: string
  signal: AbortSignal
  skills: Skills
  memory: Memory
  mode: Mode
  onTodos: (items: Todo[]) => void
  onGoalComplete: (summary: string) => void
}

export interface Todo {
  text: string
  status: "pending" | "in_progress" | "completed"
}

export interface ToolDef {
  name: string
  description: string
  parameters: Record<string, any>
  /** Only offered in build mode. */
  buildOnly?: boolean
  run: (args: any, ctx: ToolCtx) => Promise<Outcome> | Outcome
}

const MAX_OUTPUT = 30_000
const MAX_READ_LINES = 2000

const ok = (output: string, summary: string, detail: string[] = []): Outcome => ({ output, summary, detail, ok: true })
const fail = (msg: string): Outcome => ({ output: `Error: ${msg}`, summary: msg, detail: [], ok: false })
const obj = (props: Record<string, any>, required: string[] = []) => ({ type: "object", properties: props, required, additionalProperties: false })
const str = (description?: string) => ({ type: "string", ...(description ? { description } : {}) })

export function resolvePath(cwd: string, p: string): string {
  p = (p ?? "").trim()
  if (p.startsWith("~/")) p = join(homedir(), p.slice(2))
  return normalize(isAbsolute(p) ? p : resolve(cwd, p))
}

export const inside = (cwd: string, p: string) => {
  const r = relative(cwd, p)
  return r === "" || (!r.startsWith("..") && !isAbsolute(r))
}

export function rel(cwd: string, p: string): string {
  return inside(cwd, p) ? relative(cwd, p) || "." : p.replace(homedir(), "~")
}

export function oneLine(s: string, max: number): string {
  const flat = s.split(/\s+/).filter(Boolean).join(" ")
  return flat.length > max ? flat.slice(0, max - 1) + "…" : flat
}

function truncate(s: string): string {
  if (s.length <= MAX_OUTPUT) return s
  const half = MAX_OUTPUT / 2
  return `${s.slice(0, half)}\n\n… [${s.length - MAX_OUTPUT} chars omitted] …\n\n${s.slice(-half)}`
}

function preview(s: string, n: number): string[] {
  const lines = s.split("\n")
  const out = lines.slice(0, n).map((l) => (l.length > 200 ? l.slice(0, 199) + "…" : l).replace(/\t/g, "    "))
  if (lines.length > n) out.push(`… +${lines.length - n} lines`)
  return out
}

export function diffLines(oldS: string, newS: string, max: number): string[] {
  const a = oldS.split("\n")
  const b = newS.split("\n")
  let start = 0
  while (start < a.length && start < b.length && a[start] === b[start]) start++
  let ea = a.length
  let eb = b.length
  while (ea > start && eb > start && a[ea - 1] === b[eb - 1]) ea--, eb--
  const out = [...a.slice(start, ea).map((l) => `-${l}`), ...b.slice(start, eb).map((l) => `+${l}`)]
  if (out.length > max) return [...out.slice(0, max), `… +${out.length - max} lines`]
  return out
}

export function stripAnsi(s: string): string {
  return s.replace(/\x1b\[[0-9;?]*[ -/]*[@-~]/g, "").replace(/\x1b\][^\x07\x1b]*(\x07|\x1b\\)/g, "").replace(/\r(?!\n)/g, "").replace(/\r\n/g, "\n")
}

/** Commands that only inspect state (auto permission, plan mode). */
export function isSafeCommand(cmd: string, extra: string[] = []): boolean {
  const c = cmd.trim()
  if (!c || /[>`]|\$\(/.test(c)) return false
  const SAFE = new Set("ls pwd cat head tail wc echo grep rg fd find tree stat file which whoami date uname env printenv du df sort uniq cut tr diff less more basename dirname realpath readlink type true nl column jq sha256sum md5sum".split(" "))
  const GIT = new Set("status diff log show branch remote rev-parse ls-files blame describe tag".split(" "))
  return c
    .split(/[|;&]/)
    .map((s) => s.trim())
    .filter(Boolean)
    .every((seg) => {
      if (extra.some((e) => seg.startsWith(e))) return true
      const [first = "", second = ""] = seg.split(/\s+/)
      if (first === "find" && /-exec|-delete|-ok/.test(seg)) return false
      if (first === "git") return GIT.has(second) && !/branch\s+-[dD]/.test(seg)
      if (first === "cargo") return ["check", "metadata", "tree", "--version"].includes(second)
      return SAFE.has(first)
    })
}

export function risk(name: string, args: any, cwd: string, extraSafe: string[]): Risk {
  switch (name) {
    case "todo_write":
    case "memory":
    case "load_skill":
    case "goal_complete":
      return "internal"
    case "read_file":
    case "list_dir":
    case "glob":
    case "grep":
      return !args?.path || inside(cwd, resolvePath(cwd, args.path)) ? "read" : "read-outside"
    case "write_file":
    case "edit_file":
      return "write"
    case "bash":
      return isSafeCommand(String(args?.command ?? ""), extraSafe) ? "safe-exec" : "exec"
    case "web_fetch":
      return "external"
    default:
      return "external"
  }
}

/** Short human title, e.g. `Bash cargo test`. */
export function title(name: string, args: any, cwd: string): string {
  const path = () => (args?.path ? rel(cwd, resolvePath(cwd, args.path)) : ".")
  switch (name) {
    case "read_file":
      return `Read ${path()}`
    case "list_dir":
      return `List ${path()}`
    case "glob":
      return `Glob ${args?.pattern ?? ""}`
    case "grep":
      return `Grep "${args?.pattern ?? ""}"${args?.path ? ` in ${path()}` : ""}`
    case "write_file":
      return `Write ${path()}`
    case "edit_file":
      return `Edit ${path()}`
    case "bash":
      return `Bash ${oneLine(String(args?.command ?? ""), 120)}`
    case "web_fetch":
      return `Fetch ${args?.url ?? ""}`
    case "todo_write":
      return "Update todos"
    case "memory":
      return `Memory ${args?.action ?? ""}`
    case "load_skill":
      return `Skill ${args?.name ?? ""}`
    case "goal_complete":
      return "Goal complete"
  }
  if (name.startsWith("mcp__")) {
    const [, server, tool] = name.split("__")
    const a = args && Object.keys(args).length ? oneLine(JSON.stringify(args), 100) : ""
    return `MCP ${server}/${tool} ${a}`.trim()
  }
  return name
}

/** What a permission prompt previews before a call runs. */
export function permissionPreview(name: string, args: any): string[] {
  if (name === "edit_file") return diffLines(String(args?.old_string ?? ""), String(args?.new_string ?? ""), 10)
  if (name === "write_file") {
    const lines = String(args?.content ?? "").split("\n")
    return [...lines.slice(0, 8).map((l) => `+${l}`), ...(lines.length > 8 ? [`… +${lines.length - 8} lines`] : [])]
  }
  if (name === "bash") {
    const c = String(args?.command ?? "")
    return c.includes("\n") || c.length > 100 ? c.split("\n").slice(0, 8).map((l) => `$ ${l}`) : []
  }
  if (name.startsWith("mcp__")) return JSON.stringify(args ?? {}, null, 2).split("\n").slice(0, 8)
  return []
}

// ---------------------------------------------------------------------------
// Implementations

function walk(root: string, signal: AbortSignal, onFile: (path: string) => boolean | void) {
  const ignored = loadGitignore(root)
  const stack = [root]
  while (stack.length) {
    if (signal.aborted) return
    const dir = stack.pop()!
    let entries: import("node:fs").Dirent[]
    try {
      entries = readdirSync(dir, { withFileTypes: true })
    } catch {
      continue
    }
    for (const e of entries) {
      if (e.name === ".git" || e.name === "node_modules" || e.name === "target") continue
      const p = join(dir, e.name)
      const r = relative(root, p)
      if (ignored.some((g) => g.test(r) || g.test(e.name))) continue
      if (e.isDirectory()) stack.push(p)
      else if (e.isFile() && onFile(p) === false) return
    }
  }
}

function loadGitignore(root: string): RegExp[] {
  try {
    return readFileSync(join(root, ".gitignore"), "utf8")
      .split("\n")
      .map((l) => l.trim())
      .filter((l) => l && !l.startsWith("#") && !l.startsWith("!"))
      .map((l) => globToRegex(l.replace(/^\//, "").replace(/\/$/, "")))
  } catch {
    return []
  }
}

export function globToRegex(glob: string): RegExp {
  let re = ""
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i]!
    if (c === "*") {
      if (glob[i + 1] === "*") {
        re += glob[i + 2] === "/" ? "(?:.*/)?" : ".*"
        i += glob[i + 2] === "/" ? 2 : 1
      } else re += "[^/]*"
    } else if (c === "?") re += "[^/]"
    else if (c === "{") {
      const end = glob.indexOf("}", i)
      re += "(?:" + glob.slice(i + 1, end).split(",").map((s) => s.replace(/[.+^$()|[\]\\]/g, "\\$&")).join("|") + ")"
      i = end
    } else re += c.replace(/[.+^$()|[\]\\]/g, "\\$&")
  }
  return new RegExp(`^${re}$`)
}

const isBinary = (b: Buffer) => b.subarray(0, 8192).includes(0)

export const TOOLS: ToolDef[] = [
  {
    name: "read_file",
    description: "Read a text file. Returns numbered lines. Use offset/limit (1-based line numbers) for large files.",
    parameters: obj({ path: str("File path, absolute or relative to the workspace"), offset: { type: "integer", description: "First line (1-based)" }, limit: { type: "integer", description: "Maximum lines (default 2000)" } }, ["path"]),
    run(a, ctx) {
      const p = resolvePath(ctx.cwd, a.path)
      let buf: Buffer
      try {
        buf = readFileSync(p)
      } catch (e: any) {
        return fail(`${rel(ctx.cwd, p)}: ${e.code ?? e.message}`)
      }
      if (isBinary(buf)) return fail(`${rel(ctx.cwd, p)} looks like a binary file (${buf.length} bytes)`)
      const lines = buf.toString("utf8").split("\n")
      if (lines.at(-1) === "") lines.pop()
      const offset = Math.max(1, Number(a.offset) || 1)
      const limit = Math.min(MAX_READ_LINES, Number(a.limit) || MAX_READ_LINES)
      const slice = lines.slice(offset - 1, offset - 1 + limit)
      let out = slice.map((l, i) => `${String(offset + i).padStart(6)}\t${l.length > 2000 ? l.slice(0, 2000) + "…" : l}`).join("\n")
      const rest = lines.length - (offset - 1 + slice.length)
      if (rest > 0) out += `\n… (${rest} more lines; use offset=${offset + slice.length})`
      return ok(lines.length ? out : "(empty file)", `${slice.length} of ${lines.length} lines`)
    },
  },
  {
    name: "list_dir",
    description: "List a directory (non-recursive). Directories end with '/'.",
    parameters: obj({ path: str("Directory (default: workspace root)") }),
    run(a, ctx) {
      const p = a.path ? resolvePath(ctx.cwd, a.path) : ctx.cwd
      try {
        const entries = readdirSync(p, { withFileTypes: true }).map((e) => (e.isDirectory() ? `${e.name}/` : e.name)).sort()
        return ok(entries.join("\n") || "(empty)", `${entries.length} entries`)
      } catch (e: any) {
        return fail(`${rel(ctx.cwd, p)}: ${e.code ?? e.message}`)
      }
    },
  },
  {
    name: "glob",
    description: "Find files by glob pattern (e.g. 'src/**/*.ts'), respecting .gitignore. Returns up to 500 paths.",
    parameters: obj({ pattern: str(), path: str("Directory to search (default: workspace root)") }, ["pattern"]),
    run(a, ctx) {
      const root = a.path ? resolvePath(ctx.cwd, a.path) : ctx.cwd
      const re = globToRegex(String(a.pattern))
      const nameOnly = !String(a.pattern).includes("/")
      const out: string[] = []
      walk(root, ctx.signal, (p) => {
        const r = relative(root, p)
        if (re.test(r) || (nameOnly && re.test(p.split("/").pop()!))) out.push(rel(ctx.cwd, p))
        return out.length < 500
      })
      out.sort()
      return out.length ? ok(out.join("\n"), `${out.length} files`) : ok("No files matched.", "no matches")
    },
  },
  {
    name: "grep",
    description: "Search file contents with a regular expression (JavaScript syntax), respecting .gitignore. Returns 'path:line: text' (max 300).",
    parameters: obj({ pattern: str(), path: str("File or directory (default: workspace root)"), glob: str("Only files matching this glob, e.g. '*.ts'"), case_insensitive: { type: "boolean" } }, ["pattern"]),
    run(a, ctx) {
      let re: RegExp
      try {
        re = new RegExp(String(a.pattern), a.case_insensitive ? "i" : "")
      } catch (e: any) {
        return fail(`bad regex: ${e.message}`)
      }
      const filter = a.glob ? globToRegex(String(a.glob)) : null
      const root = a.path ? resolvePath(ctx.cwd, a.path) : ctx.cwd
      const out: string[] = []
      let files = 0
      const search = (p: string) => {
        if (filter && !filter.test(p.split("/").pop()!) && !filter.test(relative(root, p))) return true
        let buf: Buffer
        try {
          if (statSync(p).size > 4 << 20) return true
          buf = readFileSync(p)
        } catch {
          return true
        }
        if (isBinary(buf)) return true
        let hit = false
        const lines = buf.toString("utf8").split("\n")
        for (let i = 0; i < lines.length; i++) {
          if (re.test(lines[i]!)) {
            hit = true
            out.push(`${rel(ctx.cwd, p)}:${i + 1}: ${oneLine(lines[i]!, 300)}`)
            if (out.length >= 300) return false
          }
        }
        if (hit) files++
        return true
      }
      if (existsSync(root) && statSync(root).isFile()) search(root)
      else walk(root, ctx.signal, search)
      return out.length ? ok(out.join("\n"), `${out.length} matches in ${files} files`) : ok("No matches.", "no matches")
    },
  },
  {
    name: "write_file",
    description: "Create or overwrite a file with the given content. Parent directories are created.",
    parameters: obj({ path: str(), content: str() }, ["path", "content"]),
    buildOnly: true,
    run(a, ctx) {
      const p = resolvePath(ctx.cwd, a.path)
      const existed = existsSync(p)
      const old = existed ? readFileSync(p, "utf8") : ""
      try {
        mkdirSync(dirname(p), { recursive: true })
        writeFileSync(p, String(a.content))
      } catch (e: any) {
        return fail(`${rel(ctx.cwd, p)}: ${e.code ?? e.message}`)
      }
      const n = String(a.content).split("\n").length
      const o = ok(`Wrote ${rel(ctx.cwd, p)} (${n} lines).`, existed ? `overwrote · ${n} lines` : `created · ${n} lines`)
      o.detail = existed ? diffLines(old, String(a.content), 12) : String(a.content).split("\n").slice(0, 8).map((l) => `+${l}`)
      return o
    },
  },
  {
    name: "edit_file",
    description: "Replace an exact string in a file. old_string must match exactly once unless replace_all is true. Read the file first.",
    parameters: obj({ path: str(), old_string: str(), new_string: str(), replace_all: { type: "boolean" } }, ["path", "old_string", "new_string"]),
    buildOnly: true,
    run(a, ctx) {
      const p = resolvePath(ctx.cwd, a.path)
      let text: string
      try {
        text = readFileSync(p, "utf8")
      } catch (e: any) {
        return fail(`${rel(ctx.cwd, p)}: ${e.code ?? e.message}`)
      }
      const oldS = String(a.old_string)
      const newS = String(a.new_string)
      if (!oldS) return fail("old_string is empty; use write_file to create files")
      const count = text.split(oldS).length - 1
      if (count === 0) return fail(`old_string not found in ${rel(ctx.cwd, p)}`)
      if (count > 1 && !a.replace_all) return fail(`old_string matches ${count} times; add context or set replace_all`)
      writeFileSync(p, a.replace_all ? text.split(oldS).join(newS) : text.replace(oldS, () => newS))
      const o = ok(`Edited ${rel(ctx.cwd, p)} (${count} replacement${count === 1 ? "" : "s"}).`, `+${newS.split("\n").length} -${oldS.split("\n").length}`)
      o.detail = diffLines(oldS, newS, 16)
      return o
    },
  },
  {
    name: "bash",
    description: "Run a shell command in the workspace with bash. Returns combined stdout/stderr and the exit code. Avoid interactive commands.",
    parameters: obj({ command: str(), timeout_ms: { type: "integer", description: "Default 120000, max 600000" } }, ["command"]),
    async run(a, ctx) {
      const command = String(a.command ?? "")
      if (!command.trim()) return fail("empty command")
      if (ctx.mode === "plan" && !isSafeCommand(command)) return fail("plan mode only allows read-only commands")
      const timeout = Math.min(600_000, Math.max(1000, Number(a.timeout_ms) || 120_000))
      const proc = Bun.spawn(["bash", "-c", command], {
        cwd: ctx.cwd,
        stdin: "ignore",
        stdout: "pipe",
        stderr: "pipe",
        env: { ...process.env, NUR: "1", TERM: "dumb", PAGER: "cat", GIT_PAGER: "cat" },
      })
      const kill = () => proc.kill("SIGKILL")
      ctx.signal.addEventListener("abort", kill, { once: true })
      const timer = setTimeout(kill, timeout)
      const [out, err, code] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text(), proc.exited])
      clearTimeout(timer)
      ctx.signal.removeEventListener("abort", kill)
      if (ctx.signal.aborted) return fail("interrupted")
      let text = stripAnsi(out + (err.trim() ? (out && !out.endsWith("\n") ? "\n" : "") + err : "")).trimEnd()
      const lines = text ? text.split("\n").length : 0
      return { output: `${text ? truncate(text) : "(no output)"}\nexit code: ${code}`, summary: `exit ${code} · ${lines} lines`, detail: preview(text, 6), ok: code === 0 }
    },
  },
  {
    name: "web_fetch",
    description: "Fetch a URL over HTTPS and return its text (HTML is converted to readable text). Max 100 KB.",
    parameters: obj({ url: str("https:// URL") }, ["url"]),
    async run(a, ctx) {
      let url: URL
      try {
        url = new URL(String(a.url))
      } catch {
        return fail("invalid URL")
      }
      if (!/^https?:$/.test(url.protocol)) return fail("only http(s) URLs are supported")
      try {
        const r = await fetch(url, { signal: AbortSignal.any([ctx.signal, AbortSignal.timeout(30_000)]), headers: { "User-Agent": "nur" } })
        let body = await r.text()
        if ((r.headers.get("content-type") ?? "").includes("html")) body = htmlToText(body)
        body = body.slice(0, 100_000)
        return { output: `HTTP ${r.status}\n\n${body}`, summary: `HTTP ${r.status} · ${body.length} chars`, detail: preview(body, 3), ok: r.ok }
      } catch (e: any) {
        return fail(e.message)
      }
    },
  },
  {
    name: "todo_write",
    description: "Replace the visible task list for the current work. Use it for multi-step tasks; keep exactly one item in_progress.",
    parameters: obj({ items: { type: "array", items: obj({ text: str(), status: { type: "string", enum: ["pending", "in_progress", "completed"] } }, ["text", "status"]) } }, ["items"]),
    run(a, ctx) {
      const items: Todo[] = Array.isArray(a.items) ? a.items : []
      ctx.onTodos(items)
      const done = items.filter((i) => i.status === "completed").length
      const mark = { completed: "☒", in_progress: "◐", pending: "☐" } as const
      return { output: "Todo list updated.", summary: `${done}/${items.length} done`, detail: items.map((i) => `${mark[i.status] ?? "☐"} ${i.text}`), ok: true }
    },
  },
  {
    name: "memory",
    description: "Persistent memory across sessions. 'add' saves a durable fact or preference (project scope by default, or global), 'remove' deletes entries containing text, 'list' shows all.",
    parameters: obj({ action: { type: "string", enum: ["add", "remove", "list"] }, text: str(), scope: { type: "string", enum: ["project", "global"] } }, ["action"]),
    run(a, ctx) {
      const global = a.scope === "global"
      if (a.action === "add") {
        ctx.memory.add(String(a.text ?? ""), global)
        return ok("Saved.", `remembered: ${oneLine(String(a.text ?? ""), 80)}`)
      }
      if (a.action === "remove") {
        const n = ctx.memory.remove(String(a.text ?? ""), global)
        return ok(`Removed ${n} entries.`, `forgot ${n} entries`)
      }
      return ok(ctx.memory.render() || "(memory is empty)", "listed memory")
    },
  },
  {
    name: "load_skill",
    description: "Load the full instructions of an available skill by name before using it.",
    parameters: obj({ name: str() }, ["name"]),
    run(a, ctx) {
      const body = ctx.skills.load(String(a.name))
      return body ? ok(body, `loaded skill ${a.name}`) : fail(`no skill named '${a.name}'`)
    },
  },
  {
    name: "goal_complete",
    description: "Call ONLY when the active goal is fully achieved and verified. Provide a concise summary of what was done and how it was verified.",
    parameters: obj({ summary: str() }, ["summary"]),
    run(a, ctx) {
      ctx.onGoalComplete(String(a.summary ?? ""))
      return ok("Goal marked complete.", oneLine(String(a.summary ?? ""), 200))
    },
  },
]

function htmlToText(html: string): string {
  return html
    .replace(/<(script|style|noscript)[\s\S]*?<\/\1>/gi, "")
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<\/(p|div|h\d|li|tr|section|article)>/gi, "\n")
    .replace(/<li[^>]*>/gi, "- ")
    .replace(/<[^>]+>/g, "")
    .replace(/&nbsp;/g, " ")
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/\n{3,}/g, "\n\n")
    .trim()
}
