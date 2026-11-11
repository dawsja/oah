import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { initPhoneTheme, setPhoneTheme, usePhoneTheme } from "./theme"

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn().mockResolvedValue(undefined) }))
vi.mock("@tauri-apps/api/core", () => ({ invoke }))
let saved: string | null
let darkClass: boolean
let change: () => void
let resume: () => void
let cleanup: () => void
const media = { matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }

beforeEach(() => {
  saved = null
  darkClass = false
  media.matches = false
  vi.clearAllMocks()
  media.addEventListener.mockImplementation((_name, fn) => { change = fn })
  vi.stubGlobal("window", { matchMedia: () => media })
  vi.stubGlobal("localStorage", {
    getItem: vi.fn(() => saved),
    setItem: vi.fn((_key, value) => { saved = value }),
  })
  vi.stubGlobal("document", {
    visibilityState: "visible",
    documentElement: { classList: { toggle: (_name: string, on: boolean) => { darkClass = on } } },
    addEventListener: vi.fn((_name, fn) => { resume = fn }),
    removeEventListener: vi.fn(),
  })
})
afterEach(() => { cleanup?.(); vi.unstubAllGlobals() })

describe("phone theme preference", () => {
  it("defaults to System and follows live system changes and foregrounding", () => {
    cleanup = initPhoneTheme()
    expect(usePhoneTheme.getState()).toEqual({ theme: "system", dark: false })
    media.matches = true
    change()
    expect(darkClass).toBe(true)
    media.matches = false
    resume()
    expect(darkClass).toBe(false)
  })

  it.each(["light", "dark"] as const)("restores %s and ignores system changes until System is selected", (theme) => {
    saved = theme
    cleanup = initPhoneTheme()
    expect(darkClass).toBe(theme === "dark")
    media.matches = theme !== "dark"
    change()
    expect(darkClass).toBe(theme === "dark")
    setPhoneTheme("system")
    expect(saved).toBe("system")
    expect(darkClass).toBe(media.matches)
  })

  it("persists a manual choice only in phone storage and updates native chrome", async () => {
    cleanup = initPhoneTheme()
    setPhoneTheme("light")
    expect(localStorage.setItem).toHaveBeenCalledWith("oah.phone.theme", "light")
    expect(usePhoneTheme.getState()).toEqual({ theme: "light", dark: false })
    await vi.waitFor(() => expect(invoke).toHaveBeenLastCalledWith("phone_theme", { dark: false, system: false }))
    cleanup()
    cleanup = initPhoneTheme()
    expect(usePhoneTheme.getState().theme).toBe("light")
  })

  it("falls back to System for invalid storage and still applies changes when storage fails", () => {
    saved = "invalid"
    cleanup = initPhoneTheme()
    expect(usePhoneTheme.getState().theme).toBe("system")
    vi.mocked(localStorage.setItem).mockImplementation(() => { throw new Error("Storage unavailable") })
    setPhoneTheme("dark")
    expect(darkClass).toBe(true)
  })
})
