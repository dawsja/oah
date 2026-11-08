# oah — Open Agent Harness

A chat-first desktop app for coding agents. oah drives **Claude Code**, **Codex**, **OpenCode**
and any other agent that speaks the [Agent Client Protocol](https://agentclientprotocol.com)
(the CLIs you already use, with your own sign-ins) in your projects: on this computer, in WSL
or on an SSH host, with git, worktrees and pull requests next to the chat. Rust and Tauri; no
Electron.

![oah](assets/social/oah-readme-banner-dark.png)

## What's in it

- **Chat.** Streaming answers, folded tool runs, file diffs, approvals (allow, always allow,
  deny; queued when several arrive), questions the agent asks (options, multi-select, typed
  answers, one question at a time with number keys), plans to approve or send back, a context ring
  by the send button (click it for what fills the window: messages, tools, skills, system prompt,
  free space; Claude Code reports the breakdown, the others the total), token use per turn, a model picker with each agent's own models and their effort levels (hover a
  model: Low … Max open to the right), `@` file mentions, and a `/` menu: oah's `/model`, `/plan`, `/compact` and `/clear`, then the agent's own
  commands and skills. "Always allow" lasts for the chat's session; it never writes the agent's
  settings files. Messages sent mid-turn are steered in (Codex) or queued (Claude, ACP). Chats
  are saved and resume the agent's own session.
- **Images.** Attach by paste, drop or the clip button; they're saved with the chat, shown as
  thumbnails in the message, and open full-window (fit or actual size, copy, save; ← → between
  several). Images the agent reads show beside its step, images in answers open the same way,
  and the file viewer shows image files.
- **Projects and worktrees.** Each project lists its worktrees and the chats in each one. A new
  worktree is a branch in its own folder, so several chats can work on one repo at once.
- **Side panel.** Files (tree, filter, search in files, add to chat), Source Control (stage,
  discard, commit, branches, push, pull, sync; the sparkle in the message box writes the commit
  message from the changes with the project agent's cheapest model: Haiku, Codex's fast model,
  or a free OpenCode one), History (graph), Pull Request (`gh`: open one,
  review state, checks).
- **Terminal.** A panel under the chat (Ctrl+` or the shell button in the title bar shows and
  hides it) with a shell where the project is: PowerShell (7 if installed) on Windows, your
  `$SHELL` on Linux, your login shell inside the distro for a WSL project, and over `ssh -t` for
  an SSH project, so it's already connected. Each tab says which shell it is (PowerShell,
  bash, zsh…) and where it runs. Terminals open in the chat's worktree, several per project in
  tabs, and keep running with their scrollback while the panel is hidden or another project is
  open; `exit` closes one, a failed or dropped one waits for Enter to start again. Ctrl+C copies
  a selection (Ctrl+Shift+C always), Ctrl+V pastes.
- **WSL.** On Windows, a project can live in a WSL distro. oah runs the agent CLI, git, gh and
  file reads inside the distro (`wsl.exe -d <distro> -e …`), so the agent works on Linux files
  with Linux tools; a Windows copy of a CLI that WSL puts on PATH is never used. Add one from
  the open-project menu and browse the distro's own filesystem (`/`, `/home`, `/etc`…); a
  `\\wsl.localhost\…` folder picked in Windows becomes a WSL project, and a Windows drive
  (`/mnt/c/…`) is offered as a project on this computer instead. Settings → Projects lists the
  distros with what's installed and signed in there; Usage shows a distro's own plan limits.
  A stopped distro is left asleep until you open one of its projects.
- **SSH hosts.** Hosts from `~/.ssh/config` (with `Include`) or added by hand. A remote project
  runs the agent CLI, git and gh over `ssh`, so the agent works on that machine's files.
- **Voice (optional).** Off until you turn it on in Settings → Voice, which asks before the
  one-time 731 MB download of NVIDIA Parakeet. Dictation is then transcribed on this computer;
  nothing you say leaves the machine. The mic appears in the message box only while voice is on,
  and turning it off deletes the model.
- **Usage.** Plan limits for Claude, Codex and OpenCode Go (session, week, month), in a title bar
  popover and in Settings → Usage, with tokens and cost per project.
- **MCP (optional).** Another agent (Claude Code, Codex, any MCP client) can drive oah: list
  projects, the agents and their models with effort levels, start chats (in a worktree too), send
  messages and wait for the turn, read transcripts and diffs, stop turns, and answer approvals,
  plans and questions. Its chats show in the sidebar as they run, and whoever answers an approval
  first, there or in oah, wins. Off until you turn it on in Settings → MCP, which also has the
  config to paste: clients run `oah mcp`, which talks to the open app over a local socket
  (a named pipe on Windows) that only you can open. Full access and deleting chats are separate
  switches, both off.
- **Phone (Android, optional).** The oah app for Android continues your chats away from the
  desk: projects and chats, answers streaming in, replies (queued or sent into the running
  turn), photos, approvals and questions, stop, new chats, and the model, effort and permission.
  Turn it on in Settings → Phone, then choose the computer under Nearby computers on the
  phone and tap Connect. Accept the desktop's Connect prompt and enter its 8-character code.
  Manual address/code pairing is also available. Nearby discovery uses UDP port 7788; pairing
  and chat use TCP/WebSocket on the configured port (7787 by default). On Windows, the
  Allow connections button requests administrator permission to allow oah on Private networks
  from the local subnet. Discovery cannot cross guest-network isolation or blocked firewalls.
  The phone connects directly over the same local network (Ethernet and Wi-Fi work together)
  or a VPN such as Tailscale; pairing runs SPAKE2 on the one-time code (five
  tries, five minutes), and every session after it is Noise `IK` with keys only the two of them
  hold. Settings → Phone lists the paired phones; Forget disconnects one for good.
- **Settings.** Providers, Voice, Appearance (dark, light, system), Usage, SSH Hosts, Projects, MCP,
  Phone.

Corners are subtle: 2px tags, 4px controls and rows, 6px cards and panels.

## Layout

```
crates/
  oah-core       data folder (~/.config/oah/app, or $OAH_HOME/app), settings.json, chats on disk
  oah-transport  runs a program in a folder: locally, over ssh, or in a WSL distro; distros,
                 host checks, remote folders; terminals (a shell in a pty, wherever the project is)
  oah-agents     Claude Code (stream-json), Codex (app-server) and ACP (OpenCode, added agents)
                 drivers → ChatEvent; sign-in, models and plan limits; a scripted mock
                 (OAH_MOCK_AGENT=1)
  oah-git        git and gh: status, diffs, commits, branches, sync, graph, worktrees, PRs
  oah-voice      speech to text (transcribe.cpp; needs CMake to build)
  oah-mcp        the MCP server another agent drives oah's chats with (tools, live chat status,
                 transcripts), and `oah mcp`, the stdio bridge to the running app
  oah-remote     phones: pairing (SPAKE2 on a one-time code), encrypted sessions
                 (Noise IK over WebSocket), the desktop's server and the phone's client
app/
  src-tauri      the Tauri shell: commands, chat-event / voice-state events, terminal channels;
                 remote.rs answers phones with the same chats
  src            React + Tailwind (shadcn primitives, 6px radius scale)
  src/mobile     the phone app's UI (mobile.html, built to dist-mobile/), sharing the chat
                 components and `reduce` with the desktop
android/
  src-tauri      the Android app (Tauri 2): the phone's key, paired desktops, the session that
                 reconnects by itself; gen/android is the Gradle project
```

## Develop

Needs Rust, Bun, CMake and a C++ compiler; on Linux the WebKitGTK packages Tauri lists.

```sh
cd app
bun install
bun run dev              # the app
bun run dev:debug        # the app with DevTools on 127.0.0.1:9223, data in ../.dev-home
```

`scripts/dev-cdp.ts` drives a `dev:debug` window without touching your mouse or keyboard
(`shot`, `eval`, `click`, `type`, `key`). `OAH_MOCK_AGENT=1` swaps both agents for a scripted one.

The phone app needs a JDK 17, the Android SDK and NDK, and the Rust Android targets
(`rustup target add aarch64-linux-android armv7-linux-androideabi`):

```sh
cd android
bun install
bun tauri android dev                # on a connected phone or emulator
bun tauri android build --apk        # signed when gen/android/keystore.properties exists
```

`cd app && bun run mobile:web` serves the phone UI alone (localhost:1422/mobile.html).

Checks, as CI runs them:

```sh
(cd app && bun run web:build && bun run mobile:build && bun run test)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Credits

Inspired by [Orca](https://github.com/stablyai/orca). Voice comes from Gond Code. See
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). MIT licensed.
