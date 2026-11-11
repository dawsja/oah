import { describe, expect, it } from "vitest"
import { shortOfTheScreen, visibleBox } from "./viewport"

describe("the app's box on an iPhone", () => {
  it("is the whole page when no keyboard is up", () => {
    expect(visibleBox(874, { height: 874, offsetTop: 0 })).toBeNull()
    // Rounding in the visual viewport isn't a keyboard.
    expect(visibleBox(874, { height: 873.6, offsetTop: 0 })).toBeNull()
    // Nor is a visual viewport that leaves out the notch and the home indicator (62 + 34).
    expect(visibleBox(874, { height: 778, offsetTop: 0 })).toBeNull()
  })

  it("shrinks to what the keyboard leaves", () => {
    expect(visibleBox(874, { height: 538, offsetTop: 0 })).toEqual({ height: 538, top: 0 })
  })

  it("follows the viewport when Safari slides it down to the focused field", () => {
    expect(visibleBox(874, { height: 538, offsetTop: 211.5 })).toEqual({ height: 538, top: 212 })
    expect(visibleBox(874, { height: 538, offsetTop: -3 })?.top).toBe(0)
  })
})

describe("a page WebKit left shorter than the screen", () => {
  it("is short by the notch and the home indicator", () => {
    expect(shortOfTheScreen(874, 778)).toBe(true)
    expect(shortOfTheScreen(667, 647)).toBe(true)
  })

  it("isn't when it fills the screen, or when a keyboard took the room", () => {
    expect(shortOfTheScreen(874, 874)).toBe(false)
    expect(shortOfTheScreen(874, 873.5)).toBe(false)
    expect(shortOfTheScreen(874, 494)).toBe(false)
  })
})
