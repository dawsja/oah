// App-wide state: settings (saved through Rust), what's on screen, and the layout.

import { create } from "zustand"
import { ipc } from "@/lib/ipc"
import type { AppInfo, Appearance, Location, Settings } from "@/lib/types"

export type SettingsSection = "providers" | "voice" | "appearance" | "usage" | "ssh" | "projects"
export type RightTab = "files" | "scm" | "history" | "pr"

type Layout = {
  sidebarWidth: number
  rightWidth: number
  rightOpen: boolean
  rightTab: RightTab
}

type AppStore = {
  info: AppInfo | null
  settings: Settings | null
  error: string | null
  view: "chat" | "settings"
  section: SettingsSection
  projectId: string | null
  chatId: string | null
  /** The folder a new (unsent) chat will work in: a worktree, or null for the project. */
  draftCwd: string | null
  layout: Layout

  load(): Promise<void>
  /** Applies `edit` to a copy of the settings and saves it. */
  update(edit: (s: Settings) => void): Promise<void>
  addProject(location: Location, path: string): Promise<void>
  removeProject(id: string): Promise<void>
  openSettings(section?: SettingsSection): void
  closeSettings(): void
  selectProject(id: string | null): void
  setLayout(patch: Partial<Layout>): void
}

const LAYOUT_KEY = "oah.layout"
const defaultLayout: Layout = { sidebarWidth: 272, rightWidth: 340, rightOpen: true, rightTab: "scm" }

function loadLayout(): Layout {
  try {
    return { ...defaultLayout, ...JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "{}") }
  } catch {
    return defaultLayout
  }
}

export const useApp = create<AppStore>((set, get) => ({
  info: null,
  settings: null,
  error: null,
  view: "chat",
  section: "providers",
  projectId: null,
  chatId: null,
  draftCwd: null,
  layout: loadLayout(),

  async load() {
    try {
      const [info, settings] = await Promise.all([ipc.appInfo(), ipc.settingsGet()])
      const last = localStorage.getItem("oah.project")
      const projectId = settings.projects.find((p) => p.id === last)?.id ?? settings.projects[0]?.id ?? null
      set({ info, settings, projectId })
      applyAppearance(settings.appearance)
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
    applyAppearance(next.appearance)
    try {
      set({ settings: await ipc.settingsSet(next) })
    } catch (e) {
      set({ settings: cur, error: String(e) })
      applyAppearance(cur.appearance)
    }
  },

  async addProject(location, path) {
    try {
      const [settings, project] = await ipc.projectAdd(location, path)
      set({ settings, projectId: project.id, error: null })
      localStorage.setItem("oah.project", project.id)
    } catch (e) {
      set({ error: String(e) })
    }
  },

  async removeProject(id) {
    try {
      const settings = await ipc.projectRemove(id)
      const projectId = get().projectId === id ? (settings.projects[0]?.id ?? null) : get().projectId
      set({ settings, projectId })
    } catch (e) {
      set({ error: String(e) })
    }
  },

  openSettings(section) {
    set({ view: "settings", section: section ?? get().section })
  },
  closeSettings() {
    set({ view: "chat" })
  },
  selectProject(id) {
    if (id !== get().projectId) import("./git").then((m) => m.useGit.getState().open(null))
    set({ projectId: id, view: "chat", draftCwd: id === get().projectId ? get().draftCwd : null })
    if (id) localStorage.setItem("oah.project", id)
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
  root.dataset.density = a.density
  root.style.setProperty("--oah-ui-size", `${a.uiFontSize}px`)
  root.style.setProperty("--oah-chat-size", `${a.chatFontSize}px`)
  root.style.setProperty("--oah-sans", `"${a.sansFont}"`)
  root.style.setProperty("--oah-mono", `"${a.monoFont}"`)
  try {
    localStorage.setItem("oah.theme", a.theme)
  } catch {
    /* ignore */
  }
  if ("__TAURI_INTERNALS__" in window) {
    import("@tauri-apps/api/window").then(({ getCurrentWindow }) =>
      getCurrentWindow()
        .setTheme(a.theme === "system" ? null : a.theme)
        .catch(() => {}),
    )
  }
}

export function useProject() {
  return useApp((s) => s.settings?.projects.find((p) => p.id === s.projectId) ?? null)
}
