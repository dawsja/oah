// Finding a chat by its title in the Ctrl+K palette (its text is searched by the app: chat_search).

import type { ChatMeta } from "./chat"

/** The words of a query, lowercased. */
export const termsOf = (query: string) => query.toLowerCase().split(/\s+/).filter(Boolean)

/** Chats whose title (or project's name) has every term, best first: titles that start with
 *  the first term, then the store's order (newest activity). */
export function titleMatches(metas: ChatMeta[], projectName: (id: string) => string, query: string): ChatMeta[] {
  const terms = termsOf(query)
  if (!terms.length) return []
  const hay = (m: ChatMeta) => `${m.title || "New chat"} ${projectName(m.projectId)}`.toLowerCase()
  const found = metas.filter((m) => terms.every((t) => hay(m).includes(t)))
  const starts = (m: ChatMeta) => (m.title.toLowerCase().startsWith(terms[0]) ? 0 : 1)
  return found.map((m, i) => [m, i] as const).sort((a, b) => starts(a[0]) - starts(b[0]) || a[1] - b[1]).map(([m]) => m)
}
