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
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { dirname } from "node:path"

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
  const norms = [(s: string) => s, (s: string) => s.trimEnd(), (s: string) => s.trim()]
  for (const norm of norms) {
    for (let i = from; i + needle.length <= hay.length; i++) {
      let ok = true
      for (let j = 0; j < needle.length; j++) {
        if (norm(hay[i + j]!) !== norm(needle[j]!)) {
          ok = false
          break
        }
      }
      if (ok) return i
    }
  }
  return -1
}

export function applyHunks(content: string, hunks: Hunk[], path: string): string {
  const eol = content.endsWith("\n")
  const lines = content.split("\n")
  if (eol) lines.pop()
  let cursor = 0
  for (const h of hunks) {
    if (h.anchor) {
      const a = seek(lines, [h.anchor], cursor)
      if (a >= 0) cursor = a
    }
    const oldLines = h.lines.filter((l) => !l.startsWith("+")).map((l) => l.slice(1))
    const newLines = h.lines.filter((l) => !l.startsWith("-")).map((l) => l.slice(1))
    let at = seek(lines, oldLines, cursor)
    if (at < 0) at = seek(lines, oldLines, 0)
    if (at < 0) throw new Error(`${path}: couldn't find the lines to change:\n${oldLines.slice(0, 6).join("\n")}`)
    lines.splice(at, oldLines.length, ...newLines)
    cursor = at + newLines.length
  }
  return lines.join("\n") + (eol || !lines.length ? "\n" : "")
}

export interface PatchResult {
  files: { path: string; status: "A" | "M" | "D" | "R"; added: number; removed: number }[]
  diff: string[]
}

/** Apply all-or-nothing. `resolve` maps patch paths to absolute paths. */
export function applyPatch(text: string, resolve: (p: string) => string, display: (abs: string) => string): PatchResult {
  const ops = parsePatch(text)
  const writes: { abs: string; content: string | null }[] = []
  const result: PatchResult = { files: [], diff: [] }
  for (const op of ops) {
    const abs = resolve(op.path)
    if (op.kind === "add") {
      if (existsSync(abs)) throw new Error(`${op.path} already exists (use Update File)`)
      writes.push({ abs, content: op.lines.join("\n") + "\n" })
      result.files.push({ path: display(abs), status: "A", added: op.lines.length, removed: 0 })
      result.diff.push(...op.lines.slice(0, 6).map((l) => `+${l}`))
    } else if (op.kind === "delete") {
      if (!existsSync(abs)) throw new Error(`${op.path} doesn't exist`)
      const body = readFileSync(abs, "utf8")
      const n = body ? body.replace(/\n$/, "").split("\n").length : 0
      writes.push({ abs, content: null })
      result.files.push({ path: display(abs), status: "D", added: 0, removed: n })
    } else {
      if (!existsSync(abs)) throw new Error(`${op.path} doesn't exist`)
      const updated = applyHunks(readFileSync(abs, "utf8"), op.hunks, op.path)
      const added = op.hunks.reduce((n, h) => n + h.lines.filter((l) => l.startsWith("+")).length, 0)
      const removed = op.hunks.reduce((n, h) => n + h.lines.filter((l) => l.startsWith("-")).length, 0)
      if (op.moveTo) {
        const dest = resolve(op.moveTo)
        writes.push({ abs: dest, content: updated }, { abs, content: null })
        result.files.push({ path: `${display(abs)} → ${display(dest)}`, status: "R", added, removed })
      } else {
        writes.push({ abs, content: updated })
        result.files.push({ path: display(abs), status: "M", added, removed })
      }
      for (const h of op.hunks) result.diff.push(...h.lines.filter((l) => !l.startsWith(" ")))
    }
  }
  for (const w of writes) {
    if (w.content === null) rmSync(w.abs, { force: true })
    else {
      mkdirSync(dirname(w.abs), { recursive: true })
      writeFileSync(w.abs, w.content)
    }
  }
  return result
}
