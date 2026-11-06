# nar

A full-screen AI coding agent for your terminal, written in Rust and rendered with
[OpenTUI](https://github.com/anomalyco/opentui)'s native (Zig) core. It uses your
**ChatGPT plan** through Sign in with ChatGPT, or an OpenAI API key.

![nar](docs/screenshot.png)

## The look

nar takes over the whole terminal (alternate screen, mouse, bracketed paste, Kitty keyboard):

```text
 ◆ nar  ~/code/project                                    ◎ goal · turn 2 · session 3f2a91c0   ← header
 ▌ you                                                                                         │
 ▌ add a --json flag to the export command                                                     │ ← scrollable
                                                                                               │   transcript
 ∴ Looking at how flags are parsed in src/cli.rs…                                              ┃   (scrollbar)
 ⏺ Read src/cli.rs                                                                             ┃
   ⎿ 214 of 214 lines                                                                          ┃
 ⏺ Edit src/cli.rs                                                                             ┃
   ⎿ +6 -1                                                                                     │
     -    let fmt = Format::Text;                                                              │   diffs inline
     +    let fmt = if args.json { Format::Json } else { Format::Text };                       │
 ● Added `--json`. Tests pass (`cargo test`: 48 passed).                                       │
 ☰ Tasks  2/3                                                                                  │ ← live todo list
   ☒ Parse the flag   ☒ Emit JSON   ◐ Update docs                                              │
 ⠹ Running Bash cargo test… 14s · esc to interrupt                                             ← activity line
╭──────────────────────────────────────────────────────────────────────────────────────────────╮
│ › ask anything · / for commands · shift+enter for newline                                     │ ← input
╰──────────────────────────────────────────────────────────────────────────────────────────────╯
 gpt-5.5 · high · auto · ◆ build (shift+tab)          Using ChatGPT plan · 41k / 272k ▰▰▱▱▱▱▱▱▱▱ 15%
 └ model  └ effort └ permission └ plan/build mode                               context of the model ┘
```

* **Transcript** — your messages on a tinted panel (`▌`), answers as Markdown (`●`, headings,
  lists, code blocks, inline code, links), reasoning summaries dimmed (`∴`), tool calls with a
  status glyph (`◌` running, `⏺` green/red done/failed, `⊘` denied), a one-line result (`⎿`)
  and inline diffs or output previews.
* **Activity line** — spinner, what's happening and elapsed time; toasts appear here too.
* **Input** — multi-line, soft-wrapped. Border is violet in build mode and amber in plan mode.
* **Status bar** — model · effort · permission · mode on the left; auth and a live context meter
  (green → amber → red) on the far right.
* **Overlays** — the `/` command palette opens above the input; pickers (`/model`, `/effort`,
  `/permissions`, `/session`, `/auth`) open as centered, filterable modals; permission requests
  replace the input with an amber `y` / `a` / `n` prompt.

## Features

* Scrollable history (wheel, PgUp/PgDn, Ctrl+Home/End) with a scrollbar and "↓ more" indicator
* Drag to select text in the transcript; it is copied on release (OSC 52, plus `wl-copy`,
  `xclip`, `xsel` or `pbcopy` when available). Gutters and bullets are never copied.
* ↑ / ↓ walks through previous prompts (persisted across sessions)
* `/model`, `/effort`, `/auth`, `/permissions`, `/plan`, `/build`
* `/session` and `nar --resume <id>`: every conversation is saved; **Ctrl+C** quits and prints
  the exact resume command
* `/goal <objective>` — keeps going turn after turn until the model calls `goal_complete`
  (esc pauses, `/goal clear` stops)
* `/compact [focus]` and **automatic compaction** near the context limit (85% by default)
* `/clear` for a fresh conversation
* **Memory** — `AGENTS.md` / `NAR.md` / `CLAUDE.md` instruction files (global and per directory),
  plus persistent notes saved with `#note`, `/memory add`, or by the model's `memory` tool
* **Skills** — `SKILL.md` folders; listed to the model, loaded on demand, or run with `/<name>`
* **MCP** — stdio and Streamable HTTP servers; tools appear as `mcp__<server>__<tool>`
* Three permission levels: **auto**, **always ask**, **always allow**
* Plan mode (read-only research → plan) and build mode, toggled with **Shift+Tab**

## Install

Requirements: Rust 1.80+, [Zig 0.17](https://ziglang.org/download/), git.

```sh
git clone --recursive https://github.com/dawsja/nar
cd nar
cargo build --release          # builds OpenTUI's native library with Zig, then nar
./target/release/nar
```

The build copies `libopentui.so` (or `.dylib`) next to the binary, which finds it through an
`$ORIGIN` rpath. To install, copy both files into the same directory (or the library into
`../lib`).

Already have the OpenTUI library? Skip Zig with
`OPENTUI_LIB_DIR=/path/to/dir/with/libopentui cargo build --release`. The prebuilt
`libopentui.so` in the `@opentui/core-<platform>@0.8.2` npm packages works too, since it
exports the same C ABI. `OPENTUI_SRC_DIR` points the build at another OpenTUI checkout, and
`OPENTUI_REBUILD=1` forces a rebuild.

Supported: Linux and macOS (x86_64, aarch64).

## Sign in

Run `/auth` (or `nar login`) and choose **Continue with ChatGPT**. nar follows the
[open-source ChatGPT plan usage flow](https://developers.openai.com/siwc/token-sharing-open-source):

* a stable, opaque host id (`urn:uuid:…`) is created once in `~/.nar/host_id`
* the first sign-in registers nar as a user-defined agent (`dynamic_agent_client`) and stores
  the issued client id; later sign-ins reuse it
* PKCE (S256), state and nonce, loopback redirect on `127.0.0.1:1455`
* the ID token is verified against OpenAI's JWKS, and the granted scopes must include
  `chatgpt.tokens.use.direct`
* tokens refresh automatically and are stored with `0600` permissions; `/auth logout` revokes them

Requests go to `POST https://api.openai.com/v1/responses` with `store: false` and `stream: true`,
replaying history (including encrypted reasoning) on each turn. Usage-limit errors link to
[chatgpt.com/settings/usage](https://chatgpt.com/settings/usage).

Prefer an API key? Choose **Use an OpenAI API key** in `/auth`, or set `OPENAI_API_KEY`.

## Keys

| Key | Action |
| --- | --- |
| Enter | send |
| Shift+Enter, Alt+Enter, Ctrl+J | newline |
| ↑ / ↓ | previous / next prompt (from the first / last line) |
| Shift+Tab | toggle plan ↔ build |
| Esc | interrupt the running turn |
| PgUp / PgDn, wheel | scroll · Ctrl+Home / Ctrl+End jump |
| mouse drag | select and copy |
| Ctrl+A/E, Ctrl+U/K/W, Alt+←/→ | line and word editing |
| Ctrl+C | quit (prints the resume command) |

## Commands

| Command | |
| --- | --- |
| `/model [slug]` | choose the model (listed from the API) |
| `/effort [minimal…xhigh]` | reasoning effort |
| `/permissions [auto\|ask\|allow]` | permission level |
| `/plan`, `/build` | switch mode |
| `/auth [login\|apikey\|logout\|status]` | authentication |
| `/goal <objective>` · `/goal clear` | autonomous mode until complete |
| `/compact [focus]` | summarize history to free context |
| `/clear` | new conversation |
| `/session [id]` | browse and resume sessions |
| `/memory [add\|global\|remove\|clear]` | persistent memory |
| `/skills`, `/mcp` | list skills and MCP servers |
| `/init` | have the agent write an `AGENTS.md` |
| `/copy` | copy the last answer |
| `/help`, `/exit` | |

CLI: `nar [prompt]`, `nar -c` (continue latest here), `nar -r [id]`, `nar sessions`,
`nar login`, `nar logout`, `-m <model>`, `-e <effort>`, `-p <auto|ask|allow>`, `--plan`.

## Permissions

| Level | Reads in workspace | Safe commands (`ls`, `git status`, …) | Edits, other commands, MCP, reads outside |
| --- | --- | --- | --- |
| auto | run | run | ask |
| always ask | ask | ask | ask |
| always allow | run | run | run |

At a prompt: **y** allow once, **a** always allow (this session; per command name, per MCP
tool, or all edits), **n** / Esc deny. Add your own safe command prefixes to
`allowed_commands` in `~/.nar/settings.json`. Plan mode only exposes read-only tools.

## Memory, skills and MCP

* Instruction files: `~/.nar/AGENTS.md`, then for each directory from the repository root down to
  the working directory the first of `AGENTS.md`, `NAR.md`, `CLAUDE.md`.
* Notes: `~/.nar/memory/global.md` and `~/.nar/memory/projects/<project>-<hash>.md`.
* Skills: `./.nar/skills`, `./.agents/skills`, `./.claude/skills`, `~/.nar/skills`,
  `~/.agents/skills`, `~/.claude/skills` — each skill is a folder with a `SKILL.md`:

  ```markdown
  ---
  name: release-notes
  description: Write release notes from the git log
  ---
  Steps…
  ```

* MCP servers: `~/.nar/mcp.json`, `./.nar/mcp.json` or `./.mcp.json`:

  ```json
  {
    "mcpServers": {
      "filesystem": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "."] },
      "docs": { "url": "https://example.com/mcp", "headers": { "Authorization": "Bearer …" } }
    }
  }
  ```

## Files

```text
~/.nar/                 ($NAR_HOME overrides)
  settings.json         model, effort, permission, mode, api_base, context_window, auto_compact_at
  auth.json             credentials (0600)
  host_id               ext_agent_host_id for Sign in with ChatGPT
  history.jsonl         prompt history
  sessions/<id>.json    saved conversations
  memory/               persistent notes
  skills/, mcp.json, AGENTS.md
```

## Architecture

```text
crates/opentui   Safe Rust bindings to OpenTUI's C ABI (opentui.h, ABI v2). The host owns raw
                 mode, the clock and the event loop; OpenTUI handles terminal setup and
                 restoration, capability detection, input parsing, diffed rendering and OSC 52.
                 Frames are drawn immediate-mode into an offscreen buffer and presented.
crates/nar       The app: ui/ (state, rendering, editor, Markdown), agent (turn loop),
                 api (Responses streaming), auth (Sign in with ChatGPT), tools, mcp,
                 skills, memory, session.
vendor/opentui   OpenTUI v0.8.2 (git submodule), built with Zig by crates/opentui/build.rs.
```

## License

MIT
