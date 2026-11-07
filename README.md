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
- **Voice.** Dictation transcribed on this computer with NVIDIA Parakeet (a one-time 731 MB
  download). Nothing you say leaves the machine.
- **Usage.** Plan limits for Claude, Codex and OpenCode Go (session, week, month), in a title bar
  popover and in Settings → Usage, with tokens and cost per project.
- **Settings.** Providers, Voice, Appearance (dark, light, system), Usage, SSH Hosts, Projects.

Corners are subtle: 2px tags, 4px controls and rows, 6px cards and panels.

## Layout

```
crates/
  oah-core       data folder (~/.config/oah/app, or $OAH_HOME/app), settings.json, chats on disk
  oah-transport  runs a program in a folder: locally, over ssh, or in a WSL distro; distros,
                 host checks, remote folders
  oah-agents     Claude Code (stream-json), Codex (app-server) and ACP (OpenCode, added agents)
                 drivers → ChatEvent; sign-in, models and plan limits; a scripted mock
                 (OAH_MOCK_AGENT=1)
  oah-git        git and gh: status, diffs, commits, branches, sync, graph, worktrees, PRs
  oah-voice      speech to text (transcribe.cpp; needs CMake to build)
app/
  src-tauri      the Tauri shell: commands, chat-event / voice-state events
  src            React + Tailwind (shadcn primitives, 6px radius scale)
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

Checks, as CI runs them:

```sh
(cd app && bun run web:build && bun run test)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Credits

Inspired by [Orca](https://github.com/stablyai/orca). Voice comes from Gond Code. See
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). MIT licensed.
