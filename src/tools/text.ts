export const OUTPUT_LIMIT = 30_000

/** Bounded streaming collection with an honest total, independent of retained text. */
export class BoundedText {
  private head = ""
  private tail = ""
  private total = 0
  constructor(private limit = 60_000) {}
  append(text: string) {
    this.total += text.length
    const half = Math.floor(this.limit / 2)
    if (this.head.length < half) this.head += text.slice(0, half - this.head.length)
    this.tail = (this.tail + text).slice(-half)
  }
  toString(): string {
    if (this.total <= this.head.length) return this.head
    if (this.total <= this.limit) return this.head + this.tail.slice(-(this.total - this.head.length))
    const marker = `\n… [stream output elided; ${this.total} characters total] …\n`
    const half = Math.max(0, Math.floor((this.limit - marker.length) / 2))
    return this.head.slice(0, half) + marker + this.tail.slice(-half)
  }
}

/** Bounded text with an explicit omission marker, never silently clipped. */
export function capText(text: string, max = OUTPUT_LIMIT): string {
  if (text.length <= max) return text
  const marker = `\n… [output elided; ${text.length} characters total; request a narrower result] …\n`
  const room = Math.max(0, max - marker.length)
  return text.slice(0, Math.ceil(room / 2)) + marker + text.slice(-Math.floor(room / 2))
}

/** Contiguous numbered source window. The next offset always advances, including giant lines. */
export function readWindow(lines: string[], offset: number, limit: number, max = OUTPUT_LIMIT): { text: string; count: number } {
  const out: string[] = []
  let chars = 0
  for (let i = offset - 1; i < lines.length && out.length < limit; i++) {
    const line = lines[i]!
    const shown = line.length > 2000 ? `${line.slice(0, 2000)} … [line truncated; ${line.length} chars]` : line
    const numbered = `${i + 1}\t${shown}`
    if (chars + numbered.length + 1 > max - 150) break
    out.push(numbered)
    chars += numbered.length + 1
  }
  const next = offset + out.length
  if (next <= lines.length) out.push(`… (${lines.length - next + 1} more lines; use offset=${next})`)
  return { text: out.join("\n"), count: next - offset }
}

/** Diagnostic candidates only; similarity never authorizes a replacement. */
export function editCandidates(text: string, old: string): string {
  const lines = text.split("\n")
  const tokens = new Set(old.match(/[\w$]+/g) ?? [])
  const ranked = lines.map((l, i) => ({ i, score: (l.match(/[\w$]+/g) ?? []).filter((w) => tokens.has(w)).length }))
    .filter((x) => x.score > 0).sort((a, b) => b.score - a.score || a.i - b.i)
  const chosen: number[] = []
  for (const x of ranked) {
    if (chosen.every((i) => Math.abs(i - x.i) > 3)) chosen.push(x.i)
    if (chosen.length === 3) break
  }
  return chosen.length ? "\nCandidate regions (not applied):\n" + chosen.map((i) => readWindow(lines, Math.max(1, i), 5, 3000).text).join("\n---\n") : ""
}

export function postEditContext(before: string, after: string): string {
  const a = before.split("\n"), b = after.split("\n")
  let i = 0
  while (i < a.length && i < b.length && a[i] === b[i]) i++
  return readWindow(b, Math.max(1, i - 1), 8, 4000).text
}
