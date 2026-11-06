# oah

A full-screen AI coding agent for your terminal. Built with
[OpenTUI](https://github.com/anomalyco/opentui) (Zig core, React bindings), the
[AI SDK](https://ai-sdk.dev) and the [models.dev](https://models.dev) catalog — the same stack as opencode.

Use your **ChatGPT plan** (Sign in with ChatGPT), local models in **Ollama** or **LM Studio**, or a key for **Anthropic, OpenAI, Gemini, xAI, DeepSeek,
OpenRouter, Synthetic, Kimi, Z.ai or MiniMax** (including the Kimi Code, GLM Coding and MiniMax Coding plans).

![oah](docs/screenshot.png)

```sh
curl -fsSL https://oah.dev/install.sh | sh
oah
```

## Layout

Answer-first. Your prompt is a `›` heading; thinking and tool calls fold into one live
`▸ 5 steps · Read, Edit, Bash · 12s` line (edits keep their diffs, failures show in red), then the answer.
The input box shows **plan/build · model · effort** in its bottom-left corner; the status bar below shows
the **permission** level on the left and the **context meter** on the right.

## Scripts and CI

```sh
oah -p "fix the failing tests"              # run once: answer on stdout, progress on stderr
npm test 2>&1 | oah -p "why did this fail?"  # piped input is added to the prompt
oah -c -p "now add a test for it"            # continue the latest session here
oah -p --json "summarize src/" | jq .cost    # one JSON object: result, session, cost, errors
```

Nobody is around to approve anything, so tool calls that would ask (edits, non-read-only commands) are
denied and listed on stderr. Pass `--permission allow` (or `--yolo`) to let it change files. Exit code is 1
on errors, 130 if interrupted. Checkpoints are still taken, so `/undo` in the app can roll a run back.

## Keys

| Key | |
| --- | --- |
| Enter | send · Shift+Enter / Alt+Enter / Ctrl+J / `\` then Enter: newline |
| Enter *(while working)* | queue the message — it's sent when the current turn finishes |
| **Ctrl+Enter** *(while working)* | send now: the model sees it after its current step, without stopping (empty input: sends the queued messages; plain Enter on an empty input does too) |
| ↑ / ↓ | previous / next prompt |
| **Ctrl+V** | attach an image from the clipboard as `[Image #1]` — Ctrl+Shift+V and Alt+V work too; so does dragging in an image file |
| **Ctrl+T** | switch model — recent models on top, previous one preselected (Ctrl+T → Enter flips back) |
| **Tab** | toggle **plan ↔ build** |
| **Shift+Tab** | cycle permissions: **auto → always ask → always allow** |
| Esc | interrupt the running turn (queued messages go back into the input) |
| PgUp / PgDn, wheel | scroll |
| mouse drag | select text — copied on release (OSC 52 + wl-copy / xclip / pbcopy / clip.exe) |
| Ctrl+C | stop the running turn; when idle, press twice to quit — prints `oah --resume <id>` |
| `#note` | save a note to project memory |

## Commands

`/model` `/effort` `/permissions` `/plan` `/build` `/auth` `/goal <objective>` `/compact [focus]`
`/clear` `/session [id]` `/memory` `/skills` `/mcp` `/import` `/tools` `/init` `/copy` `/help` `/exit`, plus `/<skill>`.

- **/goal** keeps working turn after turn until the model calls `goal_complete` (Esc pauses, `/goal clear` stops).
- **Auto-compaction** summarizes the conversation at 85% of the model's context window (from models.dev).
- **/session** lists saved conversations; `oah -c` continues the latest here, `oah -r <id>` resumes one.
- **/skills** lists skill names; enter puts `/<name>` in the input.
- **/mcp** lists servers with their state; pick one that **needs authentication** to sign in (OAuth in your
  browser, tokens refreshed automatically), a failed one to retry, a ready one to see its tools.
  `/mcp login|logout <server>`, `/mcp reload`.
- **/import** `[mcp] [skills] [sessions]` brings over Claude Code's setup: user, plugin and this project's
  MCP servers; plugin and synced skills (`~/.claude/skills` is read directly); and sessions, which open in
  `/session`. Anything already imported is skipped, so it's safe to run again.

## Providers

| Provider | Example model id | Key (`/auth`, or env) |
| --- | --- | --- |
| ChatGPT plan | `chatgpt/gpt-5.6` | Continue with ChatGPT (or `oah login`) |
| Anthropic | `anthropic/claude-opus-5-5` | `ANTHROPIC_API_KEY` |
| OpenAI | `openai/gpt-5.6` | `OPENAI_API_KEY` |
| Gemini | `google/gemini-4-argon` | `GEMINI_API_KEY` |
| xAI | `xai/grok-4.7` | `XAI_API_KEY` |
| DeepSeek | `deepseek/deepseek-v4-pro` | `DEEPSEEK_API_KEY` |
| OpenRouter | `openrouter/anthropic/claude-opus-5.5` | `OPENROUTER_API_KEY` |
| Synthetic | `synthetic/hf:moonshotai/Kimi-K3` | `SYNTHETIC_API_KEY` |
| Kimi (Moonshot API) | `kimi/kimi-k3.1-pro` | `MOONSHOT_API_KEY` |
| Kimi Code plan | `kimi-code/kimi-for-coding` | `KIMI_API_KEY` |
| Z.ai | `zai/glm-5.3` | `ZHIPU_API_KEY` |
| Z.ai GLM Coding plan | `zai-coding/glm-5.3` | `ZHIPU_API_KEY` |
| MiniMax | `minimax/MiniMax-M3` | `MINIMAX_API_KEY` |
| MiniMax Coding plan | `minimax-coding/MiniMax-M3` | `MINIMAX_API_KEY` |
| Ollama (local) | `ollama/qwen3-coder:30b` | none — detected at `localhost:11434` (or `OLLAMA_HOST`) |
| LM Studio (local) | `lmstudio/openai/gpt-oss-20b` | none — detected at `localhost:1234` |

Local servers are picked up whenever they're running; their models come from the server, filtered to
chat models with tool calling, with real context sizes. Start Ollama with `OLLAMA_CONTEXT_LENGTH=32768`
(or more) — its default context is too small for agent work.

`/auth gemini` (or any provider name) jumps straight to pasting that key. `OAH_<PROVIDER>_BASE_URL`
(e.g. `OAH_ZAI_CODING_BASE_URL`) points a provider at a proxy or gateway.

`/model` lists tool-calling models from models.dev for the providers you have credentials for; effort levels
come from each model's `reasoning_options`.

- **ChatGPT plan** follows OpenAI's open-source Sign in with ChatGPT flow: a stable `ext_agent_host_id`,
  dynamic client registration on first sign-in (the issued client id is reused), PKCE + state + nonce on a
  127.0.0.1 loopback, ID-token verification against OpenAI's JWKS, the `chatgpt.tokens.use.direct` scope
  check, automatic refresh, and revocation on sign-out. Requests go to `/v1/responses` with `store: false`,
  and replay encrypted reasoning.
- **Gemini, xAI, DeepSeek, OpenRouter** use their AI SDK providers; **Synthetic, Kimi, Z.ai** the
  OpenAI-compatible chat API; **MiniMax** its Anthropic-compatible API. Each gets reasoning effort in its own
  format, limited to the levels models.dev lists for the model.
- **Anthropic** uses adaptive thinking with summarized display, `effort` (low → max), server-side refusal
  fallbacks on models that support them, and prompt caching.

## Tools

| Tool | What it does | Permission in `auto` |
| --- | --- | --- |
| `read_file` | numbered lines, offset/limit | runs (outside the workspace: safety monitor) |
| `ls` | list a directory; `all` for dotfiles, `long` for sizes/dates, `depth` 1–3 | runs |
| `find` | find files/dirs by name or path glob, type, depth, size, modified age; .gitignore-aware | runs |
| `grep` | search contents (ripgrep when installed, built-in fallback): regex or literal, case-insensitive, file glob, context lines, `content` / `files` / `count` output | runs |
| `write_file` | create/overwrite a file *(build mode)* | runs in the workspace (undo with `/undo`); outside: safety monitor |
| `edit_file` | exact string replace — one edit, or several in `edits`, all-or-nothing *(build mode)* | runs in the workspace; outside: safety monitor |
| `apply_patch` | add/update/move/delete several files in one all-or-nothing patch *(build mode, GPT models)* | runs in the workspace; outside: safety monitor |
| `bash` | run a shell command (read-only in plan mode); `background: true` for servers and watchers | safe commands run; others: safety monitor |
| `bash_output` / `bash_kill` | read new output from / stop a background shell (`⧗ N bg` in the status bar) | runs |
| `task` | delegate research to a read-only sub-agent with a fresh context; live progress, runs in parallel | runs (its own tools follow your permissions) |
| `ask_user` | ask you a question with options, or take your own typed answer — oah asks instead of assuming | runs |
| `web_fetch` | fetch a URL as text | safety monitor |
| `todo_write` | the live task list | runs |
| `memory` | add/remove/list persistent notes | runs |
| `load_skill` | load a skill's instructions | runs |
| `goal_complete` | finish an active `/goal` | runs |
| `oah_setup` | oah configuring itself: add/remove MCP servers, sign in to them, install/remove skills (GitHub `owner/repo`, URL or path), status | changes: safety monitor; status and sign-in run |
| MCP tools | `mcp__<server>__<tool>` from your MCP servers | safety monitor |

When oah asks a question: **↑↓** or **1–9** to pick (space ticks in multi-select), **enter** to answer, or type
your own answer in the composer; **esc** dismisses. Background shells are stopped on `/clear` and on exit.

At a prompt: **y** allow once · **a** always allow (this session: per command, per MCP tool, or all edits) ·
**n**/Esc deny. `always ask` asks for everything; `always allow` never asks.

### Auto mode

`auto` lets oah carry out multi-step work on its own while a **safety monitor** reviews every action that
isn't obviously safe. Reading, editing files in the project (reversible with `/undo`) and safe commands just
run. Everything else — other commands, anything outside the project, the network, MCP tools, oah's own
config — is checked first by a quick model call that sees what you asked for:

- **Allowed:** normal steps toward your request (build, test, install project deps, local git, reading docs).
- **Blocked:** destructive or irreversible actions, work outside the project's scope, exposing secrets (reading
  keys, sending env vars anywhere), deploying/publishing/pushing, dropping databases, running downloaded
  scripts. oah gets the reason and finds a safer way, or asks you with `ask_user` — once you explicitly
  approve, the monitor lets it through.
- **Your call:** catastrophic commands (wiping `/` or `~`, overwriting disks), a third block in one turn, or
  the monitor being unavailable stop with an approval card showing why.

The monitor uses a small model from your current provider (Haiku, a mini or flash model) and falls back to
your current model if there isn't one or it isn't on your plan; set `safetyModel` in `settings.json` to choose.

## Memory, skills, MCP

- **Instructions:** `~/.config/oah/AGENTS.md`, then from the repo root down to the working directory the
  first of `AGENTS.md`, `OAH.md`, `CLAUDE.md`. `/init` writes one.
- **Notes:** `#note`, `/memory add|global|remove`, or the model's `memory` tool.
- **Skills:** folders with a `SKILL.md` in `./.oah/skills`, `./.agents/skills`, `./.claude/skills`,
  `~/.config/oah/skills`, `~/.agents/skills`, `~/.claude/skills`.
- **Setting up:** just ask — "add the Linear MCP server", "connect https://mcp.example.com/mcp", "install the
  shadcn skill" — and oah does it with `oah_setup` (finding the official URL or repo if needed). Or `/import`.
- **MCP:** `~/.config/oah/mcp.json`, `./.oah/mcp.json` or `./.mcp.json` (stdio, HTTP and SSE, via `@ai-sdk/mcp`;
  OAuth for remote servers; `${VAR}` / `${VAR:-default}` are expanded):

  ```json
  { "mcpServers": {
      "fs":   { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "."] },
      "docs": { "url": "https://example.com/mcp", "headers": { "Authorization": "Bearer …" } } } }
  ```

## Files

Everything lives in `~/.config/oah` (`$XDG_CONFIG_HOME/oah`, or `$OAH_HOME`):

```text
bin/oah              the binary (installed by install.sh)
settings.json        model, effort, permission, mode, autoCompactAt, allowedCommands, safetyModel
auth.json            credentials (0600)    host_id   ext_agent_host_id for ChatGPT
sessions/<id>.json   conversations         history.jsonl  prompt history
memory/  skills/  mcp.json  mcp-auth.json (MCP OAuth tokens, 0600)  AGENTS.md  models.json (models.dev cache)
```

## Install & release

```sh
curl -fsSL https://oah.dev/install.sh | sh                     # latest
curl -fsSL https://oah.dev/install.sh | OAH_VERSION=0.1.0 sh   # pinned
```

Windows (PowerShell):

```powershell
irm https://oah.dev/install.ps1 | iex
```

`scripts/install.sh` detects the platform (Linux/macOS, x64/arm64), downloads
`$OAH_DOWNLOAD_URL/v<version>/oah-<os>-<arch>.tar.gz`, verifies its `.sha256`, installs to
`~/.config/oah/bin/oah` and adds that to your shell's PATH. `scripts/install.ps1` does the same on Windows
(x64/arm64): `~\.config\oah\bin\oah.exe`, added to your user PATH. On Windows the shell tool runs
PowerShell (`pwsh` when installed, else Windows PowerShell), and the model is told to write PowerShell.

Hosting on Cloudflare: serve `scripts/install.sh` at `https://oah.dev/install.sh` and put the release files
under `https://oah.dev/download/`:

```text
download/latest.txt                         0.1.0
download/v0.1.0/oah-linux-x64.tar.gz        + .sha256
download/v0.1.0/oah-linux-arm64.tar.gz      + .sha256
download/v0.1.0/oah-darwin-x64.tar.gz       + .sha256
download/v0.1.0/oah-darwin-arm64.tar.gz     + .sha256
download/v0.1.0/oah-windows-x64.tar.gz      + .sha256   (oah.exe)
download/v0.1.0/oah-windows-arm64.tar.gz    + .sha256
```

and `scripts/install.ps1` at `https://oah.dev/install.ps1`. `bun scripts/build.ts --os darwin,windows` builds
just those platforms.

Pushing a `v*` tag runs `.github/workflows/release.yml`, which builds all six with `bun scripts/build.ts --all`
and attaches them (plus `latest.txt`, `install.sh` and `install.ps1`) to a GitHub release. Until oah.dev is live, install
from GitHub with `OAH_DOWNLOAD_URL=https://github.com/dawsja/oah/releases/download`.

## Develop

```sh
git clone --recursive https://github.com/dawsja/oah && cd oah
bun install
bun run dev            # run from source
bun test && bun run typecheck   # tests live in test/ (bunfig.toml)
bun run build          # dist/oah-<os>-<arch>.tar.gz for this machine
```

Each binary is the Bun runtime plus oah plus OpenTUI's prebuilt native library (compiled from Zig), so
users need nothing else installed.

### AI SDK skills

The repo ships the AI SDK's agent skills (installed with `npx skills add vercel/ai`, pinned in
`skills-lock.json`): `ai-sdk` and `migrate-ai-sdk-v6-to-v7` in `.agents/skills/`, symlinked into
`.claude/skills/`. Coding agents working on this repo — including oah itself, which reads both folders — use
them to check AI SDK APIs against the version-matched docs in `node_modules/ai/docs`. Update with
`npx skills update`.

### Rust wrapper

`crates/opentui` is a standalone Rust binding to OpenTUI's native C ABI (`opentui.h`, v0.8.2): host-owned
raw mode and event loop, immediate-mode drawing, Kitty keyboard / SGR mouse / paste parsing, OSC 52. It
builds the Zig core from the `vendor/opentui` submodule (Zig 0.17), or links an existing library with
`OPENTUI_LIB_DIR`:

```sh
cargo test
cargo run --example hello
```

## License

MIT
