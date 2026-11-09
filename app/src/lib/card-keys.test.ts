import { describe, expect, it } from "vitest"
import { cardKeyScope } from "./card-keys"

/** A minimal element: a tag, where it sits, and what `closest` finds. */
function el(tagName: string, opts: { in?: string[]; value?: string; composer?: boolean } = {}) {
  const inside = new Set(opts.in ?? [])
  return {
    tagName,
    value: opts.value,
    dataset: opts.composer ? { composer: "" } : {},
    closest: (sel: string) => (sel.split(",").some((s) => inside.has(s.trim()) || s.trim() === tagName.toLowerCase()) ? {} : null),
  } as unknown as Element
}

function doc(overlay = false) {
  const body = el("BODY")
  return { body, documentElement: el("HTML"), querySelector: () => (overlay ? {} : null) } as unknown as Document
}

const key = (target: unknown, extra: Partial<KeyboardEvent> = {}) =>
  ({ target, repeat: false, ctrlKey: false, altKey: false, metaKey: false, ...extra }) as unknown as KeyboardEvent

describe("cardKeyScope", () => {
  const inCard = new Set<unknown>()
  const card = { contains: (x: unknown) => inCard.has(x) } as unknown as Element

  it("takes keys when nothing else is focused, or the message box is empty", () => {
    const d = doc()
    expect(cardKeyScope(key(d.body), card, d)).toBe("card")
    expect(cardKeyScope(key(el("TEXTAREA", { composer: true, value: "  " })), card, d)).toBe("card")
    expect(cardKeyScope(key(el("TEXTAREA", { composer: true, value: "hi" })), card, d)).toBe(null)
  })

  it("leaves other fields, buttons, menus and held keys alone", () => {
    const d = doc()
    // The Files filter, a rename box: typing "a" there must not "always allow".
    expect(cardKeyScope(key(el("INPUT")), card, d)).toBe(null)
    // A focused Deny button gets its own Enter.
    const deny = el("BUTTON")
    inCard.add(deny)
    expect(cardKeyScope(key(deny), card, d)).toBe(null)
    expect(cardKeyScope(key(el("BUTTON")), card, d)).toBe(null)
    // A menu or dialog on top keeps its Esc.
    expect(cardKeyScope(key(doc(true).body), card, doc(true))).toBe(null)
    expect(cardKeyScope(key(d.body, { repeat: true }), card, d)).toBe(null)
    expect(cardKeyScope(key(d.body, { ctrlKey: true }), card, d)).toBe(null)
    expect(cardKeyScope(key(el("DIV", { in: [".xterm"] })), card, d)).toBe(null)
  })

  it("tells the card's own fields apart", () => {
    const d = doc()
    const field = el("INPUT")
    const region = el("DIV")
    inCard.add(field).add(region)
    expect(cardKeyScope(key(field), card, d)).toBe("field")
    expect(cardKeyScope(key(region), card, d)).toBe("card")
  })
})
