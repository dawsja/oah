export interface Command {
  name: string
  args: string
  desc: string
}

export const COMMANDS: Command[] = [
  { name: "model", args: "[provider/model]", desc: "Choose the model (ChatGPT plan, OpenAI, Anthropic)" },
  { name: "effort", args: "[minimal…max]", desc: "Set reasoning effort" },
  { name: "permissions", args: "[auto|ask|allow]", desc: "Permission level (also shift+tab)" },
  { name: "plan", args: "", desc: "Plan mode: read-only research, produce a plan (also tab)" },
  { name: "build", args: "", desc: "Build mode: edit files, run commands (also tab)" },
  { name: "auth", args: "", desc: "Sign in with ChatGPT, or add OpenAI / Anthropic keys" },
  { name: "goal", args: "<objective>|clear", desc: "Keep working autonomously until the goal is done" },
  { name: "compact", args: "[focus]", desc: "Summarize the conversation to free context" },
  { name: "clear", args: "", desc: "Start a fresh conversation" },
  { name: "session", args: "[id]", desc: "Browse and resume saved sessions" },
  { name: "design", args: "[name]", desc: "Transcript look: panels · bubbles · cards · gutter · focus" },
  { name: "memory", args: "[add|global|remove <text>]", desc: "View or edit persistent memory" },
  { name: "skills", args: "", desc: "List available skills" },
  { name: "mcp", args: "", desc: "Show MCP servers and tools" },
  { name: "tools", args: "", desc: "List the tools nur can use" },
  { name: "init", args: "", desc: "Write an AGENTS.md for this project" },
  { name: "copy", args: "", desc: "Copy the last answer to the clipboard" },
  { name: "help", args: "", desc: "Keys and commands" },
  { name: "exit", args: "", desc: "Quit (prints the resume command)" },
]

const ALIASES: Record<string, string> = {
  perm: "permissions",
  permission: "permissions",
  new: "clear",
  reset: "clear",
  sessions: "session",
  resume: "session",
  login: "auth",
  models: "model",
  theme: "design",
  quit: "exit",
  q: "exit",
  "?": "help",
}

export const resolveCommand = (n: string) => ALIASES[n] ?? n

export function matchCommands(prefix: string, skills: { name: string; description: string }[]): Command[] {
  const p = prefix.toLowerCase()
  const out = COMMANDS.filter((c) => c.name.startsWith(p))
  for (const s of skills) if (s.name.toLowerCase().startsWith(p) && !out.some((c) => c.name === s.name)) out.push({ name: s.name, args: "", desc: `skill · ${s.description}` })
  if (!out.length && p) {
    for (const c of COMMANDS) {
      let i = 0
      for (const ch of c.name) if (ch === p[i]) i++
      if (i === p.length) out.push(c)
    }
  }
  return out
}

export const HELP = `## Keys
- **enter** send · **shift+enter** / **alt+enter** / **ctrl+j** newline
- **↑ / ↓** previous / next prompt (from the first / last line)
- **tab** plan ↔ build · **shift+tab** cycle permissions (auto → always ask → always allow)
- **esc** interrupt the running turn · **ctrl+c** quit (prints how to resume)
- **pgup / pgdn**, mouse wheel — scroll · drag to select text (copied on release)
- **#note** saves a note to project memory

## Commands
- **/model**, **/effort**, **/permissions**, **/plan**, **/build**, **/design**
- **/auth** — continue with ChatGPT (uses your plan), or add an OpenAI / Anthropic API key
- **/goal** <objective> — keep going until done (**/goal clear** stops)
- **/compact** [focus] — summarize history; auto-compaction runs near the context limit
- **/clear** new conversation · **/session** resume one · **/memory** · **/init**
- **/skills**, **/mcp**, **/tools** · **/<skill>** runs a skill · **/copy** · **/exit**`
