// Desktop notifications while oah is in the background: a turn finished, background work
// (a command, a subagent) ended, or an agent is waiting for an approval or an answer. Nothing is sent while the window has focus. Each one
// carries its chat's id: a click opens the chat (attention.ts), and one that waits on the user
// has an Answer button too.

import type { BackgroundTask, ChatEvent, ChatMeta } from "./chat"

let allowed: boolean | null = null

async function permitted(): Promise<boolean> {
  if (allowed !== null) return allowed
  const n = await import("@tauri-apps/plugin-notification")
  allowed = (await n.isPermissionGranted()) || (await n.requestPermission()) === "granted"
  return allowed
}

/** Per chat, the background tasks running as last heard, to tell which ones just ended. */
const runningTasks = new Map<string, Set<string>>()

/** The tasks in `tasks` that were running and aren't now (and remembers what runs now). */
export function endedTasks(chatId: string, tasks: BackgroundTask[]): BackgroundTask[] {
  const before = runningTasks.get(chatId) ?? new Set<string>()
  runningTasks.set(chatId, new Set(tasks.filter((t) => t.status === "running").map((t) => t.id)))
  return tasks.filter((t) => t.status !== "running" && before.has(t.id))
}

/** What to say about `event`, if anything. */
export function notification(event: ChatEvent, meta: ChatMeta | undefined): { title: string; body: string; waits?: true } | null {
  const chat = meta?.title || "A chat"
  switch (event.type) {
    case "tasks": {
      // Stopped ones were stopped on purpose (or went with the agent): no news.
      const ended = endedTasks(meta?.id ?? "", event.tasks).filter((t) => t.status !== "stopped")
      if (!ended.length) return null
      const failed = ended.some((t) => t.status === "failed")
      const what = ended.length === 1 ? ended[0].title : `${ended.length} background tasks`
      return { title: failed ? "Background work failed" : "Background work finished", body: `${chat}: ${what}` }
    }
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

/** One notification per chat and kind per few seconds (a burst of approvals is one ping). A
 *  "waiting for you" is never held back by the "Done" just before it. */
const last = new Map<string, number>()
const COOLDOWN = 5_000

/** Chats with messages queued behind the running turn: their "Done" isn't the end. */
let hasQueued: (chatId: string) => boolean = () => false
export function setQueuedCheck(check: (chatId: string) => boolean) {
  hasQueued = check
}
let nextId = 1

export async function notify(event: ChatEvent, meta: ChatMeta | undefined) {
  // Task lists are followed even while focused, so an end is told apart from a stale list.
  const n = event.type === "tasks" || !document.hasFocus() ? notification(event, meta) : null
  if (document.hasFocus()) return
  if (!n) return
  // The next queued message goes as this turn ends; the last one's end is the "Done".
  if (event.type === "turnFinished" && meta && hasQueued(meta.id)) return
  const key = `${meta?.id ?? ""}|${n.waits ? "waits" : "other"}`
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
