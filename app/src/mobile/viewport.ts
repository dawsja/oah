// The on-screen keyboard on an iPhone. Android's WebView resizes the page for it (the viewport
// tag's interactive-widget=resizes-content). Safari's lays it over the page instead: only the
// visual viewport shrinks, and it slides down the page to show the focused field. So there the
// app (mobile.css: html.ios.keyboard body) is sized and moved to fill the visual viewport itself
// while the keyboard is up.

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

/** `OAH_DEBUG=1`: the numbers this file works from, on the screen, for a screenshot from a
 *  simulator (there's no console to read there). */
export function showViewportNumbers() {
  const probe = document.createElement("div")
  probe.style.cssText = "position:absolute;visibility:hidden;padding:env(safe-area-inset-top) env(safe-area-inset-right) env(safe-area-inset-bottom) env(safe-area-inset-left)"
  const out = document.createElement("pre")
  out.style.cssText = "position:absolute;top:22%;left:8px;right:8px;z-index:9999;margin:0;padding:8px;font:11px/1.45 ui-monospace,monospace;background:#000d;color:#7f7;white-space:pre-wrap;pointer-events:none"
  document.body.append(probe, out)
  // The native side's view of the same thing (an iPhone's web view in its window), and what a
  // fresh layout of it changes: asked once at the start, then again with the fix after a while.
  let native: string[] = []
  const ask = (fix: boolean) =>
    void import("@tauri-apps/api/core")
      .then(({ invoke }) => invoke<{ lines: string[] }>("debug_layout", { fix }))
      .then((l) => (native = [fix ? "after a fresh layout:" : "native:", ...l.lines]))
      .catch((e) => (native = [`native: ${e}`]))
  ask(false)
  setTimeout(() => ask(true), 10000)
  const show = () => {
    const visual = window.visualViewport
    const pad = getComputedStyle(probe)
    const body = document.body.getBoundingClientRect()
    out.textContent = [
      `html.${document.documentElement.className.replace(/ /g, ".")}`,
      `inner ${window.innerWidth}x${window.innerHeight}  client ${document.documentElement.clientWidth}x${document.documentElement.clientHeight}  screen ${screen.width}x${screen.height}`,
      `visual ${visual ? `${visual.width.toFixed(1)}x${visual.height.toFixed(1)} top ${visual.offsetTop.toFixed(1)} page ${visual.pageTop.toFixed(1)} scale ${visual.scale}` : "none"}`,
      `body ${body.width.toFixed(1)}x${body.height.toFixed(1)} at ${body.top.toFixed(1)} ${getComputedStyle(document.body).position}`,
      `safe ${pad.paddingTop} ${pad.paddingRight} ${pad.paddingBottom} ${pad.paddingLeft}`,
      `app-height ${document.documentElement.style.getPropertyValue("--app-height") || "-"} app-top ${document.documentElement.style.getPropertyValue("--app-top") || "-"}`,
      ...native,
    ].join("\n")
  }
  show()
  setInterval(show, 300)
}
