// Text handed to the chat from elsewhere in the window: a file mention from the side panel,
// terminal output to ask about. It goes where the active pane takes typing: its composer, or
// its agent's CLI when the chat is open there.

import { paneOf, useApp } from "./app"
import { pasteIntoCli } from "./cli"

/** What a composer hears (`oah:insert`): `pane` is the one it's for. */
export type InsertDetail = { pane: string; text: string }

export function insertIntoChat(text: string) {
  const app = useApp.getState()
  if (app.view !== "chat") app.closeSettings()
  const pane = paneOf(app)
  // A file or diff on screen in the chat's place: back to the chat, which takes it.
  if (pane.viewer) app.setPane({ viewer: null }, pane.id)
  if (pane.chatId && pasteIntoCli(pane.chatId, text.replace(/\n+$/, ""))) return
  // Once the composer is there (it isn't while a file is shown).
  requestAnimationFrame(() => requestAnimationFrame(() => window.dispatchEvent(new CustomEvent<InsertDetail>("oah:insert", { detail: { pane: pane.id, text } }))))
}
