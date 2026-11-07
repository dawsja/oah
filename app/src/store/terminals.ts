// The terminal panel's tabs and the terminals behind them. Each tab is an xterm.js terminal
// attached to a shell where the project is (this computer, a WSL distro, an SSH host). The
// terminals live here, outside React: hiding the panel or switching projects only hides them,
// so a shell (and an ssh or WSL session) stays connected until its tab is closed.

import { create } from "zustand"
import type { Terminal } from "@xterm/xterm"
import type { FitAddon } from "@xterm/addon-fit"
import { copyText } from "@/lib/clipboard"
import { terminal as ipc } from "@/lib/ipc"
import { locationKey, parseShellOsc, SHELL_OSC, terminalTheme } from "@/lib/terminal"
import type { Location } from "@/lib/types"
import { useApp } from "./app"
import { currentScope } from "./git"

export type TermTab = {
  id: string
  projectId: string
  /** The folder it started in: a worktree, or null for the project's. */
  cwd: string | null
  /** `pwsh`, `bash`…; null until the shell is known. */
  shell: string | null
  status: "starting" | "running" | "exited"
}

type TermStore = {
  tabs: TermTab[]
  /** The tab on screen, per project. */
  active: Record<string, string>
  /** The shell last seen on each system (`locationKey`), so the button shows it before connecting. */
  shells: Record<string, string>
  /** A new terminal in the project (in `cwd`, a worktree), shown. */
  add(projectId: string, cwd: string | null): string
  select(projectId: string, id: string): void
  /** Ends the shell and removes the tab. */
  close(id: string): void
  /** Shows or hides the panel; showing it with no terminal in the project starts one. */
  toggle(): void
}

const SHELLS_KEY = "oah.terminal.shells"
let counter = 0

export const useTerminals = create<TermStore>((set, get) => ({
  tabs: [],
  active: {},
  shells: loadShells(),

  add(projectId, cwd) {
    const id = `tab${++counter}`
    const location = locationOf(projectId)
    const shell = location ? (get().shells[locationKey(location)] ?? null) : null
    set((s) => ({ tabs: [...s.tabs, { id, projectId, cwd, shell, status: "starting" }], active: { ...s.active, [projectId]: id } }))
    useApp.getState().setLayout({ terminalOpen: true })
    return id
  },

  select(projectId, id) {
    set((s) => ({ active: { ...s.active, [projectId]: id } }))
    requestAnimationFrame(() => focusTerminal(id))
  },

  close(id) {
    const tab = get().tabs.find((t) => t.id === id)
    if (!tab) return
    dispose(id)
    const tabs = get().tabs.filter((t) => t.id !== id)
    const left = tabs.filter((t) => t.projectId === tab.projectId)
    const active = { ...get().active }
    if (active[tab.projectId] === id) {
      if (left.length) active[tab.projectId] = left[left.length - 1].id
      else delete active[tab.projectId]
    }
    set({ tabs, active })
    const app = useApp.getState()
    // The project's last terminal closing puts the panel away.
    if (!left.length && app.projectId === tab.projectId) app.setLayout({ terminalOpen: false })
    else if (active[tab.projectId]) requestAnimationFrame(() => focusTerminal(active[tab.projectId]))
  },

  toggle() {
    const app = useApp.getState()
    const scope = currentScope()
    if (!scope) return
    const mine = get().tabs.some((t) => t.projectId === scope.projectId)
    if (app.layout.terminalOpen && mine && app.view === "chat") {
      app.setLayout({ terminalOpen: false })
      return
    }
    if (app.view !== "chat") app.closeSettings()
    if (!mine) get().add(scope.projectId, scope.cwd)
    else app.setLayout({ terminalOpen: true })
    const id = get().active[scope.projectId]
    if (id) requestAnimationFrame(() => focusTerminal(id))
  },
}))

function patch(id: string, p: Partial<TermTab>) {
  useTerminals.setState((s) => ({ tabs: s.tabs.map((t) => (t.id === id ? { ...t, ...p } : t)) }))
}

function locationOf(projectId: string): Location | null {
  return useApp.getState().settings?.projects.find((p) => p.id === projectId)?.location ?? null
}

/** Remembers the shell a tab runs, for the tab and for its system. */
function learnShell(id: string, shell: string) {
  patch(id, { shell })
  const tab = useTerminals.getState().tabs.find((t) => t.id === id)
  const location = tab && locationOf(tab.projectId)
  if (!location) return
  const shells = { ...useTerminals.getState().shells, [locationKey(location)]: shell }
  useTerminals.setState({ shells })
  try {
    localStorage.setItem(SHELLS_KEY, JSON.stringify(shells))
  } catch {
    /* ignore */
  }
}

function loadShells(): Record<string, string> {
  try {
    return JSON.parse(localStorage.getItem(SHELLS_KEY) ?? "{}")
  } catch {
    return {}
  }
}

/** The shell a new terminal on this computer runs, asked once. */
let localShell: Promise<void> | null = null
export function learnLocalShell(projectId: string) {
  localShell ??= ipc
    .shell(projectId)
    .then((shell) => {
      if (!shell) return
      const shells = { ...useTerminals.getState().shells, local: shell }
      useTerminals.setState({ shells })
    })
    .catch(() => {
      localShell = null
    })
}

// ---- The terminals -----------------------------------------------------------------------

type Instance = {
  term: Terminal
  fit: FitAddon
  /** The element xterm draws in; moved between hosts as React remounts them. */
  el: HTMLDivElement
  /** The shell's id on the Rust side, once started. */
  session: string | null
  /** Input from before the shell's id is known: answers to Windows' pseudo console asking
   *  where the cursor is, which it waits for. */
  queued: string[]
  /** Bumped per start, so a late reply from an earlier shell is ignored. */
  run: number
  disposed: boolean
}

const instances = new Map<string, Instance>()

const FONT_SIZE = 13

function fontFamily() {
  const mono = getComputedStyle(document.documentElement).getPropertyValue("--oah-mono").trim()
  return `${mono ? `${mono}, ` : ""}"JetBrains Mono Variable", "JetBrains Mono", ui-monospace, Consolas, monospace`
}

function theme() {
  const css = getComputedStyle(document.documentElement)
  const dark = document.documentElement.classList.contains("dark")
  return terminalTheme(dark, css.getPropertyValue("--sidebar").trim() || (dark ? "#0e0e10" : "#eef0f3"), css.getPropertyValue("--foreground").trim() || (dark ? "#ececef" : "#16181d"))
}

// Theme and font changes (Settings → Appearance) reach open terminals.
let watching = false
function watchAppearance() {
  if (watching) return
  watching = true
  new MutationObserver(() => {
    const t = theme()
    const f = fontFamily()
    for (const i of instances.values()) {
      i.term.options.theme = t
      if (i.term.options.fontFamily !== f) {
        i.term.options.fontFamily = f
        fitTerminal(i)
      }
    }
  }).observe(document.documentElement, { attributes: true, attributeFilter: ["class", "style"] })
}

/** The element each tab's terminal belongs in now (React may remount it). */
const hosts = new Map<string, HTMLElement>()
const creating = new Set<string>()

/** Puts the tab's terminal in `host`, creating it (and starting its shell) the first time. */
export function attachTerminal(id: string, host: HTMLElement) {
  hosts.set(id, host)
  const inst = instances.get(id)
  if (inst) {
    if (inst.el.parentElement !== host) host.appendChild(inst.el)
    fitTerminal(inst)
  } else if (!creating.has(id)) {
    creating.add(id)
    void build(id).finally(() => creating.delete(id))
  }
}

/** The host is going away (its terminal stays, to be attached again). */
export function detachTerminal(id: string, host: HTMLElement) {
  if (hosts.get(id) === host) hosts.delete(id)
}

async function build(id: string) {
  const [{ Terminal }, { FitAddon }, { WebLinksAddon }] = await Promise.all([import("@xterm/xterm"), import("@xterm/addon-fit"), import("@xterm/addon-web-links")])
  // Measure the mono font once it's loaded, or the cells come out the wrong width.
  await document.fonts.load(`${FONT_SIZE}px "JetBrains Mono Variable"`).catch(() => undefined)
  const host = hosts.get(id)
  if (!host || !useTerminals.getState().tabs.some((t) => t.id === id)) return
  const el = document.createElement("div")
  el.className = "h-full w-full"
  host.appendChild(el)
  const windows = useApp.getState().info?.platform === "windows"
  const term = new Terminal({
    fontFamily: fontFamily(),
    fontSize: FONT_SIZE,
    lineHeight: 1.2,
    cursorBlink: true,
    scrollback: 10_000,
    theme: theme(),
    // Every shell runs in a Windows pseudo console there (ssh.exe and wsl.exe too).
    windowsPty: windows ? { backend: "conpty" } : undefined,
  })
  const fit = new FitAddon()
  term.loadAddon(fit)
  term.loadAddon(
    new WebLinksAddon((_e, uri) => {
      import("@tauri-apps/plugin-opener").then((o) => o.openUrl(uri)).catch(() => undefined)
    }),
  )
  const inst: Instance = { term, fit, el, session: null, queued: [], run: 0, disposed: false }
  instances.set(id, inst)
  watchAppearance()

  term.parser.registerOscHandler(SHELL_OSC, (data) => {
    const shell = parseShellOsc(data)
    if (shell) learnShell(id, shell)
    return true
  })
  term.attachCustomKeyEventHandler((e) => keyForApp(inst, e))
  term.onData((data) => {
    const tab = useTerminals.getState().tabs.find((t) => t.id === id)
    if (tab?.status === "exited") {
      if (data === "\r") void start(id, inst)
      return
    }
    if (inst.session) void ipc.write(inst.session, data).catch(() => undefined)
    else inst.queued.push(data)
  })
  term.onResize(({ cols, rows }) => {
    if (inst.session) void ipc.resize(inst.session, cols, rows).catch(() => undefined)
  })
  term.open(el)
  fitTerminal(inst)
  term.focus()
  await start(id, inst)
}

/** Starts (or restarts) the tab's shell. */
async function start(id: string, inst: Instance) {
  const tab = useTerminals.getState().tabs.find((t) => t.id === id)
  if (!tab) return
  const run = ++inst.run
  inst.session = null
  inst.queued = []
  patch(id, { status: "starting" })
  const exited = (code: number | null) => {
    if (inst.run !== run || inst.disposed) return
    inst.session = null
    // `exit` in the shell closes the tab; a failure or a dropped connection stays to read.
    if (code === 0) return useTerminals.getState().close(id)
    patch(id, { status: "exited" })
    const why = code === null ? "The shell ended" : `The shell exited with code ${code}`
    inst.term.write(`\r\n\x1b[2m${why}. Press Enter to start it again.\x1b[0m\r\n`)
  }
  try {
    const opened = await ipc.open(tab.projectId, tab.cwd, inst.term.cols, inst.term.rows, (e) => {
      if (inst.run !== run || inst.disposed) return
      if (e.kind === "data") inst.term.write(e.data)
      else exited(e.code)
    })
    if (inst.disposed || inst.run !== run) return void ipc.close(opened.id).catch(() => undefined)
    // An exit can beat the reply; then there's no shell to talk to.
    if (useTerminals.getState().tabs.find((t) => t.id === id)?.status === "exited") return
    inst.session = opened.id
    if (inst.queued.length) void ipc.write(opened.id, inst.queued.splice(0).join("")).catch(() => undefined)
    patch(id, { status: "running" })
    if (opened.shell) learnShell(id, opened.shell)
  } catch (e) {
    if (inst.run !== run || inst.disposed) return
    patch(id, { status: "exited" })
    inst.term.write(`\x1b[31m${String(e)}\x1b[0m\r\n\x1b[2mPress Enter to try again.\x1b[0m\r\n`)
  }
}

/** Keys the app keeps from the shell: copy, paste, and the terminal toggle. */
function keyForApp(inst: Instance, e: KeyboardEvent): boolean {
  const mod = e.ctrlKey || e.metaKey
  if (!mod || e.altKey) return true
  if (e.code === "Backquote" || e.key === ",") return false
  const k = e.key.toLowerCase()
  // Ctrl+Shift+C always copies; Ctrl+C copies when there's a selection (else it's ^C).
  if (k === "c" && (e.shiftKey || inst.term.hasSelection())) {
    if (e.type === "keydown" && inst.term.hasSelection()) {
      copyText(inst.term.getSelection())
      inst.term.clearSelection()
    }
    e.preventDefault()
    return false
  }
  // Ctrl+V and Ctrl+Shift+V: the browser's own paste, which xterm turns into input.
  if (k === "v") return false
  return true
}

function fitTerminal(inst: Instance) {
  if (!inst.el.isConnected || inst.el.offsetWidth === 0 || inst.el.offsetHeight === 0) return
  try {
    inst.fit.fit()
  } catch {
    /* not measurable yet */
  }
}

/** Fits the tab's terminal to its host (after the panel is shown or resized). */
export function fitTab(id: string) {
  const inst = instances.get(id)
  if (inst) fitTerminal(inst)
}

export function focusTerminal(id: string) {
  instances.get(id)?.term.focus()
}

function dispose(id: string) {
  const inst = instances.get(id)
  hosts.delete(id)
  if (!inst) return
  inst.disposed = true
  instances.delete(id)
  if (inst.session) void ipc.close(inst.session).catch(() => undefined)
  inst.term.dispose()
  inst.el.remove()
}
