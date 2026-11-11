// Which providers Settings → Providers lists. By default, the ones that are there to use: installed
// on this computer, the default agent, or one a chat already uses (an agent installed only in a
// WSL distro or on an SSH host isn't found here, but its chats are). The rest wait in the
// "Add provider" menu. Adding one keeps it listed (to see how to install it), hiding one keeps
// it out, whatever is installed; both are this window's own choice, kept in localStorage.

import { AGENTS, type AgentKind } from "./types"

/** What the user chose for a provider: listed, hidden, or neither (follow the default). */
export type ListChoice = "shown" | "hidden"

export type ListFacts = {
  /** Found on this computer; null while it's still being looked for. */
  installed: boolean | null
  isDefault: boolean
  inUse: boolean
}

/** Whether a provider is listed: the user's choice, else whether it's there to use. Unknown
 *  yet (still looking): only if something else says it's used. */
export function listed(choice: ListChoice | undefined, f: ListFacts): boolean {
  if (choice) return choice === "shown"
  return f.installed === true || f.isDefault || f.inUse
}

const KEY = "oah.providers.listed"

export function loadChoices(): Partial<Record<AgentKind, ListChoice>> {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "{}")
    if (!v || typeof v !== "object") return {}
    return Object.fromEntries(AGENTS.filter((a) => v[a] === "shown" || v[a] === "hidden").map((a) => [a, v[a]]))
  } catch {
    return {}
  }
}

export function saveChoices(choices: Partial<Record<AgentKind, ListChoice>>) {
  try {
    localStorage.setItem(KEY, JSON.stringify(choices))
  } catch {
    /* ignore */
  }
}
