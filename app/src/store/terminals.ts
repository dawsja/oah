// The terminal panel's tabs and the terminals behind them. Each tab is an xterm.js terminal
// attached to a shell where the project is (this computer, a WSL distro, an SSH host). The
// terminals live here, outside React: hiding the panel or switching projects only hides them,
// so a shell (and an ssh or WSL session) stays connected until its tab is closed.

import { create } from "zustand"
import { terminal as ipc } from "@/lib/ipc"
import { locationKey, parseShellOsc, selectionBlock, SHELL_OSC } from "@/lib/terminal"
import type { Location } from "@/lib/types"
import { disposeXterm, fitXterm, loadXterm, makeXterm, moveXterm, type Xterm } from "@/lib/xterm"
import { paneOf, useApp } from "./app"
import { currentScope } from "./git"
import { insertIntoChat } from "./insert"

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
  /** Tabs with text selected in them. */
  selected: Record<string, boolean>
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
  selected: {},

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
    if (!left.length && paneOf(app).projectId === tab.projectId) app.setLayout({ terminalOpen: false })
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

type Instance = Xterm & {
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

/** The element each tab's terminal belongs in now (React may remount it). */
const hosts = new Map<string, HTMLElement>()
const creating = new Set<string>()

/** Puts the tab's terminal in `host`, creating it (and starting its shell) the first time. */
export function attachTerminal(id: string, host: HTMLElement) {
  hosts.set(id, host)
  const inst = instances.get(id)
  if (inst) {
    moveXterm(inst, host)
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
  const modules = await loadXterm()
  const host = hosts.get(id)
  if (!host || !useTerminals.getState().tabs.some((t) => t.id === id)) return
  const inst: Instance = Object.assign(makeXterm(modules, host, useApp.getState().info?.platform === "windows"), { session: null, queued: [], run: 0, disposed: false })
  const term = inst.term
  instances.set(id, inst)

  term.parser.registerOscHandler(SHELL_OSC, (data) => {
    const shell = parseShellOsc(data)
    if (shell) learnShell(id, shell)
    return true
  })
  term.onSelectionChange(() => {
    const has = term.hasSelection() && term.getSelection().trim() !== ""
    if (!!useTerminals.getState().selected[id] !== has) useTerminals.setState((s) => ({ selected: { ...s.selected, [id]: has } }))
  })
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

/** Fits the tab's terminal to its host (after the panel is shown or resized). */
export function fitTab(id: string) {
  const inst = instances.get(id)
  if (inst) fitXterm(inst)
}

export function focusTerminal(id: string) {
  instances.get(id)?.term.focus()
}

/** The tab's selected text as a code block for the chat's composer: "the test failed, here is
 *  the output". The composer gets it at its cursor, ready for a question under it. */
export function askAgentAbout(id: string) {
  const inst = instances.get(id)
  const text = inst ? selectionBlock(inst.term.getSelection()) : null
  if (!inst || !text) return
  inst.term.clearSelection()
  insertIntoChat(text)
}

function dispose(id: string) {
  const inst = instances.get(id)
  hosts.delete(id)
  if (!inst) return
  inst.disposed = true
  instances.delete(id)
  if (inst.session) void ipc.close(inst.session).catch(() => undefined)
  disposeXterm(inst)
}
