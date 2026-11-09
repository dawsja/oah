// App-wide state: settings (saved through Rust), what's on screen, and the layout.

import { create } from "zustand"
import { toast } from "sonner"
import { ipc } from "@/lib/ipc"
import { BASE_UI_SIZE, type AppInfo, type Appearance, type Location, type Settings } from "@/lib/types"

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
  /** Adds a project and opens it; false (after saying why) when it couldn't be added. */
  addProject(location: Location, path: string): Promise<boolean>
  removeProject(id: string): Promise<void>
  openSettings(section?: SettingsSection): void
  closeSettings(): void
  selectProject(id: string | null): void
  setLayout(patch: Partial<Layout>): void
}

const LAYOUT_KEY = "oah.layout"
export const defaultLayout: Readonly<Layout> = { sidebarWidth: 272, rightWidth: 340, rightOpen: true, rightTab: "scm", terminalOpen: false, terminalHeight: 280 }

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
      lastSaved = settings
      set({ info, settings, projectId, error: null })
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
      set({ settings, projectId: project.id, chatId: null, draftCwd: null, view: "chat", error: null })
      import("./git").then((m) => m.useGit.getState().open(null))
      localStorage.setItem("oah.project", project.id)
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
      if (get().projectId === id) {
        import("./git").then((m) => m.useGit.getState().open(null))
        set({ settings, projectId: settings.projects[0]?.id ?? null, chatId: null, draftCwd: null })
      } else {
        set({ settings })
      }
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

export function useProject() {
  return useApp((s) => s.settings?.projects.find((p) => p.id === s.projectId) ?? null)
}
