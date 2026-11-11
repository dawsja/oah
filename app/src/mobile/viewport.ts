// The on-screen keyboard on an iPhone. Android's WebView resizes the page for it (the viewport
// tag's interactive-widget=resizes-content). Safari's lays it over the page instead: only the
// visual viewport shrinks, and it slides down the page to show the focused field. So there the
// app (mobile.css: html.ios body) is sized and moved to fill the visual viewport itself.

import { isIOS } from "./rpc"

/** Where the app sits: the visual viewport's height and offset in the page, and whether a
 *  keyboard is taking the rest. */
export function visibleBox(pageHeight: number, visual: { height: number; offsetTop: number }): { height: number; top: number; keyboard: boolean } {
  const height = Math.round(visual.height)
  // A keyboard takes far more than the pixel or two rounding can.
  return { height, top: Math.max(0, Math.round(visual.offsetTop)), keyboard: pageHeight - height > 100 }
}

/** Install once at startup. Does nothing on Android. */
export function initViewport(): () => void {
  const visual = window.visualViewport
  if (!isIOS() || !visual) return () => {}
  const root = document.documentElement
  root.classList.add("ios")
  const update = () => {
    const box = visibleBox(root.clientHeight, visual)
    root.style.setProperty("--app-height", `${box.height}px`)
    root.style.setProperty("--app-top", `${box.top}px`)
    root.classList.toggle("keyboard", box.keyboard)
  }
  update()
  visual.addEventListener("resize", update)
  visual.addEventListener("scroll", update)
  return () => {
    visual.removeEventListener("resize", update)
    visual.removeEventListener("scroll", update)
  }
}
