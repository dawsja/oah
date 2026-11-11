// The phone app's calls: its own Rust side (phone/src-tauri: pairing, the session) and, through
// it, the desktop's methods (crates/oah-remote/src/proto.rs).

import type { ChatEvent, ChatMeta, Decision, ModelOption, SlashCommand, Snapshot } from "@/lib/chat"
import type { AgentKind, PermissionMode, Project } from "@/lib/types"

/** A desktop this phone paired with. `id` is its public key. `demo`: the one inside the app,
 *  there to try the app without a computer (never saved). */
export type Desktop = { id: string; name: string; addresses: string[]; deviceId: string; pairedAt: number; lastAddress: string | null; demo?: boolean }

export type LinkState = "idle" | "connecting" | "connected" | "offline" | "revoked"
export type LinkEvent =
  | { kind: "state"; state: Exclude<LinkState, "idle">; error?: string; address?: string; desktopName?: string }
  | { kind: "push"; push: Push }
/** `seq`: the event's number in its chat (absent from desktops before it was sent). */
export type Push = { push: "chat"; chatId: string; seq?: number; event: ChatEvent } | { push: "chats" } | { push: "resync" }

/** What a chat waits for from you. */
export type Waiting = "approval" | "question" | "plan"

/** An agent the desktop has turned on. */
export type AgentInfo = { kind: AgentKind; name: string; defaultModel: string; permissionMode: PermissionMode }
export type Catalog = { models: ModelOption[]; commands: SlashCommand[] }
export type Image = { mime: string; data: string }

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core")
  return invoke<T>(cmd, args)
}

export type NearbyDesktop = { name: string; address: string }
export const native = {
  discover: () => invoke<NearbyDesktop[]>("discover"),
  requestPairing: (address: string, name: string) => invoke<void>("request_pairing", { address, name }),
  desktops: () => invoke<Desktop[]>("desktops"),
  /** Starts the demo desktop inside the app and pairs with it. */
  demo: () => invoke<Desktop>("demo_start"),
  /** How the app was asked to start (screenshots and tests): in the demo, on a screen. */
  launch: () => invoke<{ demo: boolean; route: string | null }>("launch"),
  /** With the address and code the desktop shows. */
  pair: (o: { addresses: string[]; code: string; name: string }) => invoke<Desktop>("pair", { addresses: o.addresses, code: o.code, name: o.name }),
  forget: (id: string) => invoke<void>("forget", { id }),
  async connect(id: string, onEvent: (e: LinkEvent) => void) {
    const { Channel } = await import("@tauri-apps/api/core")
    const ch = new Channel<LinkEvent>()
    ch.onmessage = onEvent
    return invoke<void>("connect", { id, onEvent: ch })
  },
  disconnect: () => invoke<void>("disconnect"),
  wake: () => invoke<void>("wake"),
  /** This app's page in the phone's Settings (an iPhone's Local Network switch). */
  openSettings: () => invoke<void>("open_settings"),
}

/** A light tap under the finger. Safari has no `navigator.vibrate`: the iPhone's own haptics do it. */
export function tap() {
  if (isIOS()) void invoke<void>("haptic").catch(() => {})
  else navigator.vibrate?.(10)
}

/** An iPhone keeps apps off the local network until it's allowed in Settings; a connection then
 *  fails as if the desktop weren't there. */
export function needsLocalNetwork(error: string | null | undefined, ios = isIOS()): boolean {
  return ios && !!error && /Local Network|No route to host|os error 65\b/i.test(error)
}

const rpc = <T>(method: string, params: Record<string, unknown> = {}) => invoke<T>("rpc", { method, params })

export const api = {
  projects: () => rpc<Project[]>("projects.list"),
  agents: () => rpc<{ agents: AgentInfo[]; defaultAgent: AgentKind }>("agents.list"),
  chats: () => rpc<{ metas: ChatMeta[]; running: string[]; busy: string[]; waiting?: Record<string, Waiting>; cli?: string[] }>("chats.list"),
  open: (chatId: string) => rpc<Snapshot>("chat.open", { chatId }),
  close: (chatId: string) => rpc<void>("chat.close", { chatId }),
  create: (projectId: string, agent: AgentKind, model: string | null) => rpc<ChatMeta>("chat.create", { projectId, agent, model }),
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
  /** Ends the agent's CLI the chat is open in on the desktop; the chat takes messages again. */
  endCli: (chatId: string) => rpc<boolean>("chat.endCli", { chatId }),
  readImage: (chatId: string, path: string) => rpc<string>("chat.readImage", { chatId, path }),
  image: (chatId: string, file: string) => rpc<string>("chat.image", { chatId, file }),
  rename: (chatId: string, title: string) => rpc<ChatMeta>("chat.rename", { chatId, title }),
  pin: (chatId: string, pinned: boolean) => rpc<ChatMeta>("chat.pin", { chatId, pinned }),
  sleep: (chatId: string) => rpc<void>("chat.sleep", { chatId }),
  remove: (chatId: string) => rpc<void>("chat.delete", { chatId }),
  catalog: (kind: AgentKind, projectId: string | null) => rpc<Catalog>("provider.catalog", { kind, projectId }),
}

/** An iPhone (or iPad): its WebView is Safari's, which differs from Android's in a few places. */
export function isIOS(ua = navigator.userAgent): boolean {
  return /\b(iPhone|iPad|iPod)\b/.test(ua)
}

/** What this phone calls itself when it pairs: its model, from the WebView's user agent (iOS
 *  doesn't say which iPhone). */
export function phoneName(ua = navigator.userAgent): string {
  const apple = /\b(iPhone|iPad|iPod)\b/.exec(ua)
  if (apple) return apple[1]
  if (!/Android/.test(ua)) return "Phone"
  const m = /Android [\d.]+; ([^;)]+?)(?: Build\/[^;)]*)?[;)]/.exec(ua)
  const model = m?.[1]?.trim()
  return model && model !== "K" && !/^wv$/i.test(model) ? model : "Android phone"
}
