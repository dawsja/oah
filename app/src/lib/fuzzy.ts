// Fuzzy path matching for the composer's @ menu: each term is a greedy subsequence of the
// lowercased path; gaps cost, matches right after / . - _ or at a camelCase hump earn, a term
// inside the file name earns a lot. Lower scores are better.

const BOUNDARY = new Set(["/", ".", "-", "_", "\\"])

function scoreTerm(path: string, lower: string, term: string): number | null {
  let score = 0
  let at = -1
  for (const ch of term) {
    const want = ch === "-" || ch === "_" ? ["-", "_"] : [ch]
    let found = -1
    for (let i = at + 1; i < lower.length; i++) {
      if (want.includes(lower[i])) {
        found = i
        break
      }
    }
    if (found < 0) return null
    score += at < 0 ? 0 : found - at - 1
    const prev = path[found - 1]
    if (found === 0 || BOUNDARY.has(prev) || (path[found] !== lower[found] && prev === prev?.toLowerCase())) score -= 5
    at = found
  }
  const name = lower.slice(lower.lastIndexOf("/") + 1)
  if (name.includes(term)) score -= 100
  return score
}

/** Paths matching `query` (space-separated terms, all must match), best first, at most `limit`. */
export function fuzzyPaths(paths: string[], query: string, limit = 20): string[] {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean)
  if (!terms.length) return paths.slice(0, limit)
  const scored: [number, string][] = []
  for (const p of paths) {
    const lower = p.toLowerCase()
    let total = 0
    let ok = true
    for (const t of terms) {
      const s = scoreTerm(p, lower, t)
      if (s === null) {
        ok = false
        break
      }
      total += s
    }
    if (ok) scored.push([total + p.length / 1000, p])
  }
  scored.sort((a, b) => a[0] - b[0])
  return scored.slice(0, limit).map(([, p]) => p)
}

/** The `@query` being typed right before the caret, if any. */
export function mentionAt(text: string, caret: number): { start: number; query: string } | null {
  const m = /(?:^|\s)@(\S*)$/.exec(text.slice(0, caret))
  if (!m) return null
  return { start: caret - m[1].length - 1, query: m[1] }
}

/** How a path goes into the prompt: `@path`, quoted when it has spaces or @. */
export const mention = (path: string) => (/[\s@]/.test(path) ? `@"${path}"` : `@${path}`)
