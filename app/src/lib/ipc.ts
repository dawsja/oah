// The commands the Rust side exposes (app/src-tauri/src/lib.rs). Outside Tauri (plain `vite` in a
// browser) a small in-memory stand-in answers instead, so the UI can be developed and looked at
// without the shell.

import type { ChatEvent, ChatMeta, Decision, ModelOption } from "./chat"
import type { Account, AgentKind, AppInfo, HostCheck, Limits, Location, PermissionMode, Probe, Project, Settings } from "./types"

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
  sshCheck: (host: string) => call<HostCheck>("ssh_check", { host }),
  sshListDir: (host: string, path: string) => call<{ path: string; dirs: string[] }>("ssh_list_dir", { host, path }),
  providerProbe: (kind: AgentKind, binary: string) => call<Probe>("provider_probe", { kind, binary }),
  providerAccount: (kind: AgentKind, host: string | null = null) => call<Account>("provider_account", { kind, host }),
  providerLimits: (kind: AgentKind) => call<Limits>("provider_limits", { kind }),
  providerLogin: (kind: AgentKind) => call<void>("provider_login", { kind }),

  chatList: () => call<ChatMeta[]>("chat_list"),
  chatRunning: () => call<string[]>("chat_running"),
  chatCreate: (projectId: string, agent: AgentKind | null, acpAgent: string | null, model: string | null, cwd: string | null) =>
    call<ChatMeta>("chat_create", { projectId, agent, acpAgent, model, cwd }),
  providerModels: (kind: AgentKind, projectId: string | null) => call<ModelOption[]>("provider_models", { kind, projectId }),
  chatEvents: (chatId: string) => call<ChatEvent[]>("chat_events", { chatId }),
  chatSend: (chatId: string, prompt: { id: string; text: string; images: { mime: string; data: string }[] }, permission: PermissionMode | null) =>
    call<void>("chat_send", { chatId, prompt, permission }),
  chatInterrupt: (chatId: string) => call<void>("chat_interrupt", { chatId }),
  chatAnswer: (chatId: string, requestId: string, decision: Decision) => call<void>("chat_answer", { chatId, requestId, decision }),
  chatSetPermission: (chatId: string, mode: PermissionMode) => call<void>("chat_set_permission", { chatId, mode }),
  chatAnswerQuestion: (chatId: string, requestId: string, answers: Record<string, string[]>) => call<void>("chat_answer_question", { chatId, requestId, answers }),
  chatSetModel: (chatId: string, model: string) => call<ChatMeta>("chat_set_model", { chatId, model }),
  chatClose: (chatId: string) => call<void>("chat_close", { chatId }),
  chatDelete: (chatId: string) => call<void>("chat_delete", { chatId }),
  chatRename: (chatId: string, title: string) => call<ChatMeta>("chat_rename", { chatId, title }),
}

/** Calls `fn` for every chat event the app emits; returns the unsubscribe. */
export async function onChatEvent(fn: (chatId: string, event: ChatEvent) => void): Promise<() => void> {
  if (!inTauri) return () => {}
  const { listen } = await import("@tauri-apps/api/event")
  return listen<{ chatId: string; event: ChatEvent }>("chat-event", (e) => fn(e.payload.chatId, e.payload.event))
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
    providers: { claude: provider(), codex: provider(), opencode: provider(), acp: [], defaultAgent: "claude" },
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
      return { version: "0.2.0-web", platform: "web", dataDir: "~/.config/oah/app", home: "~", bundle: null } satisfies AppInfo
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
    case "provider_account":
      return { signedIn: true, label: "you@example.com", plan: a.kind === "claude" ? "max" : null, error: null }
    case "provider_limits":
      return { windows: [], error: "Limits load in the app" }
    case "chat_list":
    case "chat_running":
    case "chat_events":
      return []
    default:
      throw new Error(`${cmd} isn't available outside the app`)
  }
}
