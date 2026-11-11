// App-wide state: settings (saved through Rust), what's on screen, and the layout.

import { createContext, useContext } from "react"
import { create } from "zustand"
import { toast } from "sonner"
import { ipc } from "@/lib/ipc"
import { closePane, draftSwaps, MAX_PANES, paneOf, patchPane, splitPanes, type Pane, type Panes, type PaneTarget } from "@/lib/panes"
import { BASE_UI_SIZE, type AppInfo, type Appearance, type Location, type Settings } from "@/lib/types"
import { drafts } from "./drafts"

export { paneOf, type Pane, type PaneTarget } from "@/lib/panes"

export type SettingsSection = "sessions" | "providers" | "voice" | "appearance" | "usage" | "ssh" | "projects" | "mcp" | "phone"
export type RightTab = "files" | "scm" | "history" | "pr"

type Layout = {
  sidebarWidth: number
  rightWidth: number
  rightOpen: boolean
  rightTab: RightTab
  /** The terminal panel under the chat: shown, and its height. */
  terminalOpen: boolean
  terminalHeight: number
  /** With two panes, the left one's share of the width. */
  split: number
}

type AppStore = {
  info: AppInfo | null
  settings: Settings | null
  error: string | null
  view: "chat" | "settings"
  section: SettingsSection
  /** What the main area shows: one pane, or two side by side (the left one first). */
  panes: Pane[]
  /** The pane the sidebar, the side panel, the terminal and the title bar follow. */
  activePane: string
  layout: Layout

  load(): Promise<void>
  /** Applies `edit` to a copy of the settings and saves it. */
  update(edit: (s: Settings) => void): Promise<void>
  /** Adds a project and opens it; false (after saying why) when it couldn't be added. */
  addProject(location: Location, path: string): Promise<boolean>
  removeProject(id: string): Promise<void>
  openSettings(section?: SettingsSection): void
  closeSettings(): void
  /** Changes what a pane shows (the active one without an id). */
  setPane(target: PaneTarget, pane?: string | null): void
  focusPane(id: string): void
  /** Shows `target` beside the active pane and moves there: in a second pane, or in the other
   *  one when there are two. Without a target, a new chat in the same project. */
  splitPane(target?: PaneTarget): void
  /** Closes a pane (the active one without an id); the other fills the space. */
  closePane(id?: string | null): void
  setLayout(patch: Partial<Layout>): void
}

const LAYOUT_KEY = "oah.layout"
export const defaultLayout: Readonly<Layout> = { sidebarWidth: 272, rightWidth: 340, rightOpen: true, rightTab: "scm", terminalOpen: false, terminalHeight: 280, split: 0.5 }

function loadLayout(): Layout {
  try {
    return { ...defaultLayout, ...JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "{}") }
  } catch {
    return defaultLayout
  }
}

/** The settings as last saved, to go back to when a save fails. */
let lastSaved: Settings | null = null
let saveSeq = 0
let saving: Promise<void> = Promise.resolve()

let paneCount = 1
const emptyPane = (id: string, projectId: string | null = null): Pane => ({ id, projectId, chatId: null, draftCwd: null, viewer: null })

/** Applies a change to the panes. A pane whose new-chat draft is kept elsewhere from now on
 *  (the other pane closed, or came to the same new chat) keeps what was typed in it. */
function commit(next: Panes, more: Partial<AppStore> = {}) {
  for (const [a, b] of draftSwaps(useApp.getState().panes, next.panes)) drafts.swap(a, b)
  useApp.setState({ panes: next.panes, activePane: next.activePane, ...more })
  // The next start opens on the project last worked in.
  const project = paneOf(next).projectId
  try {
    if (project) localStorage.setItem("oah.project", project)
  } catch {
    /* ignore */
  }
}

export const useApp = create<AppStore>((set, get) => ({
  info: null,
  settings: null,
  error: null,
  view: "chat",
  section: "providers",
  panes: [emptyPane("p1")],
  activePane: "p1",
  layout: loadLayout(),

  async load() {
    try {
      const [info, settings] = await Promise.all([ipc.appInfo(), ipc.settingsGet()])
      const last = localStorage.getItem("oah.project")
      const projectId = settings.projects.find((p) => p.id === last)?.id ?? settings.projects[0]?.id ?? null
      lastSaved = settings
      // Each pane keeps what it shows across a reload of the settings; a project that's gone
      // gives way to the last one worked in.
      const known = (id: string | null) => settings.projects.some((p) => p.id === id)
      set((s) => ({ info, settings, error: null, panes: s.panes.map((p) => (known(p.projectId) ? p : emptyPane(p.id, projectId))) }))
      import("@/lib/updates").then((u) => u.updater.start(info.bundle))
      applyAppearance(settings.appearance)
      if (info.settingsProblem) toast.error("Settings were reset", { description: info.settingsProblem, duration: Infinity })
    } catch (e) {
      set({ error: String(e) })
    }
  },

  async update(edit) {
    const cur = get().settings
    if (!cur) return
    const next = structuredClone(cur)
    edit(next)
    set({ settings: next })
    if (JSON.stringify(next.appearance) !== JSON.stringify(cur.appearance)) applyAppearance(next.appearance)
    // Saves go one at a time, each with the settings as they are by then, and only the newest
    // answer is applied: quick edits can't land out of order or undo each other.
    const seq = ++saveSeq
    saving = saving.then(async () => {
      if (seq !== saveSeq) return
      const want = get().settings
      if (!want) return
      try {
        const saved = await ipc.settingsSet(want)
        lastSaved = saved
        if (seq === saveSeq) set({ settings: saved })
      } catch (e) {
        if (seq !== saveSeq) return
        const before = get().settings
        set({ settings: lastSaved })
        if (lastSaved && JSON.stringify(before?.appearance) !== JSON.stringify(lastSaved.appearance)) applyAppearance(lastSaved.appearance)
        toast.error("Couldn't save settings", { description: String(e) })
      }
    })
    await saving
  },

  async addProject(location, path) {
    try {
      const [settings, project] = await ipc.projectAdd(location, path)
      lastSaved = settings
      // A new chat starts in the new project, not in a worktree of the one before.
      commit(patchPane(get(), { projectId: project.id, chatId: null, draftCwd: null, viewer: null }), { settings, view: "chat", error: null })
      return true
    } catch (e) {
      toast.error("Couldn't add the project", { description: String(e) })
      return false
    }
  },

  async removeProject(id) {
    const project = get().settings?.projects.find((p) => p.id === id)
    if (!project) return
    const { useChats } = await import("./chats")
    const chats = useChats.getState().metas.filter((m) => m.projectId === id)
    const { confirmAction } = await import("@/components/Prompt")
    const ok = await confirmAction({
      title: `Remove ${project.name} from oah?`,
      description: `The folder isn't touched.${chats.length ? ` Its ${chats.length === 1 ? "chat is" : `${chats.length} chats are`} kept and come back if you add the folder again; any agent running in them stops.` : ""}`,
      confirm: "Remove",
      danger: true,
    })
    if (!ok) return
    try {
      const settings = await ipc.projectRemove(id)
      lastSaved = settings
      // A pane that showed it shows the first project left.
      const first = settings.projects[0]?.id ?? null
      commit({ panes: get().panes.map((p) => (p.projectId === id ? emptyPane(p.id, first) : p)), activePane: get().activePane }, { settings })
    } catch (e) {
      toast.error("Couldn't remove the project", { description: String(e) })
    }
  },

  openSettings(section) {
    set({ view: "settings", section: section ?? get().section })
  },
  closeSettings() {
    set({ view: "chat" })
  },
  setPane(target, pane) {
    commit(patchPane(get(), target, pane))
  },
  focusPane(id) {
    if (id !== get().activePane && get().panes.some((p) => p.id === id)) commit({ panes: get().panes, activePane: id })
  },
  splitPane(target) {
    commit(splitPanes(get(), `p${++paneCount}`, target), { view: "chat" })
  },
  closePane(id) {
    commit(closePane(get(), id))
  },
  setLayout(patch) {
    const layout = { ...get().layout, ...patch }
    set({ layout })
    try {
      localStorage.setItem(LAYOUT_KEY, JSON.stringify(layout))
    } catch {
      /* ignore */
    }
  },
}))

export { MAX_PANES }

/** The pane a component is in; none outside the panes (the sidebar, the side panel…), where
 *  the active pane is meant. */
export const PaneContext = createContext<string | null>(null)

/** The pane this component is in, or the active one outside the panes. */
export function usePane(): Pane {
  const id = useContext(PaneContext)
  return useApp((s) => paneOf(s, id))
}

/** Whether this component's pane is the active one (always, outside the panes). */
export function usePaneActive(): boolean {
  const id = useContext(PaneContext)
  return useApp((s) => !id || paneOf(s, id).id === s.activePane)
}

// ---- Appearance ------------------------------------------------------------------------------

const media = typeof window !== "undefined" ? window.matchMedia("(prefers-color-scheme: dark)") : null
let current: Appearance | null = null
media?.addEventListener("change", () => current && applyAppearance(current))

/** Puts the theme, sizes and fonts on <html>. */
export function applyAppearance(a: Appearance) {
  current = a
  const root = document.documentElement
  const dark = a.theme === "dark" || (a.theme === "system" && (media?.matches ?? true))
  root.classList.toggle("dark", dark)
  root.style.setProperty("--oah-chat-size", `${a.chatFontSize}px`)
  root.style.setProperty("--oah-sans", `"${a.sansFont}"`)
  root.style.setProperty("--oah-mono", `"${a.monoFont}"`)
  try {
    localStorage.setItem("oah.theme", a.theme)
  } catch {
    /* ignore */
  }
  import("@tauri-apps/api/window").then(({ getCurrentWindow }) =>
    getCurrentWindow()
      .setTheme(a.theme === "system" ? null : a.theme)
      .catch(() => {}),
  )
  // Interface size zooms the whole window (text, icons, spacing), like the browser's zoom.
  import("@tauri-apps/api/webview").then(({ getCurrentWebview }) =>
    getCurrentWebview()
      .setZoom(a.uiFontSize / BASE_UI_SIZE)
      .catch(() => {}),
  )
}

/** The project of this component's pane (the active pane's, outside the panes). */
export function useProject() {
  const id = useContext(PaneContext)
  return useApp((s) => s.settings?.projects.find((p) => p.id === paneOf(s, id).projectId) ?? null)
}
