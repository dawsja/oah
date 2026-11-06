// apply_patch: the patch format OpenAI's coding models are trained on.
//
//   *** Begin Patch
//   *** Add File: path/new.ts
//   +line
//   *** Update File: path/old.ts
//   *** Move to: path/renamed.ts        (optional)
//   @@ optional anchor line, e.g. a function signature
//    context
//   -removed
//   +added
//   *** Delete File: path/gone.ts
//   *** End Patch
//
// Every hunk is located and every file computed in memory first; nothing is
// written unless the whole patch applies.
import { existsSync, lstatSync, mkdirSync, readFileSync, realpathSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs"
import { basename, dirname, join } from "node:path"
import { canonicalPath } from "./versions"

export type FileOp =
  | { kind: "add"; path: string; lines: string[] }
  | { kind: "delete"; path: string }
  | { kind: "update"; path: string; moveTo?: string; hunks: Hunk[] }

export interface Hunk {
  anchor?: string
  lines: string[] // each starts with ' ', '-' or '+'
}

export function parsePatch(text: string): FileOp[] {
  const lines = text.replace(/\r\n/g, "\n").split("\n")
  let i = lines.findIndex((l) => l.trim() === "*** Begin Patch")
  if (i < 0) throw new Error("patch must start with '*** Begin Patch'")
  i++
  const ops: FileOp[] = []
  while (i < lines.length) {
    const line = lines[i]!
    if (line.trim() === "*** End Patch") return ops
    let m: RegExpExecArray | null
    if ((m = /^\*\*\* Add File: (.+)$/.exec(line))) {
      const out: string[] = []
      i++
      while (i < lines.length && !lines[i]!.startsWith("*** ")) {
        const l = lines[i]!
        if (!l.startsWith("+")) throw new Error(`line ${i + 1}: lines of an added file must start with '+'`)
        out.push(l.slice(1))
        i++
      }
      ops.push({ kind: "add", path: m[1]!.trim(), lines: out })
    } else if ((m = /^\*\*\* Delete File: (.+)$/.exec(line))) {
      ops.push({ kind: "delete", path: m[1]!.trim() })
      i++
    } else if ((m = /^\*\*\* Update File: (.+)$/.exec(line))) {
      const op: FileOp = { kind: "update", path: m[1]!.trim(), hunks: [] }
      i++
      if (i < lines.length && (m = /^\*\*\* Move to: (.+)$/.exec(lines[i]!))) {
        op.moveTo = m[1]!.trim()
        i++
      }
      let hunk: Hunk | null = null
      while (i < lines.length && !/^\*\*\* (Add File|Delete File|Update File|End Patch)/.test(lines[i]!)) {
        const l = lines[i]!
        if (l.startsWith("@@")) {
          hunk = { anchor: l.slice(2).trim() || undefined, lines: [] }
          op.hunks.push(hunk)
        } else if (l === "*** End of File") {
          // marker only
        } else {
          if (!hunk) {
            hunk = { lines: [] }
            op.hunks.push(hunk)
          }
          hunk.lines.push(/^[ +-]/.test(l) ? l : ` ${l}`)
        }
        i++
      }
      if (!op.hunks.length) throw new Error(`no changes for ${op.path}`)
      ops.push(op)
    } else if (line.trim() === "") {
      i++
    } else {
      throw new Error(`line ${i + 1}: unexpected '${line.slice(0, 60)}'`)
    }
  }
  throw new Error("patch must end with '*** End Patch'")
}

/** Find `needle` (array of lines) in `hay` at or after `from`, tolerating whitespace differences. */
function seek(hay: string[], needle: string[], from: number): number {
  if (!needle.length) return from
  const norms = [(s: string) => s, (s: string) => s.trimEnd()]
  for (const norm of norms) {
    const matches: number[] = []
    for (let i = from; i + needle.length <= hay.length; i++) {
      let ok = true
      for (let j = 0; j < needle.length; j++) {
        if (norm(hay[i + j]!) !== norm(needle[j]!)) {
          ok = false
          break
        }
      }
      if (ok) matches.push(i)
    }
    if (matches.length > 1) throw new Error("ambiguous patch hunk; provide more context or an anchor")
    if (matches.length === 1) return matches[0]!
  }
  return -1
}

export function applyHunks(content: string, hunks: Hunk[], path: string): string {
  const eol = content.endsWith("\n")
  const newline = content.includes("\r\n") ? "\r\n" : "\n"
  const lines = content.replace(/\r\n/g, "\n").split("\n")
  if (eol) lines.pop()
  let cursor = 0
  for (const h of hunks) {
    if (h.anchor) {
      const a = seek(lines, [h.anchor], cursor)
      if (a < 0) throw new Error(`${path}: anchor not found: ${h.anchor}`)
      cursor = a
    }
    const oldLines = h.lines.filter((l) => !l.startsWith("+")).map((l) => l.slice(1))
    const newLines = h.lines.filter((l) => !l.startsWith("-")).map((l) => l.slice(1))
    let at = seek(lines, oldLines, cursor)
    if (at < 0 && !h.anchor) at = seek(lines, oldLines, 0)
    if (at < 0) throw new Error(`${path}: couldn't find the lines to change:\n${oldLines.slice(0, 6).join("\n")}`)
    lines.splice(at, oldLines.length, ...newLines)
    cursor = at + newLines.length
  }
  return lines.join(newline) + (eol || !lines.length ? newline : "")
}

export interface PatchResult {
  files: { path: string; status: "A" | "M" | "D" | "R"; added: number; removed: number }[]
  diff: string[]
}

/** Atomic per-file replacement. Multi-file rollback is best effort, not crash atomicity. */
export function replaceFile(path: string, content: string | Buffer, mode?: number) {
  if (existsSync(path) && lstatSync(path).isSymbolicLink()) path = realpathSync(path)
  const temp = `${path}.oah-${crypto.randomUUID()}.tmp`
  mkdirSync(dirname(path), { recursive: true })
  try {
    writeFileSync(temp, content, { mode: mode ?? (existsSync(path) ? statSync(path).mode : 0o666) })
    renameSync(temp, path)
  } finally { rmSync(temp, { force: true }) }
}

/** Preflight all operations against a staged filesystem, then commit with rollback. */
export function applyPatch(text: string, resolve: (p: string) => string, display: (abs: string) => string, commit: (p: string, value: string | null) => void = (p, v) => v === null ? rmSync(p, { force: true }) : replaceFile(p, v)): PatchResult {
  const resolveFile = (p: string) => { const abs = resolve(p); return join(canonicalPath(dirname(abs)), basename(abs)) }
  const originals = new Map<string, { text: string | null; bytes: Buffer | null; mode?: number }>()
  const staged = new Map<string, string | null>()
  const load = (p: string): string | null => {
    if (!originals.has(p)) {
      const entry = lstatSync(p, { throwIfNoEntry: false })
      if (entry && !entry.isFile()) throw new Error(`${p}: patch target must be a regular file, not a symlink or directory`)
      const bytes = entry ? readFileSync(p) : null
      originals.set(p, { text: bytes?.toString("utf8") ?? null, bytes, mode: entry?.mode })
    }
    return staged.has(p) ? staged.get(p)! : originals.get(p)!.text
  }
  const result: PatchResult = { files: [], diff: [] }
  try {
    for (const op of parsePatch(text)) {
      const abs = resolveFile(op.path)
      const body = load(abs)
      if (op.kind === "add") {
        if (body !== null) throw new Error(`${op.path} already exists (use Update File)`)
        staged.set(abs, op.lines.join("\n") + "\n")
        result.files.push({ path: display(abs), status: "A", added: op.lines.length, removed: 0 })
        result.diff.push(...op.lines.slice(0, 6).map((l) => `+${l}`))
      } else if (op.kind === "delete") {
        if (body === null) throw new Error(`${op.path} doesn't exist`)
        staged.set(abs, null)
        result.files.push({ path: display(abs), status: "D", added: 0, removed: body ? body.replace(/\n$/, "").split("\n").length : 0 })
      } else {
        if (body === null) throw new Error(`${op.path} doesn't exist`)
        const bytes = originals.get(abs)!.bytes
        if (!staged.has(abs) && bytes && (bytes.includes(0) || !Buffer.from(body, "utf8").equals(bytes))) throw new Error(`${op.path}: binary files cannot be updated with a text patch`)
        const updated = applyHunks(body, op.hunks, op.path)
        const added = op.hunks.reduce((n, h) => n + h.lines.filter((l) => l.startsWith("+")).length, 0)
        const removed = op.hunks.reduce((n, h) => n + h.lines.filter((l) => l.startsWith("-")).length, 0)
        if (op.moveTo) {
          const dest = resolveFile(op.moveTo)
          if (dest === abs) throw new Error(`${op.path}: move destination equals source`)
          if (load(dest) !== null) throw new Error(`${op.moveTo} already exists; refusing to overwrite`)
          staged.set(dest, updated)
          staged.set(abs, null)
          result.files.push({ path: `${display(abs)} → ${display(dest)}`, status: "R", added, removed })
        } else {
          staged.set(abs, updated)
          result.files.push({ path: display(abs), status: "M", added, removed })
        }
        for (const h of op.hunks) result.diff.push(...h.lines.filter((l) => !l.startsWith(" ")))
      }
    }
  } catch (e: any) { throw new Error(`no files changed: ${e.message}`) }
  const touched: string[] = []
  try {
    for (const [path, value] of staged) {
      touched.push(path)
      commit(path, value)
    }
  } catch (e: any) {
    const failures: string[] = []
    for (const path of touched.reverse()) {
      try {
        const old = originals.get(path)!
        if (old.text === null) rmSync(path, { force: true })
        else replaceFile(path, old.bytes!, old.mode)
      } catch { failures.push(path) }
    }
    throw new Error(failures.length ? `partial changes possible; rollback failed for ${failures.join(", ")}: ${e.message}` : `no files changed (rolled back): ${e.message}`)
  }
  return result
}
