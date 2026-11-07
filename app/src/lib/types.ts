// Mirrors crates/oah-core/src/settings.rs (serde camelCase).

export type AgentKind = "claude" | "codex" | "opencode" | "acp"
/** The built-in agents (an added ACP agent is `acp` plus its id). */
export const AGENTS: Exclude<AgentKind, "acp">[] = ["claude", "codex", "opencode"]
export const AGENT_LABEL: Record<AgentKind, string> = { claude: "Claude Code", codex: "Codex", opencode: "OpenCode", acp: "ACP agent" }
/** Agents whose plan limits oah can read. */
export const LIMIT_AGENTS: AgentKind[] = ["claude", "codex"]

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

/** An agent the user added that speaks ACP: `binary` is its command, `args` follow it. */
export type AcpAgent = Provider & { id: string; name: string; args: string[] }

export type Providers = { claude: Provider; codex: Provider; opencode: Provider; acp: AcpAgent[]; defaultAgent: AgentKind }

export type Voice = { enabled: boolean; microphone: string; holdKey: string; silenceStopSecs: number }

export type Theme = "system" | "dark" | "light"
export type Density = "compact" | "comfortable"
export type Appearance = {
  theme: Theme
  uiFontSize: number
  chatFontSize: number
  sansFont: string
  monoFont: string
  density: Density
}

export type SshHost = { host: string; port: number; identityFile: string; label: string }

export type Location = { kind: "local" } | { kind: "ssh"; host: string }

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

export type Settings = {
  providers: Providers
  voice: Voice
  appearance: Appearance
  sshHosts: SshHost[]
  projects: Project[]
}

export type AppInfo = { version: string; platform: "windows" | "linux" | "macos" | "web"; dataDir: string; home: string; bundle: string | null }

export function locationLabel(l: Location): string {
  return l.kind === "local" ? "This computer" : l.host
}

export type Probe = { found: boolean; path: string | null; version: string | null; error: string | null; outdated: string | null }

export type Account = { signedIn: boolean; label: string | null; plan: string | null; error: string | null }
export type LimitWindow = { label: string; usedPercent: number; resetsAt: number | null }
export type Limits = { windows: LimitWindow[]; error: string | null }

export type Tool = { path: string | null; version: string | null }
export type HostCheck = { ok: boolean; error: string | null; home: string | null; os: string | null; git: Tool; gh: Tool; claude: Tool; codex: Tool }
