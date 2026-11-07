// The commands the Rust side exposes (app/src-tauri/src/lib.rs). Outside Tauri (plain `vite` in a
// browser) a small in-memory stand-in answers instead, so the UI can be developed and looked at
// without the shell.

import type { AgentKind, AppInfo, Location, Probe, Project, Settings } from "./types"

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (inTauri) {
    const { invoke } = await import("@tauri-apps/api/core")
    return invoke<T>(cmd, args)
  }
  return web(cmd, args ?? {}) as Promise<T>
}

export const ipc = {
  appInfo: () => call<AppInfo>("app_info"),
  settingsGet: () => call<Settings>("settings_get"),
  settingsSet: (settings: Settings) => call<Settings>("settings_set", { settings }),
  projectAdd: (location: Location, path: string) => call<[Settings, Project]>("project_add", { location, path }),
  projectRemove: (id: string) => call<Settings>("project_remove", { id }),
  sshConfigHosts: () => call<string[]>("ssh_config_hosts"),
  providerProbe: (kind: AgentKind, binary: string) => call<Probe>("provider_probe", { kind, binary }),
}

/** Picks a folder on this computer; null when cancelled. */
export async function pickFolder(title: string): Promise<string | null> {
  if (!inTauri) return window.prompt(title, "C:\\Users\\you\\code\\project")
  const { open } = await import("@tauri-apps/plugin-dialog")
  const r = await open({ directory: true, multiple: false, title })
  return typeof r === "string" ? r : null
}

// ---- Browser stand-in ------------------------------------------------------------------------

export function defaultSettings(): Settings {
  const provider = () => ({ enabled: true, binary: "", defaultModel: "", permissionMode: "ask" as const, env: [] })
  return {
    providers: { claude: provider(), codex: provider(), defaultAgent: "claude" },
    voice: { enabled: false, microphone: "", holdKey: "Ctrl+Space", silenceStopSecs: 0 },
    appearance: {
      theme: "system",
      uiFontSize: 13,
      chatFontSize: 14,
      sansFont: "Inter Variable",
      monoFont: "JetBrains Mono Variable",
      density: "comfortable",
    },
    sshHosts: [],
    projects: [],
  }
}

let mem: Settings | null = null
function memSettings(): Settings {
  if (mem) return mem
  try {
    const saved = localStorage.getItem("oah.web.settings")
    mem = saved ? { ...defaultSettings(), ...JSON.parse(saved) } : defaultSettings()
  } catch {
    mem = defaultSettings()
  }
  return mem!
}
function memSave(s: Settings) {
  mem = s
  try {
    localStorage.setItem("oah.web.settings", JSON.stringify(s))
  } catch {
    /* private window */
  }
}

async function web(cmd: string, a: Record<string, unknown>): Promise<unknown> {
  switch (cmd) {
    case "app_info":
      return { version: "0.2.0-web", platform: "web", dataDir: "~/.config/oah/app", home: "~" } satisfies AppInfo
    case "settings_get":
      return structuredClone(memSettings())
    case "settings_set":
      memSave(a.settings as Settings)
      return structuredClone(a.settings)
    case "project_add": {
      const s = structuredClone(memSettings())
      const path = String(a.path)
      const name = path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || path
      const p: Project = {
        id: Math.random().toString(16).slice(2, 14),
        name,
        location: a.location as Location,
        path,
        defaultAgent: null,
        defaultModel: "",
        git: { baseBranch: "", worktreeDir: "", deleteBranchOnRemove: false },
      }
      s.projects.push(p)
      memSave(s)
      return [s, p]
    }
    case "project_remove": {
      const s = structuredClone(memSettings())
      s.projects = s.projects.filter((p) => p.id !== a.id)
      memSave(s)
      return s
    }
    case "ssh_config_hosts":
      return ["devbox", "gpu-box"]
    case "provider_probe":
      return a.kind === "claude"
        ? { found: true, path: "C:/Users/you/.local/bin/claude.exe", version: "2.1.319", error: null }
        : { found: true, path: "C:/Users/you/AppData/Roaming/npm/codex.cmd", version: "0.166.0", error: null }
    default:
      throw new Error(`${cmd} isn't available outside the app`)
  }
}
