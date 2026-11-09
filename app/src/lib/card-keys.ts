import { inTerminal } from "./terminal"

/** Something on top that owns the keyboard: a dialog, a menu, a select's list, the palette. */
const OVERLAY = '[role="dialog"],[role="alertdialog"],[role="menu"],[role="listbox"]'

type Key = Pick<KeyboardEvent, "target" | "repeat" | "ctrlKey" | "altKey" | "metaKey">

/**
 * Whether a card docked above the composer (an approval, the agent's questions) takes a key:
 * - `"card"`: the key is the card's (nothing else is focused: the page, the card itself, or an
 *   empty message box);
 * - `"field"`: it's in a text field inside the card (only Esc means anything there);
 * - `null`: it belongs to something else. A focused button gets its own Enter, a field outside
 *   the card keeps its letters, and a menu or dialog on top keeps its Esc.
 * Held keys never count, so holding Enter can't walk through a queue of approvals.
 */
export function cardKeyScope(e: Key, card: Element | null, doc: Pick<Document, "querySelector" | "body" | "documentElement"> = document): "card" | "field" | null {
  if (e.repeat || e.ctrlKey || e.altKey || e.metaKey) return null
  const el = e.target as (Element & { value?: string; dataset?: DOMStringMap }) | null
  if (inTerminal(el)) return null
  if (card && el && typeof card.contains === "function" && card.contains(el)) {
    const tag = el.tagName
    if (tag === "INPUT" || tag === "TEXTAREA") return "field"
    if (typeof el.closest === "function" && el.closest("button, a, [role=button]")) return null
    return doc.querySelector(OVERLAY) ? null : "card"
  }
  if (doc.querySelector(OVERLAY)) return null
  if (!el || el === doc.body || el === doc.documentElement || (el as unknown) === doc) return "card"
  // The message box, while it's empty (typing there is typing, not answering).
  if (el.tagName === "TEXTAREA" && el.dataset?.composer !== undefined) return (el.value ?? "").trim() === "" ? "card" : null
  return null
}

/** Keys that arrive right as a card appears were meant for what was there before it. */
export const CARD_ARM_MS = 300
