// Desktop notifications while oah is in the background: a turn finished, or an agent is
// waiting for an approval or an answer. Nothing is sent while the window has focus.

import type { ChatEvent, ChatMeta } from "./chat"
import { inTauri } from "./ipc"

let allowed: boolean | null = null

async function permitted(): Promise<boolean> {
  if (!inTauri) return false
  if (allowed !== null) return allowed
  const n = await import("@tauri-apps/plugin-notification")
  allowed = (await n.isPermissionGranted()) || (await n.requestPermission()) === "granted"
  return allowed
}

/** What to say about `event`, if anything. */
export function notification(event: ChatEvent, meta: ChatMeta | undefined): { title: string; body: string } | null {
  const chat = meta?.title || "A chat"
  switch (event.type) {
    case "turnFinished":
      return event.interrupted ? null : { title: "Done", body: chat }
    case "approval":
      return { title: "Waiting for you", body: `${chat}: ${event.request.title}` }
    case "question":
      return { title: "A question for you", body: `${chat}: ${event.request.questions[0]?.question ?? ""}` }
    case "error":
      return { title: "Something went wrong", body: `${chat}: ${event.message}` }
    default:
      return null
  }
}

/** One notification per chat per few seconds (a burst of approvals is one ping). */
const last = new Map<string, number>()
const COOLDOWN = 5_000

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
  sendNotification(n)
}
