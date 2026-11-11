// Chats waiting on the user, outside the chat list: the taskbar/dock badge with their count, and
// a click on a notification opening the chat it was about.

import { needsYou, useChats } from "@/store/chats"
import { useApp } from "@/store/app"
import { ipc, onNotificationAction } from "./ipc"

let started = false

export function initAttention() {
  if (started) return
  started = true
  let shown = -1
  const update = () => {
    const count = needsYou(useChats.getState()).length
    if (count === shown) return
    shown = count
    void ipc.setBadge(count, useApp.getState().info?.platform === "windows" ? badgeIcon(count) : null).catch(() => undefined)
  }
  useChats.subscribe(update)
  update()
  void onNotificationAction((chatId, action) => {
    const chats = useChats.getState()
    const meta = chats.metas.find((m) => m.id === chatId)
    if (!meta) return
    if (useApp.getState().view === "settings") useApp.getState().closeSettings()
    // Answer, or a click on a notification about a chat that still waits: straight to the card.
    if (action === "answer" || needsYou(chats).includes(chatId)) void chats.answerChat(chatId)
    else void chats.open(chatId)
  })
}

/** What the badge says: the count, `9+` past nine. */
export const badgeText = (count: number) => (count > 9 ? "9+" : String(count))

/** Windows' taskbar overlay: the count on an accent disc, as 32×32 RGBA; null for none. */
function badgeIcon(count: number): number[] | null {
  if (count <= 0) return null
  const size = 32
  const canvas = document.createElement("canvas")
  canvas.width = canvas.height = size
  const g = canvas.getContext("2d")
  if (!g) return null
  g.fillStyle = "#ffd43b"
  g.beginPath()
  g.arc(size / 2, size / 2, size / 2, 0, Math.PI * 2)
  g.fill()
  const text = badgeText(count)
  g.fillStyle = "#0c0c0c"
  g.font = `700 ${text.length > 1 ? 17 : 22}px "Inter Variable", system-ui, sans-serif`
  g.textAlign = "center"
  g.textBaseline = "middle"
  g.fillText(text, size / 2, size / 2 + 1)
  return Array.from(g.getImageData(0, 0, size, size).data)
}
