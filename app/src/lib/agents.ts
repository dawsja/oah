// Which agent a chat uses: a built-in kind, or "acp" plus the id of an agent the user added.

import { AGENTS, AGENT_LABEL, type AgentKind, type Provider, type Settings } from "./types"

export type AgentRef = { kind: AgentKind; acpId: string | null }

/** A string for selects: `claude`, `codex`, `opencode`, or `acp:<id>`. */
export const agentKey = (r: AgentRef) => (r.kind === "acp" ? `acp:${r.acpId ?? ""}` : r.kind)

export function parseAgentKey(key: string): AgentRef {
  return key.startsWith("acp:") ? { kind: "acp", acpId: key.slice(4) } : { kind: key as AgentKind, acpId: null }
}

export function agentName(settings: Settings, r: AgentRef): string {
  if (r.kind !== "acp") return AGENT_LABEL[r.kind]
  return settings.providers.acp.find((a) => a.id === r.acpId)?.name || "ACP agent"
}

export function providerFor(settings: Settings, r: AgentRef): Provider | null {
  if (r.kind === "acp") return settings.providers.acp.find((a) => a.id === r.acpId) ?? null
  return settings.providers[r.kind]
}

/** Agents a new chat can use: enabled built-ins, then enabled added agents. */
export function agentChoices(settings: Settings): AgentRef[] {
  return [
    ...AGENTS.filter((k) => settings.providers[k].enabled).map((kind) => ({ kind, acpId: null })),
    ...settings.providers.acp.filter((a) => a.enabled && a.binary.trim()).map((a) => ({ kind: "acp" as const, acpId: a.id })),
  ]
}
