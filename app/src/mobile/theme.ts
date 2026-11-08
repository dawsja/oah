import { create } from "zustand"
import type { Theme } from "@/lib/types"

const KEY = "oah.phone.theme"
const QUERY = "(prefers-color-scheme: dark)"
let media: MediaQueryList | null = null

export const usePhoneTheme = create<{ theme: Theme; dark: boolean }>(() => ({ theme: "system", dark: true }))

function apply(theme: Theme) {
  const dark = theme === "dark" || (theme === "system" && (media?.matches ?? true))
  document.documentElement.classList.toggle("dark", dark)
  usePhoneTheme.setState({ theme, dark })
  // The WebView uses the shared desktop CSS; Android's system bars need matching icon colors.
  void import("@tauri-apps/api/core").then(({ invoke }) => invoke("phone_theme", { dark })).catch(() => {})
}

export function setPhoneTheme(theme: Theme) {
  try { localStorage.setItem(KEY, theme) } catch { /* Still apply when storage is unavailable. */ }
  apply(theme)
}

/** Install once at startup, before React renders. The preference belongs to this phone. */
export function initPhoneTheme() {
  media = window.matchMedia(QUERY)
  let theme: Theme = "system"
  try {
    const saved = localStorage.getItem(KEY)
    if (saved === "dark" || saved === "light") theme = saved
  } catch { /* Use the system default. */ }
  apply(theme)
  const update = () => apply(usePhoneTheme.getState().theme)
  media.addEventListener("change", update)
  // Recheck after returning from Android Settings, including WebViews that miss the change event.
  const resume = () => { if (document.visibilityState === "visible") update() }
  document.addEventListener("visibilitychange", resume)
  const watched = media
  return () => {
    watched.removeEventListener("change", update)
    document.removeEventListener("visibilitychange", resume)
  }
}
