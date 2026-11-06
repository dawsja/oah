# oah desktop

The oah coding agent with a desktop interface, for Windows and Linux. It
runs the **same engine as the `oah` CLI** — same agent, tools, commands, pickers,
permissions, checkpoints, MCP and skills — and shares its data folder, so you can
start a session in one and continue it in the other.

```
~/.config/oah                 Linux
%USERPROFILE%\.config\oah     Windows
```

(`$OAH_HOME` and `$XDG_CONFIG_HOME` are honored, exactly as by the CLI.)

## How it fits together

```
React UI (shadcn/ui, Tailwind)   ── Tauri IPC ──   Rust shell (src-tauri, core)
  sessions sidebar, chat,                            window, session index,
  pickers, settings                                  desktop.json, engine process
                                                           │ JSON lines (stdio)
                                                     oah serve  (../src/serve.ts)
                                                       the CLI's Store: agent,
                                                       tools, sessions, commands
```

- **`../src/serve.ts`** — `oah serve`: the engine without the terminal UI. It
  streams state snapshots and transcript diffs and takes ops (submit, interrupt,
  pick, permission answers…). Everything typed in the composer goes through the
  same `Store.submit` as the TUI, so every `/command` works.
- **`core/`** — no GUI dependencies: where the data lives (`paths`), the session
  list for the sidebar (`sessions`, using the engine's `sessions/meta` summaries),
  `desktop.json` preferences (`prefs`) and starting/stopping the engine (`engine`).
- **`src-tauri/`** — the Tauri 2 app: window, commands the UI calls, capabilities.
- **`src/`** — the UI. `lib/protocol.ts` imports the protocol types from the
  engine's source, so the two can't drift.

Which engine runs: `$OAH_ENGINE`, else the one bundled with the app
(`oah-engine`), else (debug builds) this repo's source with Bun, else the
installed CLI (`~/.config/oah/bin/oah`, then `oah` on PATH). On Linux the
engine gets your login shell's `PATH`, so its tools find what a terminal would.

## Develop

Needs Bun and Rust. On Linux also WebKitGTK:

```sh
sudo apt install build-essential pkg-config libwebkit2gtk-4.1-dev libssl-dev librsvg2-dev libxdo-dev libayatana-appindicator3-dev
```

```sh
bun install            # in the repo root and in desktop/
bun run dev            # the app, engine from ../src (hot reload for the UI)
bun run typecheck
cargo test -p oah-desktop-core   # includes a round trip through the real engine
```

UI only, in a browser (no Rust needed): `bun run web:bridge` in one terminal and
`bun run web` in another, then open http://localhost:1420. The bridge runs the
engine from source; set `OAH_HOME` to try it on a scratch config.

## Install

```sh
curl -fsSL https://dl.oah.dev/install-desktop.sh | sh       # Linux
irm https://dl.oah.dev/install-desktop.ps1 | iex            # Windows (PowerShell)
```

Every download is listed in https://dl.oah.dev/desktop/latest.json (`files`).

## Updates

The app updates itself in the background (Settings → Updates & engine, on by
default). It polls `https://dl.oah.dev/desktop/latest.json`; each install finds
its own signed update there under `{os}-{arch}-{installer}` (e.g.
`linux-x86_64-deb`, `windows-x86_64-nsis`), falling back to `{os}-{arch}`.
AppImage installs update in place silently; Windows
and deb/rpm installs download silently and install on "Restart to update".

Updates are signed with the key whose public half is in `tauri.conf.json`
(`plugins.updater.pubkey`); the private key is the `TAURI_SIGNING_PRIVATE_KEY`
repo secret. Keep a backup of it — without it, installed apps can't be updated.

## Release

Bump `version` in `desktop/package.json` (the app reads it from there), commit,
then push a tag:

```sh
git tag desktop-v0.1.2 && git push origin desktop-v0.1.2
```

`.github/workflows/desktop.yml` builds every platform, signs the updates, uploads
to R2 (`scripts/publish-desktop-r2.sh`: `desktop/v<version>/…`, then
`desktop/latest.json` last) and makes a GitHub release. A version is published
once; installed apps pick it up within a few hours.

## Build installers locally

```sh
bun run build          # compiles the engine for this machine, then tauri build
```

Output lands in `target/release/bundle/` (`.deb`/`.rpm`/`.AppImage`, `-setup.exe`).
CI builds both platforms: see `.github/workflows/desktop.yml` (push a
`desktop-v*` tag, or run the workflow by hand).

Icons are generated from `../assets/png/icon/oah-icon-dark-1024.png`
(`bun run icons`).

## Keys

Same as the TUI where it makes sense: Enter sends, Shift+Enter newline,
Ctrl+Enter sends now mid-turn, Esc interrupts, Tab plan ↔ build, Shift+Tab
cycles permissions, ↑/↓ history, Ctrl+T model, Ctrl+End jump to bottom.
Desktop extras: Ctrl+N new session, Ctrl+O open a folder, Ctrl+B sidebar,
Ctrl+I details panel, Ctrl+, settings.
