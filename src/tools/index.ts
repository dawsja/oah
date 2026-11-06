// Built-in tools. Each returns text for the model plus a short summary and
// preview lines for the transcript. Permission gating happens in the agent.
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs"
import { dirname, isAbsolute, join, normalize, relative, resolve } from "node:path"
import { homedir } from "node:os"
import type { Mode } from "../config"
import type { Memory } from "../memory"
import type { Skills } from "../skills"
import { applyPatch } from "./patch"
import { getShell, killShell, readShell, runningShells, shellStatus, startShell } from "./shells"

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
  /** Only offered to OpenAI models (ChatGPT plan / API), which are trained on the format. */
  openaiOnly?: boolean
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
    case "bash_output":
    case "bash_kill":
    case "ask_user":
    case "task":
      return "internal"
    case "read_file":
    case "ls":
    case "find":
    case "grep":
      return !args?.path || inside(cwd, resolvePath(cwd, args.path)) ? "read" : "read-outside"
    case "write_file":
    case "edit_file":
    case "apply_patch":
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
    case "ls":
      return `List ${path()}`
    case "find":
      return `Find ${[args?.name, args?.path_pattern, args?.type && args.type !== "any" ? args.type : "", args?.modified_within ? `modified < ${args.modified_within}` : ""].filter(Boolean).join(" ") || "*"}${args?.path ? ` in ${path()}` : ""}`
    case "grep":
      return `Grep "${args?.pattern ?? ""}"${args?.path ? ` in ${path()}` : ""}`
    case "write_file":
      return `Write ${path()}`
    case "edit_file":
      return `Edit ${path()}`
    case "bash":
      return `${args?.background ? "Background" : "Bash"} ${oneLine(String(args?.command ?? ""), 120)}`
    case "bash_output":
      return `Output ${args?.id ?? ""}${args?.filter ? ` /${args.filter}/` : ""}`
    case "bash_kill":
      return `Stop ${args?.id ?? ""}`
    case "apply_patch": {
      const files = [...String(args?.patch ?? "").matchAll(/^\*\*\* (?:Add|Update|Delete) File: (.+)$/gm)].map((m) => m[1]!.trim())
      return `Patch ${files.length > 2 ? `${files.slice(0, 2).join(", ")} +${files.length - 2} more` : files.join(", ")}`
    }
    case "task":
      return `Task ${oneLine(String(args?.description ?? ""), 80)}`
    case "ask_user":
      return `Ask ${oneLine(String(args?.question ?? ""), 100)}`
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
/** The edits of an edit_file call, single or batched. */
export function editList(a: any): { old_string: string; new_string: string; replace_all?: boolean }[] {
  if (Array.isArray(a?.edits) && a.edits.length) return a.edits.map((e: any) => ({ old_string: String(e?.old_string ?? ""), new_string: String(e?.new_string ?? ""), replace_all: !!e?.replace_all }))
  if (a?.old_string !== undefined) return [{ old_string: String(a.old_string), new_string: String(a.new_string ?? ""), replace_all: !!a.replace_all }]
  return []
}

export function permissionPreview(name: string, args: any): string[] {
  if (name === "edit_file") {
    const out = editList(args).flatMap((e) => diffLines(e.old_string, e.new_string, 8))
    return out.length > 14 ? [...out.slice(0, 14), `… +${out.length - 14} lines`] : out
  }
  if (name === "apply_patch") {
    const out = String(args?.patch ?? "")
      .split("\n")
      .filter((l) => /^(\*\*\* (Add|Update|Delete) File|\*\*\* Move to|[+-])/.test(l))
      .map((l) => l.replace(/^\*\*\* /, "▸ "))
    return out.length > 14 ? [...out.slice(0, 14), `… +${out.length - 14} lines`] : out
  }
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

const SKIP_DIRS = new Set([".git", "node_modules", "target"])

export function humanSize(n: number): string {
  if (n < 1024) return `${n}B`
  const units = ["K", "M", "G", "T"]
  let v = n / 1024
  let i = 0
  while (v >= 1024 && i < units.length - 1) (v /= 1024), i++
  return `${v < 10 ? v.toFixed(1) : Math.round(v)}${units[i]}`
}

export function parseSize(s: string): number | null {
  const m = /^(\d+(?:\.\d+)?)\s*([kmgt]?)b?$/i.exec(s.trim())
  if (!m) return null
  const mult = { "": 1, k: 1024, m: 1024 ** 2, g: 1024 ** 3, t: 1024 ** 4 }[m[2]!.toLowerCase() as "" | "k" | "m" | "g" | "t"]
  return Math.round(Number(m[1]) * mult)
}

export function parseAge(s: string): number | null {
  const m = /^(\d+(?:\.\d+)?)\s*(s|m|h|d|w)$/i.exec(s.trim())
  if (!m) return null
  const mult = { s: 1e3, m: 6e4, h: 36e5, d: 864e5, w: 6048e5 }[m[2]!.toLowerCase() as "s" | "m" | "h" | "d" | "w"]
  return Number(m[1]) * mult
}

/** Walk files and directories with depth / hidden / .gitignore control. Return false from `on` to stop. */
function walkEntries(root: string, signal: AbortSignal, o: { hidden: boolean; gitignore: boolean; maxDepth: number }, on: (path: string, isDir: boolean) => boolean | void) {
  const ignored = o.gitignore ? loadGitignore(root) : []
  const stack: [string, number][] = [[root, 0]]
  while (stack.length) {
    if (signal.aborted) return
    const [dir, depth] = stack.pop()!
    let entries: import("node:fs").Dirent[]
    try {
      entries = readdirSync(dir, { withFileTypes: true })
    } catch {
      continue
    }
    for (const e of entries) {
      if (SKIP_DIRS.has(e.name)) continue
      if (!o.hidden && e.name.startsWith(".")) continue
      const p = join(dir, e.name)
      const r = relative(root, p)
      if (ignored.some((g) => g.test(r) || g.test(e.name))) continue
      const isDir = e.isDirectory()
      if (on(p, isDir) === false) return
      if (isDir && depth + 1 < o.maxDepth) stack.push([p, depth + 1])
    }
  }
}

let rgPath: string | null | undefined
function findRg(): string | null {
  if (process.env.NUR_NO_RG) return null
  if (rgPath === undefined) rgPath = Bun.which("rg") ?? null
  return rgPath
}

type GrepOpts = { glob?: string; case_insensitive?: boolean; fixed_strings?: boolean; context: number; mode: "content" | "files" | "count" }

/** ripgrep output lines, or null when rg isn't available. A string is an error. */
async function ripgrep(pattern: string, root: string, o: GrepOpts, ctx: ToolCtx): Promise<string[] | string | null> {
  const rg = findRg()
  if (!rg) return null
  const args = [rg, "--no-heading", "--with-filename", "--no-require-git", "--color", "never", "--max-columns", "300", "--max-columns-preview", "--max-filesize", "4M"]
  if (o.mode === "files") args.push("--files-with-matches")
  else if (o.mode === "count") args.push("--count")
  else args.push("--line-number")
  if (o.mode === "content" && o.context) args.push("-C", String(o.context))
  if (o.case_insensitive) args.push("-i")
  if (o.fixed_strings) args.push("-F")
  if (o.glob) args.push("--glob", o.glob)
  args.push("-e", pattern, "--", inside(ctx.cwd, root) ? relative(ctx.cwd, root) || "." : root)
  const proc = Bun.spawn(args, { cwd: ctx.cwd, stdout: "pipe", stderr: "pipe", stdin: "ignore" })
  const kill = () => proc.kill()
  ctx.signal.addEventListener("abort", kill, { once: true })
  const [out, err, code] = await Promise.all([new Response(proc.stdout).text(), new Response(proc.stderr).text(), proc.exited])
  ctx.signal.removeEventListener("abort", kill)
  if (code === 1) return []
  if (code !== 0) return err.trim().split("\n")[0] || `ripgrep exited with ${code}`
  const lines = out.split("\n").filter(Boolean).map((l) => l.replace(/^\.\//, ""))
  // Match the JS fallback's "path:line: text" shape.
  return o.mode === "content" ? lines.map((l) => (l === "--" ? "--" : l.replace(/^(.+?)([:-])(\d+)\2/, (_m, f, sep, n) => `${f}${sep}${n}${sep} `))) : lines
}

const GREP_CAP = 5000

function grepJs(pattern: string, root: string, o: GrepOpts & { limit: number }, ctx: ToolCtx): string[] | string {
  let re: RegExp
  try {
    re = new RegExp(o.fixed_strings ? pattern.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") : pattern, o.case_insensitive ? "i" : "")
  } catch (e: any) {
    return `bad regex: ${e.message}`
  }
  const filter = o.glob ? globToRegex(o.glob) : null
  const out: string[] = []
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
    const lines = buf.toString("utf8").split("\n")
    const name = rel(ctx.cwd, p)
    let count = 0
    let lastShown = -1
    for (let i = 0; i < lines.length; i++) {
      if (!re.test(lines[i]!)) continue
      count++
      if (o.mode !== "content") continue
      const from = Math.max(0, i - o.context, lastShown + 1)
      if (o.context && lastShown >= 0 && from > lastShown + 1) out.push("--")
      for (let j = from; j <= Math.min(lines.length - 1, i + o.context); j++) {
        out.push(`${name}${j === i ? ":" : "-"}${j + 1}${j === i ? ":" : "-"} ${oneLine(lines[j]!, 300)}`)
        lastShown = j
      }
      if (out.length >= GREP_CAP) return false
    }
    if (count && o.mode === "files") out.push(name)
    if (count && o.mode === "count") out.push(`${name}:${count}`)
    return out.length < GREP_CAP
  }
  if (statSync(root).isFile()) search(root)
  else walkEntries(root, ctx.signal, { hidden: false, gitignore: true, maxDepth: Infinity }, (p, isDir) => (isDir ? undefined : search(p)))
  return out
}

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
    name: "ls",
    description:
      "List a directory like `ls`. Directories end with '/'. Set all=true to include dotfiles, long=true for sizes and modification times, depth (1-3) to include subdirectories.",
    parameters: obj({
      path: str("Directory (default: workspace root)"),
      all: { type: "boolean", description: "Include hidden entries (dotfiles)" },
      long: { type: "boolean", description: "Show size and modification time" },
      depth: { type: "integer", description: "Levels to list (1-3, default 1)" },
    }),
    run(a, ctx) {
      const root = a.path ? resolvePath(ctx.cwd, a.path) : ctx.cwd
      const depth = Math.min(3, Math.max(1, Number(a.depth) || 1))
      const out: string[] = []
      let count = 0
      const visit = (dir: string, level: number) => {
        let entries: import("node:fs").Dirent[]
        try {
          entries = readdirSync(dir, { withFileTypes: true })
        } catch (e: any) {
          if (level === 0) throw e
          return
        }
        entries.sort((x, y) => Number(y.isDirectory()) - Number(x.isDirectory()) || x.name.localeCompare(y.name))
        for (const e of entries) {
          if (!a.all && e.name.startsWith(".")) continue
          if (count >= 1000) return
          count++
          const p = join(dir, e.name)
          const name = "  ".repeat(level) + e.name + (e.isDirectory() ? "/" : e.isSymbolicLink() ? "@" : "")
          if (a.long) {
            let size = "-"
            let mtime = ""
            try {
              const st = statSync(p)
              size = e.isDirectory() ? "-" : humanSize(st.size)
              mtime = st.mtime.toISOString().slice(0, 16).replace("T", " ")
            } catch {}
            out.push(`${size.padStart(7)}  ${mtime}  ${name}`)
          } else out.push(name)
          if (e.isDirectory() && level + 1 < depth && !SKIP_DIRS.has(e.name)) visit(p, level + 1)
        }
      }
      try {
        visit(root, 0)
      } catch (e: any) {
        return fail(`${rel(ctx.cwd, root)}: ${e.code ?? e.message}`)
      }
      if (count >= 1000) out.push("… (truncated at 1000 entries)")
      return ok(out.join("\n") || "(empty)", `${count} entries`)
    },
  },
  {
    name: "find",
    description:
      "Find files and directories like `find`: filter by name glob (e.g. '*.test.ts' or '{Dockerfile,*.yml}'), type, depth, size and modification age. Respects .gitignore unless no_ignore=true. Returns up to `limit` paths (default 200).",
    parameters: obj({
      path: str("Directory to search (default: workspace root)"),
      name: str("Glob matched against the file name, e.g. '*.rs'"),
      path_pattern: str("Glob matched against the path relative to `path`, e.g. 'src/**/test_*'"),
      type: { type: "string", enum: ["file", "dir", "any"], description: "Default: any" },
      max_depth: { type: "integer", description: "Maximum depth below `path`" },
      min_size: str("e.g. '10k', '2M'"),
      max_size: str("e.g. '500k'"),
      modified_within: str("Only entries modified within this long, e.g. '30m', '2h', '7d'"),
      hidden: { type: "boolean", description: "Include dotfiles and dot-directories" },
      no_ignore: { type: "boolean", description: "Don't apply .gitignore" },
      limit: { type: "integer", description: "Maximum results (default 200, max 1000)" },
    }),
    run(a, ctx) {
      const root = a.path ? resolvePath(ctx.cwd, a.path) : ctx.cwd
      if (!existsSync(root)) return fail(`${rel(ctx.cwd, root)}: no such directory`)
      const nameRe = a.name ? globToRegex(String(a.name)) : null
      const pathRe = a.path_pattern ? globToRegex(String(a.path_pattern)) : null
      const type = a.type ?? "any"
      const maxDepth = a.max_depth === undefined ? Infinity : Math.max(1, Number(a.max_depth))
      const minSize = a.min_size ? parseSize(String(a.min_size)) : null
      const maxSize = a.max_size ? parseSize(String(a.max_size)) : null
      const within = a.modified_within ? parseAge(String(a.modified_within)) : null
      if ((a.min_size && minSize === null) || (a.max_size && maxSize === null)) return fail("sizes look like '10k', '2M' or '1G'")
      if (a.modified_within && within === null) return fail("modified_within looks like '30m', '2h' or '7d'")
      const limit = Math.min(1000, Math.max(1, Number(a.limit) || 200))
      const needStat = minSize !== null || maxSize !== null || within !== null
      const out: string[] = []
      walkEntries(root, ctx.signal, { hidden: !!a.hidden, gitignore: !a.no_ignore, maxDepth }, (p, isDir) => {
        if ((type === "file" || minSize !== null || maxSize !== null) && isDir) return
        if (type === "dir" && !isDir) return
        const relp = relative(root, p)
        if (nameRe && !nameRe.test(p.split("/").pop()!)) return
        if (pathRe && !pathRe.test(relp)) return
        if (needStat) {
          try {
            const st = statSync(p)
            if (!isDir && minSize !== null && st.size < minSize) return
            if (!isDir && maxSize !== null && st.size > maxSize) return
            if (within !== null && Date.now() - st.mtimeMs > within) return
          } catch {
            return
          }
        }
        out.push(rel(ctx.cwd, p) + (isDir ? "/" : ""))
        return out.length < limit
      })
      out.sort()
      const more = out.length >= limit ? `\n… (stopped at ${limit} results; narrow the search or raise limit)` : ""
      return out.length ? ok(out.join("\n") + more, `${out.length} result${out.length === 1 ? "" : "s"}`) : ok("Nothing found.", "no matches")
    },
  },
  {
    name: "grep",
    description:
      "Search file contents like `grep -rn` (uses ripgrep when installed), respecting .gitignore. Regex syntax (or fixed_strings=true for literal text). output: 'content' (path:line: text, default), 'files' (matching paths) or 'count' (matches per file). context adds lines around each match.",
    parameters: obj(
      {
        pattern: str("Regular expression (or literal text with fixed_strings)"),
        path: str("File or directory (default: workspace root)"),
        glob: str("Only search files matching this glob, e.g. '*.ts' or '*.{ts,tsx}'"),
        case_insensitive: { type: "boolean" },
        fixed_strings: { type: "boolean", description: "Treat pattern as literal text" },
        context: { type: "integer", description: "Lines of context before and after each match (0-5)" },
        output: { type: "string", enum: ["content", "files", "count"] },
        limit: { type: "integer", description: "Maximum output lines (default 300, max 2000)" },
      },
      ["pattern"],
    ),
    async run(a, ctx) {
      const root = a.path ? resolvePath(ctx.cwd, a.path) : ctx.cwd
      if (!existsSync(root)) return fail(`${rel(ctx.cwd, root)}: no such file or directory`)
      const limit = Math.min(2000, Math.max(1, Number(a.limit) || 300))
      const context = Math.min(5, Math.max(0, Number(a.context) || 0))
      const mode: "content" | "files" | "count" = a.output === "files" || a.output === "count" ? a.output : "content"
      const rgOut = await ripgrep(String(a.pattern), root, { ...a, context, mode }, ctx)
      const lines = rgOut ?? grepJs(String(a.pattern), root, { ...a, context, mode, limit }, ctx)
      if (typeof lines === "string") return fail(lines)
      if (!lines.length) return ok("No matches.", "no matches")
      const shown = lines.slice(0, limit)
      const more = lines.length > limit ? `\n… (${lines.length - limit} more lines; narrow the search or raise limit)` : ""
      const files = new Set(shown.map((l) => l.split(/[:-]\d+[:-]/)[0])).size
      const summary = mode === "files" ? `${shown.length} files` : mode === "count" ? `${shown.length} files with matches` : `${shown.filter((l) => /^.+?:\d+: /.test(l)).length} matches in ${files} files`
      return ok(shown.join("\n") + more, summary)
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
    description:
      "Replace exact strings in a file. Either one edit (old_string/new_string) or several in `edits`, applied in order, all-or-nothing. old_string must match exactly once unless replace_all is true. Read the file first.",
    parameters: obj(
      {
        path: str(),
        old_string: str(),
        new_string: str(),
        replace_all: { type: "boolean" },
        edits: {
          type: "array",
          description: "Several edits to the same file, applied in order",
          items: obj({ old_string: str(), new_string: str(), replace_all: { type: "boolean" } }, ["old_string", "new_string"]),
        },
      },
      ["path"],
    ),
    buildOnly: true,
    run(a, ctx) {
      const p = resolvePath(ctx.cwd, a.path)
      let text: string
      try {
        text = readFileSync(p, "utf8")
      } catch (e: any) {
        return fail(`${rel(ctx.cwd, p)}: ${e.code ?? e.message}`)
      }
      const edits = editList(a)
      if (!edits.length) return fail("provide old_string/new_string or a non-empty edits list")
      let updated = text
      let replacements = 0
      const detail: string[] = []
      let added = 0
      let removed = 0
      for (const [i, e] of edits.entries()) {
        const which = edits.length > 1 ? `edit ${i + 1}: ` : ""
        if (!e.old_string) return fail(`${which}old_string is empty; use write_file to create files`)
        const count = updated.split(e.old_string).length - 1
        if (count === 0) return fail(`${which}old_string not found in ${rel(ctx.cwd, p)}${i ? " (after the earlier edits)" : ""} — nothing was changed`)
        if (count > 1 && !e.replace_all) return fail(`${which}old_string matches ${count} times; add context or set replace_all — nothing was changed`)
        updated = e.replace_all ? updated.split(e.old_string).join(e.new_string) : updated.replace(e.old_string, () => e.new_string)
        replacements += e.replace_all ? count : 1
        detail.push(...diffLines(e.old_string, e.new_string, 8))
        added += e.new_string.split("\n").length
        removed += e.old_string.split("\n").length
      }
      writeFileSync(p, updated)
      const o = ok(`Edited ${rel(ctx.cwd, p)} (${replacements} replacement${replacements === 1 ? "" : "s"}).`, `+${added} -${removed}${edits.length > 1 ? ` · ${edits.length} edits` : ""}`)
      o.detail = detail.length > 20 ? [...detail.slice(0, 20), `… +${detail.length - 20} lines`] : detail
      return o
    },
  },
  {
    name: "apply_patch",
    description:
      "Apply a patch that can add, update, move and delete several files at once, all-or-nothing. Format:\n*** Begin Patch\n*** Update File: path\n@@ optional anchor line\n context\n-old line\n+new line\n*** Add File: path\n+content\n*** Delete File: path\n*** End Patch",
    parameters: obj({ patch: str("The full patch text, from *** Begin Patch to *** End Patch") }, ["patch"]),
    buildOnly: true,
    openaiOnly: true,
    run(a, ctx) {
      try {
        const r = applyPatch(String(a.patch ?? ""), (q) => resolvePath(ctx.cwd, q), (abs) => rel(ctx.cwd, abs))
        const lines = r.files.map((f) => `${f.status} ${f.path} (+${f.added} -${f.removed})`)
        const o = ok(`Applied patch:\n${lines.join("\n")}`, `${r.files.length} file${r.files.length === 1 ? "" : "s"} · +${r.files.reduce((n, f) => n + f.added, 0)} -${r.files.reduce((n, f) => n + f.removed, 0)}`)
        o.detail = [...lines.map((l) => `  ${l}`), ...r.diff.slice(0, 16), ...(r.diff.length > 16 ? [`… +${r.diff.length - 16} lines`] : [])]
        return o
      } catch (e: any) {
        return fail(`patch not applied (no files changed): ${e.message}`)
      }
    },
  },
  {
    name: "bash",
    description:
      "Run a shell command in the workspace with bash. Returns combined stdout/stderr and the exit code. Avoid interactive commands. For servers, watchers or anything long-running set background=true: it returns a shell id right away; read its output with bash_output and stop it with bash_kill.",
    parameters: obj(
      {
        command: str(),
        timeout_ms: { type: "integer", description: "Default 120000, max 600000 (ignored with background)" },
        background: { type: "boolean", description: "Run in the background and return immediately" },
      },
      ["command"],
    ),
    async run(a, ctx) {
      const command = String(a.command ?? "")
      if (!command.trim()) return fail("empty command")
      if (ctx.mode === "plan" && !isSafeCommand(command)) return fail("plan mode only allows read-only commands")
      if (a.background) {
        const sh = startShell(command, ctx.cwd)
        await Bun.sleep(300)
        const early = readShell(sh)
        const status = shellStatus(sh)
        return {
          output: `Started background shell ${sh.id} (${status}). Read output with bash_output {"id":"${sh.id}"}; stop it with bash_kill.${early ? `\n\nFirst output:\n${truncate(early)}` : ""}`,
          summary: `background ${sh.id} · ${status}`,
          detail: preview(early, 4),
          ok: sh.exitCode === null || sh.exitCode === 0,
        }
      }
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
    name: "bash_output",
    description: "Read new output from a background shell (since the last read) and whether it is still running. Use filter (regex) to keep only matching lines.",
    parameters: obj({ id: str("Shell id from bash background=true"), filter: str("Optional regex; only matching lines are returned") }, ["id"]),
    run(a) {
      const sh = getShell(String(a.id))
      if (!sh) return fail(`no background shell ${a.id} (running: ${runningShells().map((s) => s.id).join(", ") || "none"})`)
      let re: RegExp | undefined
      try {
        re = a.filter ? new RegExp(String(a.filter)) : undefined
      } catch (e: any) {
        return fail(`bad filter: ${e.message}`)
      }
      const text = readShell(sh, re).trimEnd()
      const status = shellStatus(sh)
      return { output: `${sh.id} · ${status}\n${text ? truncate(text) : "(no new output)"}`, summary: `${sh.id} · ${status}`, detail: preview(text, 4), ok: true }
    },
  },
  {
    name: "bash_kill",
    description: "Stop a background shell.",
    parameters: obj({ id: str() }, ["id"]),
    run(a) {
      const sh = getShell(String(a.id))
      if (!sh) return fail(`no background shell ${a.id}`)
      if (sh.exitCode !== null) return ok(`${sh.id} already ${shellStatus(sh)}.`, `${sh.id} already exited`)
      killShell(sh)
      return ok(`Stopped ${sh.id}.`, `stopped ${sh.id}`)
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
