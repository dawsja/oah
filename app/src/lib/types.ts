// Mirrors crates/oah-core/src/settings.rs (serde camelCase).

/** `opencode2`: OpenCode 2 by its installer's `opencode2` command (the same CLI and accounts).
 *  `antigravity`: Google Antigravity's CLI, `agy`. */
export type AgentKind = "claude" | "codex" | "opencode" | "opencode2" | "antigravity"
/** The agents oah drives. */
export const AGENTS: AgentKind[] = ["claude", "codex", "opencode", "opencode2", "antigravity"]
export const AGENT_LABEL: Record<AgentKind, string> = { claude: "Claude Code", codex: "Codex", opencode: "OpenCode", opencode2: "OpenCode 2", antigravity: "Antigravity" }
/** OpenCode by either command: driven the same way. */
export const isOpencode = (k: AgentKind | undefined) => k === "opencode" || k === "opencode2"
/** Driven over ACP (OpenCode, Antigravity): one prompt at a time. */
export const isAcp = (k: AgentKind | undefined) => isOpencode(k) || k === "antigravity"
/** Agents whose plan limits oah can read (OpenCode 2's are OpenCode's: one Go account). */
export const LIMIT_AGENTS: AgentKind[] = ["claude", "codex", "opencode", "antigravity"]
/** Whose limits they are (OpenCode's are its Go plan's). */
export const LIMIT_LABEL: Record<AgentKind, string> = { claude: "Claude", codex: "Codex", opencode: "OpenCode Go", opencode2: "OpenCode Go", antigravity: "Antigravity" }

export type PermissionMode = "ask" | "acceptEdits" | "plan" | "fullAccess"
export const PERMISSION_LABEL: Record<PermissionMode, string> = {
  ask: "Ask",
  acceptEdits: "Accept edits",
  plan: "Plan only",
  fullAccess: "Full access",
}

/** `value` is empty once it is saved in the OS keychain (`inKeychain`). */
export type EnvVar = { key: string; value: string; inKeychain: boolean }

export type Provider = {
  enabled: boolean
  binary: string
  defaultModel: string
  permissionMode: PermissionMode
  env: EnvVar[]
}

export type Providers = { claude: Provider; codex: Provider; opencode: Provider; opencode2: Provider; antigravity: Provider; defaultAgent: AgentKind }

export type Voice = { enabled: boolean; microphone: string; holdKey: string; silenceStopSecs: number }

export type Theme = "system" | "dark" | "light"
export type Appearance = {
  theme: Theme
  /** The window's zoom, as the base text size: 13 is 100%. */
  uiFontSize: number
  chatFontSize: number
  sansFont: string
  monoFont: string
}

/** The interface size that's 100% zoom. */
export const BASE_UI_SIZE = 13

export type SshHost = { host: string; port: number; identityFile: string; label: string }

export type Location = { kind: "local" } | { kind: "ssh"; host: string } | { kind: "wsl"; distro: string }

/** An installed WSL distro. */
export type Distro = { name: string; default: boolean; running: boolean; version: number }

/** The project's files are on another system (an SSH host, a WSL distro). */
export const isRemote = (l: Location) => l.kind !== "local"

/** `/mnt/c/Users/u` (a Windows drive as WSL mounts it) → `C:\Users\u`; null for Linux paths. */
export function windowsPath(linux: string): string | null {
  const m = /^\/mnt\/([a-z])(\/.*)?$/.exec(linux)
  return m ? `${m[1].toUpperCase()}:\\${(m[2] ?? "").replace(/^\//, "").replace(/\//g, "\\")}` : null
}

export type ProjectGit = { baseBranch: string; worktreeDir: string; deleteBranchOnRemove: boolean }

export type Project = {
  id: string
  name: string
  location: Location
  path: string
  defaultAgent: AgentKind | null
  defaultModel: string
  git: ProjectGit
}

/** A phone paired with this desktop (the oah Android app). */
export type PairedDevice = { id: string; name: string; publicKey: string; pairedAt: number; lastSeen: number }
/** Phones controlling this app. Changed only through `remote_*` commands, never `settings_set`. */
export type Remote = { enabled: boolean; port: number; devices: PairedDevice[] }

export type Settings = {
  providers: Providers
  voice: Voice
  appearance: Appearance
  sshHosts: SshHost[]
  projects: Project[]
  agentSleepMinutes: number
  mcp: Mcp
  remote: Remote
}

/** Another agent driving oah's chats over MCP (`oah mcp`). */
export type Mcp = { enabled: boolean; allowFullAccess: boolean; allowDelete: boolean }

/** Where MCP listens, and who's connected. */
export type McpStatus = { socket: string; clients: number; exe: string }

export type AppInfo = { version: string; platform: "windows" | "linux" | "macos" | "web"; dataDir: string; home: string; bundle: string | null }

export function locationLabel(l: Location): string {
  return l.kind === "local" ? "This computer" : l.kind === "ssh" ? l.host : l.distro
}

export type Probe = { found: boolean; path: string | null; version: string | null; error: string | null; outdated: string | null }

export type Account = { signedIn: boolean; label: string | null; plan: string | null; error: string | null }
export type LimitWindow = { label: string; usedPercent: number; resetsAt: number | null }
/** `extra`: rows that aren't windows (a balance; `value` null when unavailable, `note` says why). */
export type LimitExtra = { label: string; value: string | null; note: string | null }
/** `missing`: the agent's CLI isn't installed where these were asked for. */
export type Limits = { windows: LimitWindow[]; error: string | null; extra: LimitExtra[]; missing?: boolean }

/** `windows`: not installed there, but a Windows copy is on WSL's PATH (where). */
export type Tool = { path: string | null; version: string | null; windows: string | null }
export type HostCheck = { ok: boolean; error: string | null; home: string | null; os: string | null; git: Tool; gh: Tool; claude: Tool; codex: Tool; opencode: Tool; opencode2: Tool; antigravity: Tool }
