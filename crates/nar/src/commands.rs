//! Slash commands.

pub struct Command {
    pub name: &'static str,
    pub args: &'static str,
    pub desc: &'static str,
}

pub const COMMANDS: &[Command] = &[
    Command { name: "model", args: "[slug]", desc: "Choose the model" },
    Command { name: "effort", args: "[minimal|low|medium|high|xhigh]", desc: "Set reasoning effort" },
    Command { name: "permissions", args: "[auto|ask|allow]", desc: "Set permission level (auto · always ask · always allow)" },
    Command { name: "plan", args: "", desc: "Switch to plan mode (read-only, produce a plan)" },
    Command { name: "build", args: "", desc: "Switch to build mode (edit files, run commands)" },
    Command { name: "auth", args: "[login|apikey|logout|status]", desc: "Sign in with ChatGPT or use an API key" },
    Command { name: "goal", args: "<objective>|clear", desc: "Work autonomously until the goal is complete" },
    Command { name: "compact", args: "[focus]", desc: "Summarize the conversation to free context" },
    Command { name: "clear", args: "", desc: "Start a fresh conversation" },
    Command { name: "session", args: "[id]", desc: "Browse and resume saved sessions" },
    Command { name: "memory", args: "[add <text>|remove <text>|global <text>]", desc: "View or edit persistent memory" },
    Command { name: "skills", args: "", desc: "List available skills" },
    Command { name: "mcp", args: "", desc: "Show MCP servers and tools" },
    Command { name: "init", args: "", desc: "Create an AGENTS.md for this project" },
    Command { name: "copy", args: "", desc: "Copy the last response to the clipboard" },
    Command { name: "help", args: "", desc: "Keys and commands" },
    Command { name: "exit", args: "", desc: "Quit (prints the resume command)" },
];

pub const ALIASES: &[(&str, &str)] = &[
    ("perm", "permissions"),
    ("permission", "permissions"),
    ("new", "clear"),
    ("reset", "clear"),
    ("sessions", "session"),
    ("resume", "session"),
    ("login", "auth"),
    ("logout", "auth logout"),
    ("quit", "exit"),
    ("q", "exit"),
    ("models", "model"),
    ("?", "help"),
];

pub fn resolve(name: &str) -> String {
    for (a, target) in ALIASES {
        if *a == name {
            return target.to_string();
        }
    }
    name.to_string()
}

/// Commands (and skills) whose name starts with `prefix`, best first.
pub fn matches<'a>(prefix: &str, skills: &'a [(String, String)]) -> Vec<(String, String, String)> {
    let p = prefix.to_ascii_lowercase();
    let mut out: Vec<(String, String, String)> = COMMANDS
        .iter()
        .filter(|c| c.name.starts_with(&p))
        .map(|c| (c.name.to_string(), c.args.to_string(), c.desc.to_string()))
        .collect();
    for (name, desc) in skills {
        if name.to_ascii_lowercase().starts_with(&p) && !out.iter().any(|(n, _, _)| n == name) {
            out.push((name.clone(), String::new(), format!("skill · {desc}")));
        }
    }
    if out.is_empty() && !p.is_empty() {
        // Fuzzy: subsequence match.
        for c in COMMANDS {
            let mut it = c.name.chars();
            if p.chars().all(|pc| it.any(|cc| cc == pc)) {
                out.push((c.name.to_string(), c.args.to_string(), c.desc.to_string()));
            }
        }
    }
    out
}

pub const HELP: &str = "\
## Keys
- **enter** send · **shift+enter** / **alt+enter** / **ctrl+j** newline
- **↑ / ↓** previous / next prompt (when on the first / last line)
- **shift+tab** toggle plan ↔ build · **esc** interrupt the running turn
- **pgup / pgdn**, mouse wheel — scroll · **ctrl+home / ctrl+end** top / bottom
- **drag** to select text — it is copied when you release
- **ctrl+c** quit (prints how to resume) · **ctrl+l** redraw
- **#note** saves a note to project memory

## Commands
- **/model**, **/effort**, **/permissions** — choose model, reasoning effort, permission level
- **/plan**, **/build** — switch mode
- **/auth** — sign in with ChatGPT (uses your plan) or set an API key
- **/goal** <objective> — keep working autonomously until done (**/goal clear** to stop)
- **/compact** [focus] — summarize history; auto-compaction runs near the context limit
- **/clear** — new conversation · **/session** — resume a saved one
- **/memory** — view/add/remove memory · **/init** — write AGENTS.md
- **/skills**, **/mcp** — list skills and MCP servers · **/<skill>** runs a skill
- **/copy** — copy last response · **/exit**

## Permission levels
- **auto** — reads and safe commands run; edits and other commands ask
- **always ask** — every tool call asks
- **always allow** — nothing asks";
