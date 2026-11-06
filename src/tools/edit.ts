import { editCandidates } from "./text"

/** Exact first, then unique line-aligned CRLF/trailing-whitespace recovery. Never best-guess. */
export function replaceMatch(text: string, old: string, replacement: string, all = false): { text: string; count: number; method: string } {
  const exact = text.split(old).length - 1
  if (exact) {
    if (exact > 1 && !all) throw Error(`old_string matches ${exact} times; add context or set replace_all — nothing was changed`)
    return { text: all ? text.split(old).join(replacement) : text.replace(old, () => replacement), count: all ? exact : 1, method: "exact" }
  }
  // A normalized fallback is line-aligned and unique, even if replace_all was requested.
  const norm = (s: string) => s.replace(/\r$/, "").replace(/[\t ]+$/, "")
  const needle = old.replace(/\r\n/g, "\n").split("\n")
  const endsInNewline = needle.at(-1) === ""
  if (endsInNewline) needle.pop()
  const lines = text.split("\n")
  const offsets: number[] = []
  let offset = 0
  for (const line of lines) { offsets.push(offset); offset += line.length + 1 }
  const matches: number[] = []
  for (let i = 0; i + needle.length <= lines.length; i++) {
    if (needle.every((line, j) => norm(line) === norm(lines[i + j]!)) && (!endsInNewline || i + needle.length < lines.length)) matches.push(i)
  }
  if (matches.length > 1) throw Error(`normalized match is ambiguous (${matches.length} regions); provide more context — nothing was changed`)
  if (!matches.length) throw Error(`old_string not found — nothing was changed${editCandidates(text, old)}`)
  const i = matches[0]!
  const start = offsets[i]!
  const last = i + needle.length - 1
  const end = endsInNewline ? offsets[last]! + lines[last]!.length + 1 : offsets[last]! + lines[last]!.replace(/\r$/, "").length
  const eol = text.includes("\r\n") ? "\r\n" : "\n"
  const next = replacement.replace(/\r\n/g, "\n").replace(/\n/g, eol)
  return { text: text.slice(0, start) + next + text.slice(end), count: 1, method: "normalized whitespace/CRLF" }
}
