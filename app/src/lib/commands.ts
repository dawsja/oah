// The composer's / menu: oah's own commands first, then the agent's (Claude Code's from its
// `initialize`, Codex skills from `skills/list`, OpenCode's `available_commands_update`).

import type { SlashCommand } from "./chat"
import type { AgentKind } from "./types"

/** Commands oah runs itself instead of sending them to the agent. */
export type OwnCommand = "model" | "plan" | "clear" | "compact"

export type MenuCommand = SlashCommand & { own?: OwnCommand }

const OWN: (MenuCommand & { chatOnly?: boolean })[] = [
  { name: "model", description: "Pick the model, or switch to the one named", hint: "<model>", skill: false, own: "model" },
  { name: "plan", description: "Plan first: the agent reads and proposes, changes nothing", hint: "", skill: false, own: "plan" },
  { name: "compact", description: "Summarize the conversation so far to free up context", hint: "<instructions>", skill: false, own: "compact", chatOnly: true },
  { name: "clear", description: "Start the agent fresh; the chat keeps what's above", hint: "", skill: false, own: "clear", chatOnly: true },
]

/** Before an agent lists its own (a chat that hasn't started, or an older CLI). */
const FALLBACK: Partial<Record<AgentKind, SlashCommand[]>> = {
  claude: [
    { name: "init", description: "Initialize a new CLAUDE.md file with codebase documentation", hint: "", skill: false },
    { name: "review", description: "Review the current changes", hint: "", skill: false },
  ],
}

/** What the menu offers: oah's commands, then the agent's (minus names oah handles). */
export function menuCommands(kind: AgentKind, agent: SlashCommand[], inChat: boolean): MenuCommand[] {
  const own = OWN.filter((c) => inChat || !c.chatOnly)
  const taken = new Set(OWN.map((c) => c.name))
  const listed = agent.length ? agent : (FALLBACK[kind] ?? [])
  const seen = new Set<string>()
  const theirs = listed.filter((c) => !taken.has(c.name) && !seen.has(c.name) && seen.add(c.name))
  return [...own.map(({ chatOnly: _, ...c }) => c), ...theirs]
}

/** `/na` at the very start of the prompt, with nothing after it yet: the query `na`. */
export function slashQuery(text: string, caret: number): string | null {
  const m = /^\/(\S*)$/.exec(text.slice(0, caret))
  return m && !/\s/.test(text.slice(caret, caret + 1) || "") ? m[1] : null
}

/** Names starting with the query first, then names with a word that does (`code-review`,
 *  `cloudflare:wrangler`); skills after commands. */
export function filterCommands(all: MenuCommand[], query: string): MenuCommand[] {
  const q = query.toLowerCase()
  const score = (c: MenuCommand) => {
    const n = c.name.toLowerCase()
    return n.startsWith(q) ? 0 : n.split(/[-_:.]/).some((w) => w.startsWith(q)) ? 1 : -1
  }
  return all
    .map((c, i) => ({ c, i, s: score(c) }))
    .filter((x) => x.s >= 0)
    .sort((a, b) => a.s - b.s || Number(a.c.skill) - Number(b.c.skill) || a.i - b.i)
    .map((x) => x.c)
}

/** `/compact keep the API notes` → `{ name: "compact", args: "keep the API notes" }`. */
export function parseSlash(text: string): { name: string; args: string } | null {
  const m = /^\/([^\s/]+)(?:\s+([\s\S]*))?$/.exec(text.trim())
  return m ? { name: m[1], args: (m[2] ?? "").trim() } : null
}

/** The oah command a prompt runs, if any. */
export function ownCommand(text: string, inChat: boolean): { own: OwnCommand; args: string } | null {
  const p = parseSlash(text)
  const c = p && OWN.find((o) => o.name === p.name.toLowerCase() && (inChat || !o.chatOnly))
  return c && p ? { own: c.own!, args: p.args } : null
}
