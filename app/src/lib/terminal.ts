// Terminal helpers that don't need a terminal: how a tab is labelled, the colors, and whether
// a key event came from inside a terminal.

import type { ITheme } from "@xterm/xterm"
import type { Location } from "./types"

/** The OSC a remote shell's start script names itself with (crates/oah-transport/src/pty.rs). */
export const SHELL_OSC = 6973

/** `shell=bash` → `bash` (anything else: null). */
export function parseShellOsc(data: string): string | null {
  const m = /^shell=([\w.+-]{1,32})$/.exec(data.trim())
  return m ? m[1] : null
}

/** How a shell is named in the UI. */
export function shellLabel(shell: string | null | undefined): string {
  if (!shell) return "Terminal"
  const s = shell.toLowerCase()
  return s === "pwsh" ? "PowerShell" : s === "powershell" ? "Windows PowerShell" : s
}

/** One key per system a shell runs on (the same shell everywhere on it). */
export function locationKey(l: Location): string {
  return l.kind === "local" ? "local" : l.kind === "wsl" ? `wsl:${l.distro}` : `ssh:${l.host}`
}

/** The last folder of a path, either slash. */
export function baseName(path: string): string {
  return path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || path
}

/** The target is inside a terminal, whose keys belong to the shell (not the app's shortcuts). */
export function inTerminal(target: EventTarget | null): boolean {
  const el = target as Element | null
  return typeof el?.closest === "function" && !!el.closest(".xterm")
}

const DARK = {
  black: "#2a2a30",
  red: "#f87171",
  green: "#4ade80",
  yellow: "#fbbf24",
  blue: "#60a5fa",
  magenta: "#c084fc",
  cyan: "#22d3ee",
  white: "#d4d4d8",
  brightBlack: "#6a6a73",
  brightRed: "#fca5a5",
  brightGreen: "#86efac",
  brightYellow: "#fde68a",
  brightBlue: "#93c5fd",
  brightMagenta: "#d8b4fe",
  brightCyan: "#67e8f9",
  brightWhite: "#fafafa",
}

// "White" stays readable on the light paper.
const LIGHT = {
  black: "#16181d",
  red: "#b91c1c",
  green: "#047857",
  yellow: "#b45309",
  blue: "#1d4ed8",
  magenta: "#7e22ce",
  cyan: "#0e7490",
  white: "#8b919e",
  brightBlack: "#5b6170",
  brightRed: "#dc2626",
  brightGreen: "#059669",
  brightYellow: "#d97706",
  brightBlue: "#2563eb",
  brightMagenta: "#9333ea",
  brightCyan: "#0891b2",
  brightWhite: "#6b7180",
}

/** The terminal colors for the theme on screen: the side panels' background, oah's text. */
export function terminalTheme(dark: boolean, background: string, foreground: string): ITheme {
  return {
    ...(dark ? DARK : LIGHT),
    background,
    foreground,
    cursor: foreground,
    cursorAccent: background,
    selectionBackground: dark ? "rgba(245, 184, 0, 0.28)" : "rgba(245, 184, 0, 0.35)",
    scrollbarSliderBackground: dark ? "rgba(255, 255, 255, 0.09)" : "rgba(15, 23, 42, 0.14)",
    scrollbarSliderHoverBackground: dark ? "rgba(255, 255, 255, 0.14)" : "rgba(15, 23, 42, 0.24)",
    scrollbarSliderActiveBackground: dark ? "rgba(255, 255, 255, 0.2)" : "rgba(15, 23, 42, 0.3)",
  }
}
