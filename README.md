# oah — Open Agent Harness

A chat-first desktop app for coding agents. oah drives **Claude Code** and **Codex** (the CLIs
you already use, with your own sign-ins) in your projects, on this computer or on an SSH host,
with git, worktrees and pull requests next to the chat. Rust and Tauri; no Electron.

![oah](assets/social/oah-readme-banner-dark.png)

## What's in it

- **Chat.** Streaming answers, folded tool runs, file diffs, approvals (allow, always allow,
  deny), the agent's plan, context and token use per turn, image attachments. Chats are saved
  and resume the agent's own session.
- **Projects and worktrees.** Each project lists its worktrees and the chats in each one. A new
  worktree is a branch in its own folder, so several chats can work on one repo at once.
- **Side panel.** Files (tree, filter, search in files, add to chat), Source Control (stage,
  discard, commit, branches, push, pull, sync), History (graph), Pull Request (`gh`: open one,
  review state, checks).
- **SSH hosts.** Hosts from `~/.ssh/config` (with `Include`) or added by hand. A remote project
  runs the agent CLI, git and gh over `ssh`, so the agent works on that machine's files.
- **Voice.** Dictation transcribed on this computer with NVIDIA Parakeet (a one-time 731 MB
  download). Nothing you say leaves the machine.
- **Settings.** Providers, Voice, Appearance (dark, light, system), Usage (Claude and Codex plan
  limits, tokens and cost per project), SSH Hosts, Projects.

Corners are subtle: 2px tags, 4px controls and rows, 6px cards and panels.

## Layout

```
crates/
  oah-core       data folder (~/.config/oah/app, or $OAH_HOME/app), settings.json, chats on disk
  oah-transport  runs a program in a folder, locally or over ssh; host checks, remote folders
  oah-agents     Claude Code (stream-json) and Codex (app-server) drivers → ChatEvent; sign-in
                 and plan limits; a scripted mock (OAH_MOCK_AGENT=1)
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

The interface follows [Orca](https://github.com/stablyai/orca) and both themes
follow [T3 Code](https://github.com/pingdotgg/t3code); voice comes from Gond Code. See
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). MIT licensed.
