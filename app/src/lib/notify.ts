// Desktop notifications while oah is in the background: a turn finished, or an agent is
// waiting for an approval or an answer. Nothing is sent while the window has focus. Each one
// carries its chat's id: a click opens the chat (attention.ts), and one that waits on the user
// has an Answer button too.

import type { ChatEvent, ChatMeta } from "./chat"

let allowed: boolean | null = null

async function permitted(): Promise<boolean> {
  if (allowed !== null) return allowed
  const n = await import("@tauri-apps/plugin-notification")
  allowed = (await n.isPermissionGranted()) || (await n.requestPermission()) === "granted"
  return allowed
}

/** What to say about `event`, if anything. */
export function notification(event: ChatEvent, meta: ChatMeta | undefined): { title: string; body: string; waits?: true } | null {
  const chat = meta?.title || "A chat"
  switch (event.type) {
    case "turnFinished":
      return event.interrupted ? null : { title: "Done", body: chat }
    case "approval":
      return { title: "Waiting for you", body: `${chat}: ${event.request.title}`, waits: true }
    case "question":
      return { title: "A question for you", body: `${chat}: ${event.request.questions[0]?.question ?? ""}`, waits: true }
    case "error":
      return { title: "Something went wrong", body: `${chat}: ${event.message}` }
    default:
      return null
  }
}

/** One notification per chat per few seconds (a burst of approvals is one ping). */
const last = new Map<string, number>()
const COOLDOWN = 5_000
let nextId = 1

export async function notify(event: ChatEvent, meta: ChatMeta | undefined) {
  if (document.hasFocus()) return
  const n = notification(event, meta)
  if (!n) return
  const key = meta?.id ?? ""
  const now = Date.now()
  if (now - (last.get(key) ?? 0) < COOLDOWN) return
  last.set(key, now)
  if (!(await permitted())) return
  const { sendNotification } = await import("@tauri-apps/plugin-notification")
  sendNotification({
    id: nextId++,
    title: n.title,
    body: n.body,
    // The action type registered by the app (attention.rs): an Answer button.
    actionTypeId: n.waits ? "waiting" : undefined,
    extra: meta ? { chatId: meta.id } : undefined,
  })
}
