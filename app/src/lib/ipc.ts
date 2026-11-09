// The commands the Rust side exposes (app/src-tauri/src/lib.rs).

import type { ChatEvent, ChatMeta, Decision, ModelOption, SlashCommand, Snapshot } from "./chat"
import type { Account, AgentKind, AppInfo, Distro, HostCheck, Limits, Location, McpStatus, PermissionMode, Probe, Project, Settings } from "./types"

/** An agent's models and slash commands, for a chat that hasn't started. */
export type Catalog = { models: ModelOption[]; commands: SlashCommand[] }

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core")
  return invoke<T>(cmd, args)
}

export const ipc = {
  appInfo: () => call<AppInfo>("app_info"),
  settingsGet: () => call<Settings>("settings_get"),
  settingsSet: (settings: Settings) => call<Settings>("settings_set", { settings }),
  projectAdd: (location: Location, path: string) => call<[Settings, Project]>("project_add", { location, path }),
  projectRemove: (id: string) => call<Settings>("project_remove", { id }),
  sshConfigHosts: () => call<string[]>("ssh_config_hosts"),
  hostCheck: (location: Location) => call<HostCheck>("location_check", { location }),
  hostListDir: (location: Location, path: string) => call<{ path: string; dirs: string[] }>("location_list_dir", { location, path }),
  wslDistros: () => call<Distro[]>("wsl_distros"),
  wslStart: (distro: string) => call<void>("wsl_start", { distro }),
  wslPath: (path: string) => call<{ distro: string; path: string } | null>("wsl_path", { path }),
  providerProbe: (kind: AgentKind, binary: string) => call<Probe>("provider_probe", { kind, binary }),
  providerAccount: (kind: AgentKind, location: Location | null = null) => call<Account>("provider_account", { kind, location }),
  providerLimits: (kind: AgentKind, location: Location | null = null) => call<Limits>("provider_limits", { kind, location }),
  providerLogin: (kind: AgentKind, location: Location | null = null) => call<void>("provider_login", { kind, location }),

  chatList: () => call<ChatMeta[]>("chat_list"),
  chatRunning: () => call<string[]>("chat_running"),
  /** The chat as Markdown, for an issue or a PR description. */
  chatMarkdown: (chatId: string) => call<string>("chat_markdown", { chatId }),
  chatCreate: (projectId: string, agent: AgentKind | null, acpAgent: string | null, model: string | null, effort: string | null, cwd: string | null) =>
    call<ChatMeta>("chat_create", { projectId, agent, acpAgent, model, effort, cwd }),
  providerCatalog: (kind: AgentKind, projectId: string | null) => call<Catalog>("provider_catalog", { kind, projectId }),
  chatEvents: (chatId: string) => call<ChatEvent[]>("chat_events", { chatId }),
  /** The chat as it is now, mid-turn included. */
  chatSnapshot: (chatId: string) => call<Snapshot>("chat_snapshot", { chatId }),
  chatSend: (chatId: string, prompt: { id: string; text: string; images: { mime: string; data: string }[] }, permission: PermissionMode | null) =>
    call<void>("chat_send", { chatId, prompt, permission }),
  chatCompact: (chatId: string, instructions: string, permission: PermissionMode | null) => call<void>("chat_compact", { chatId, instructions, permission }),
  chatClear: (chatId: string) => call<void>("chat_clear", { chatId }),
  chatInterrupt: (chatId: string) => call<void>("chat_interrupt", { chatId }),
  chatAnswer: (chatId: string, requestId: string, decision: Decision) => call<void>("chat_answer", { chatId, requestId, decision }),
  chatSetPermission: (chatId: string, mode: PermissionMode) => call<void>("chat_set_permission", { chatId, mode }),
  chatAnswerQuestion: (chatId: string, requestId: string, answers: Record<string, string[]>) => call<void>("chat_answer_question", { chatId, requestId, answers }),
  chatSetModel: (chatId: string, model: string) => call<ChatMeta>("chat_set_model", { chatId, model }),
  chatSetEffort: (chatId: string, effort: string) => call<ChatMeta>("chat_set_effort", { chatId, effort }),
  /** An image saved with a chat, as a `data:` URL. */
  chatImage: (chatId: string, file: string) => call<string>("chat_image", { chatId, file }),
  chatSleep: (chatId: string) => call<void>("chat_sleep", { chatId }),
  chatClose: (chatId: string) => call<void>("chat_close", { chatId }),
  chatDelete: (chatId: string) => call<void>("chat_delete", { chatId }),
  chatRename: (chatId: string, title: string) => call<ChatMeta>("chat_rename", { chatId, title }),
  chatPin: (chatId: string, pinned: boolean) => call<ChatMeta>("chat_pin", { chatId, pinned }),
  mcpStatus: () => call<McpStatus>("mcp_status"),
}

/** What a terminal's shell sends: output, then its exit. */
export type TerminalEvent = { kind: "data"; data: string } | { kind: "exit"; code: number | null }

export const terminal = {
  /** The shell a new terminal in the project runs, when known without starting one (this computer). */
  shell: (projectId: string) => call<string | null>("terminal_shell", { projectId }),
  /** Starts a shell where the project is; its output arrives through `onEvent`. */
  async open(projectId: string, cwd: string | null, cols: number, rows: number, onEvent: (e: TerminalEvent) => void) {
    const { Channel } = await import("@tauri-apps/api/core")
    const channel = new Channel<TerminalEvent>()
    channel.onmessage = onEvent
    return call<{ id: string; shell: string | null }>("terminal_open", { projectId, cwd, cols, rows, onEvent: channel })
  },
  write: (id: string, data: string) => call<void>("terminal_write", { id, data }),
  resize: (id: string, cols: number, rows: number) => call<void>("terminal_resize", { id, cols, rows }),
  close: (id: string) => call<void>("terminal_close", { id }),
}

/** Calls `fn` for every chat event the app emits (`seq`: its number in the chat, as in a
 *  snapshot); returns the unsubscribe. */
export async function onChatEvent(fn: (chatId: string, event: ChatEvent, seq: number) => void): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event")
  return listen<{ chatId: string; event: ChatEvent; seq: number }>("chat-event", (e) => fn(e.payload.chatId, e.payload.event, e.payload.seq))
}

/** Calls `fn` when chats were added, renamed or removed outside the window (over MCP, or from
 *  a phone), or a turn started or ended. */
export async function onChatsChanged(fn: () => void): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event")
  return listen("chats-changed", () => fn())
}

/** Picks a folder on this computer; null when cancelled. */
export async function pickFolder(title: string): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog")
  const r = await open({ directory: true, multiple: false, title })
  return typeof r === "string" ? r : null
}
