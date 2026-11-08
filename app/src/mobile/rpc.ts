// The phone app's calls: its own Rust side (android/src-tauri: pairing, the session) and, through
// it, the desktop's methods (crates/oah-remote/src/proto.rs).

import type { ChatEvent, ChatMeta, Decision, ModelOption, SlashCommand, Snapshot } from "@/lib/chat"
import type { AgentKind, PermissionMode, Project } from "@/lib/types"

/** A desktop this phone paired with. `id` is its public key. */
export type Desktop = { id: string; name: string; addresses: string[]; deviceId: string; pairedAt: number; lastAddress: string | null }

export type LinkState = "idle" | "connecting" | "connected" | "offline" | "revoked"
export type LinkEvent =
  | { kind: "state"; state: Exclude<LinkState, "idle">; error?: string; address?: string; desktopName?: string }
  | { kind: "push"; push: Push }
export type Push = { push: "chat"; chatId: string; event: ChatEvent } | { push: "chats" } | { push: "resync" }

/** What a chat waits for from you. */
export type Waiting = "approval" | "question" | "plan"

/** An agent the desktop has turned on. */
export type AgentInfo = { kind: AgentKind; acpId: string | null; name: string; defaultModel: string; permissionMode: PermissionMode }
export type Catalog = { models: ModelOption[]; commands: SlashCommand[] }
export type Image = { mime: string; data: string }

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core")
  return invoke<T>(cmd, args)
}

export const native = {
  desktops: () => invoke<Desktop[]>("desktops"),
  /** From a scanned link, or a typed address and code. */
  pair: (o: { link?: string; addresses?: string[]; code?: string; name: string }) =>
    invoke<Desktop>("pair", { link: o.link ?? null, addresses: o.addresses ?? [], code: o.code ?? "", name: o.name }),
  forget: (id: string) => invoke<void>("forget", { id }),
  async connect(id: string, onEvent: (e: LinkEvent) => void) {
    const { Channel } = await import("@tauri-apps/api/core")
    const ch = new Channel<LinkEvent>()
    ch.onmessage = onEvent
    return invoke<void>("connect", { id, onEvent: ch })
  },
  disconnect: () => invoke<void>("disconnect"),
  wake: () => invoke<void>("wake"),
}

const rpc = <T>(method: string, params: Record<string, unknown> = {}) => invoke<T>("rpc", { method, params })

export const api = {
  hello: () => rpc<{ desktopName: string; protocol: number; deviceId: string }>("hello"),
  projects: () => rpc<Project[]>("projects.list"),
  agents: () => rpc<{ agents: AgentInfo[]; defaultAgent: AgentKind }>("agents.list"),
  chats: () => rpc<{ metas: ChatMeta[]; running: string[]; busy: string[]; waiting?: Record<string, Waiting> }>("chats.list"),
  open: (chatId: string) => rpc<Snapshot>("chat.open", { chatId }),
  close: (chatId: string) => rpc<void>("chat.close", { chatId }),
  create: (projectId: string, agent: AgentKind, acpAgent: string | null, model: string | null) => rpc<ChatMeta>("chat.create", { projectId, agent, acpAgent, model }),
  send: (chatId: string, text: string, images: Image[], permission: PermissionMode | null) =>
    rpc<void>("chat.send", { chatId, prompt: { id: crypto.randomUUID(), text, images }, permission }),
  interrupt: (chatId: string) => rpc<void>("chat.interrupt", { chatId }),
  answer: (chatId: string, requestId: string, decision: Decision) => rpc<void>("chat.answer", { chatId, requestId, decision }),
  answerQuestion: (chatId: string, requestId: string, answers: Record<string, string[]>) => rpc<void>("chat.answerQuestion", { chatId, requestId, answers }),
  setModel: (chatId: string, model: string) => rpc<ChatMeta>("chat.setModel", { chatId, model }),
  setEffort: (chatId: string, effort: string) => rpc<ChatMeta>("chat.setEffort", { chatId, effort }),
  setPermission: (chatId: string, mode: PermissionMode) => rpc<void>("chat.setPermission", { chatId, mode }),
  compact: (chatId: string, instructions: string, permission: PermissionMode | null) => rpc<void>("chat.compact", { chatId, instructions, permission }),
  clear: (chatId: string) => rpc<void>("chat.clear", { chatId }),
  image: (chatId: string, file: string) => rpc<string>("chat.image", { chatId, file }),
  rename: (chatId: string, title: string) => rpc<ChatMeta>("chat.rename", { chatId, title }),
  catalog: (kind: AgentKind, projectId: string | null) => rpc<Catalog>("provider.catalog", { kind, projectId }),
}

/** What this phone calls itself when it pairs: its model, from the WebView's user agent. */
export function phoneName(ua = navigator.userAgent): string {
  const m = /Android [\d.]+; ([^;)]+?)(?: Build\/[^;)]*)?[;)]/.exec(ua)
  const model = m?.[1]?.trim()
  return model && model !== "K" && !/^wv$/i.test(model) ? model : "Android phone"
}

/** Reads a picked file as `{mime, data}` (base64, no `data:` prefix). */
export function readImage(file: File): Promise<Image> {
  return new Promise((resolve, reject) => {
    const r = new FileReader()
    r.onload = () => {
      const s = String(r.result)
      resolve({ mime: file.type || "image/png", data: s.slice(s.indexOf(",") + 1) })
    }
    r.onerror = () => reject(r.error)
    r.readAsDataURL(file)
  })
}
