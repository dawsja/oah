// xterm.js terminals the way oah's look and behave. The shells under the chat (store/terminals)
// and the agents' own CLIs (store/cli) are both made here, so they share the font, the colors,
// the links and the keys the app keeps for itself.

import type { FitAddon } from "@xterm/addon-fit"
import type { Terminal } from "@xterm/xterm"
import { copyText } from "@/lib/clipboard"
import { terminalTheme } from "@/lib/terminal"

/** A terminal and the element it draws in (moved between hosts as React remounts them). */
export type Xterm = { term: Terminal; fit: FitAddon; el: HTMLDivElement }

const FONT_SIZE = 13

function fontFamily() {
  const mono = getComputedStyle(document.documentElement).getPropertyValue("--oah-mono").trim()
  return `${mono ? `${mono}, ` : ""}"JetBrains Mono Variable", "JetBrains Mono", ui-monospace, Consolas, monospace`
}

function theme() {
  const css = getComputedStyle(document.documentElement)
  const dark = document.documentElement.classList.contains("dark")
  return terminalTheme(dark, css.getPropertyValue("--sidebar").trim() || (dark ? "#080808" : "#eef0f3"), css.getPropertyValue("--foreground").trim() || (dark ? "#ededed" : "#16181d"))
}

const made = new Set<Xterm>()

// Theme and font changes (Settings → Appearance) reach open terminals.
let watching = false
function watchAppearance() {
  if (watching) return
  watching = true
  new MutationObserver(() => {
    const t = theme()
    const f = fontFamily()
    for (const x of made) {
      x.term.options.theme = t
      if (x.term.options.fontFamily !== f) {
        x.term.options.fontFamily = f
        fitXterm(x)
      }
    }
  }).observe(document.documentElement, { attributes: true, attributeFilter: ["class", "style"] })
}

type Modules = { Terminal: typeof import("@xterm/xterm").Terminal; FitAddon: typeof import("@xterm/addon-fit").FitAddon; WebLinksAddon: typeof import("@xterm/addon-web-links").WebLinksAddon }

/** xterm.js, loaded when the first terminal is wanted, and the font its cells are measured with. */
export async function loadXterm(): Promise<Modules> {
  const [{ Terminal }, { FitAddon }, { WebLinksAddon }] = await Promise.all([import("@xterm/xterm"), import("@xterm/addon-fit"), import("@xterm/addon-web-links")])
  // Measure the mono font once it's loaded, or the cells come out the wrong width.
  await document.fonts.load(`${FONT_SIZE}px "JetBrains Mono Variable"`).catch(() => undefined)
  return { Terminal, FitAddon, WebLinksAddon }
}

/** A terminal drawing in `host`, fitted to it. `windows`: what runs in it is behind a Windows
 *  pseudo console (every shell there, ssh.exe and wsl.exe too). */
export function makeXterm(m: Modules, host: HTMLElement, windows: boolean): Xterm {
  const el = document.createElement("div")
  el.className = "h-full w-full"
  host.appendChild(el)
  const term = new m.Terminal({
    fontFamily: fontFamily(),
    fontSize: FONT_SIZE,
    lineHeight: 1.2,
    cursorBlink: true,
    scrollback: 10_000,
    theme: theme(),
    windowsPty: windows ? { backend: "conpty" } : undefined,
  })
  const fit = new m.FitAddon()
  term.loadAddon(fit)
  term.loadAddon(
    new m.WebLinksAddon((_e, uri) => {
      import("@tauri-apps/plugin-opener").then((o) => o.openUrl(uri)).catch(() => undefined)
    }),
  )
  const x: Xterm = { term, fit, el }
  made.add(x)
  watchAppearance()
  term.attachCustomKeyEventHandler((e) => keyForTerminal(x, e))
  term.open(el)
  fitXterm(x)
  return x
}

export function disposeXterm(x: Xterm) {
  made.delete(x)
  x.term.dispose()
  x.el.remove()
}

/** Fits the terminal to its element, when that's on screen to be measured. */
export function fitXterm(x: Xterm) {
  if (!x.el.isConnected || x.el.offsetWidth === 0 || x.el.offsetHeight === 0) return
  try {
    x.fit.fit()
  } catch {
    /* not measurable yet */
  }
}

/** Puts the terminal in `host` (React remounted it, or the terminal moved panes). */
export function moveXterm(x: Xterm, host: HTMLElement) {
  if (x.el.parentElement !== host) host.appendChild(x.el)
  fitXterm(x)
}

/** Chords that are the app's even with the cursor in a terminal: the terminal toggle and
 *  Settings, moving between panes (Ctrl+1, Ctrl+2), closing one (Ctrl+Shift+\) and a chat's
 *  way in and out of its agent's CLI (Ctrl+Shift+M). Everything else there is the program's. */
export function isAppChord(e: KeyboardEvent): boolean {
  if (!(e.ctrlKey || e.metaKey) || e.altKey) return false
  if (e.code === "Backquote" || e.key === ",") return true
  if (!e.shiftKey && (e.code === "Digit1" || e.code === "Digit2")) return true
  return e.shiftKey && (e.code === "Backslash" || e.code === "KeyM")
}

/** Whether the terminal gets the key: not the app's chords, nor copy and paste. */
function keyForTerminal(x: Xterm, e: KeyboardEvent): boolean {
  const mod = e.ctrlKey || e.metaKey
  if (!mod || e.altKey) return true
  if (isAppChord(e)) return false
  const k = e.key.toLowerCase()
  // Ctrl+Shift+C always copies; Ctrl+C copies when there's a selection (else it's ^C).
  if (k === "c" && (e.shiftKey || x.term.hasSelection())) {
    if (e.type === "keydown" && x.term.hasSelection()) {
      copyText(x.term.getSelection())
      x.term.clearSelection()
    }
    e.preventDefault()
    return false
  }
  // Ctrl+V and Ctrl+Shift+V: the browser's own paste, which xterm turns into input.
  if (k === "v") return false
  return true
}
