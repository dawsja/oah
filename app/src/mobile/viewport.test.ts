import { describe, expect, it } from "vitest"
import { visibleBox } from "./viewport"

describe("the app's box on an iPhone", () => {
  it("fills the screen when no keyboard is up", () => {
    expect(visibleBox(844, { height: 844, offsetTop: 0 })).toEqual({ height: 844, top: 0, keyboard: false })
    // Rounding in the visual viewport isn't a keyboard.
    expect(visibleBox(844, { height: 843.6, offsetTop: 0 }).keyboard).toBe(false)
  })

  it("shrinks to what the keyboard leaves", () => {
    expect(visibleBox(844, { height: 508, offsetTop: 0 })).toEqual({ height: 508, top: 0, keyboard: true })
  })

  it("follows the viewport when Safari slides it down to the focused field", () => {
    expect(visibleBox(844, { height: 508, offsetTop: 211.5 })).toEqual({ height: 508, top: 212, keyboard: true })
    expect(visibleBox(844, { height: 508, offsetTop: -3 }).top).toBe(0)
  })
})
