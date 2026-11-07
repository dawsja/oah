// What's typed in a composer and not sent yet, per chat (or per new-chat target), so leaving
// and coming back finds it still there. Text is kept across restarts; attached images only for
// the session (they can be megabytes).

import type { Attachment } from "@/components/chat/Composer"

const KEY = "oah.drafts"
/** Drafts kept at most; the oldest go first. */
const MAX = 200

type Saved = { text: string; at: number }

let texts: Record<string, Saved> = load()
const images = new Map<string, Attachment[]>()
let timer: ReturnType<typeof setTimeout> | null = null

function load(): Record<string, Saved> {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "{}")
    return v && typeof v === "object" ? v : {}
  } catch {
    return {}
  }
}

function flush() {
  if (timer) clearTimeout(timer)
  timer = null
  const keys = Object.keys(texts)
  if (keys.length > MAX) {
    keys.sort((a, b) => texts[a].at - texts[b].at)
    for (const k of keys.slice(0, keys.length - MAX)) delete texts[k]
  }
  try {
    localStorage.setItem(KEY, JSON.stringify(texts))
  } catch {
    // Storage full or unavailable: the drafts still last the session.
  }
}

window.addEventListener("beforeunload", flush)

/** The chat's draft, or a new chat's in `projectId` (and worktree `cwd`). */
export const draftKey = (chatId: string | null, projectId: string, cwd?: string | null) => (chatId ? `chat:${chatId}` : `new:${projectId}:${cwd ?? ""}`)

export const drafts = {
  get(key: string): { text: string; images: Attachment[] } {
    return { text: texts[key]?.text ?? "", images: images.get(key) ?? [] }
  },

  set(key: string, text: string, imgs: Attachment[]) {
    if (imgs.length) images.set(key, imgs)
    else images.delete(key)
    if ((texts[key]?.text ?? "") === text) return
    if (text) texts[key] = { text, at: Date.now() }
    else delete texts[key]
    if (!timer) timer = setTimeout(flush, 400)
  },

  drop(key: string) {
    images.delete(key)
    if (!(key in texts)) return
    delete texts[key]
    flush()
  },
}
