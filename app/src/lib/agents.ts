// The agents a new chat can use.

import { AGENTS, type AgentKind, type Settings } from "./types"

/** Agents a new chat can use: the ones turned on in Settings → Providers. */
export function agentChoices(settings: Settings): AgentKind[] {
  return AGENTS.filter((k) => settings.providers[k].enabled)
}
