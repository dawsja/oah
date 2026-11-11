// The on-screen keyboard on an iPhone. Android's WebView resizes the page for it (the viewport
// tag's interactive-widget=resizes-content). Safari's lays it over the page instead: only the
// visual viewport shrinks, and it slides down the page to show the focused field. So there the
// app (mobile.css: html.ios body) is sized and moved to fill the visual viewport itself.

import { isIOS } from "./rpc"

/** Where the app sits while a keyboard is up: the visual viewport's height and offset in the
 *  page. Null with no keyboard: the app is the page, all of it. */
export function visibleBox(pageHeight: number, visual: { height: number; offsetTop: number }): { height: number; top: number } | null {
  const height = Math.round(visual.height)
  // A keyboard takes a third of the screen. Less than that is rounding, or the notch and home
  // indicator, which some WebViews leave out of the visual viewport.
  if (pageHeight - height <= 150) return null
  return { height, top: Math.max(0, Math.round(visual.offsetTop)) }
}

/** Install once at startup. Does nothing on Android. */
export function initViewport(): () => void {
  const visual = window.visualViewport
  if (!isIOS() || !visual) return () => {}
  const root = document.documentElement
  root.classList.add("ios")
  const update = () => {
    const box = visibleBox(root.clientHeight, visual)
    root.classList.toggle("keyboard", !!box)
    if (box) {
      root.style.setProperty("--app-height", `${box.height}px`)
      root.style.setProperty("--app-top", `${box.top}px`)
    } else {
      root.style.removeProperty("--app-height")
      root.style.removeProperty("--app-top")
    }
  }
  update()
  visual.addEventListener("resize", update)
  visual.addEventListener("scroll", update)
  return () => {
    visual.removeEventListener("resize", update)
    visual.removeEventListener("scroll", update)
  }
}
