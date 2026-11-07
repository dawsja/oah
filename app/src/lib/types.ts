// Mirrors crates/oah-core/src/settings.rs (serde camelCase).

export type AgentKind = "claude" | "codex"
export const AGENTS: AgentKind[] = ["claude", "codex"]
export const AGENT_LABEL: Record<AgentKind, string> = { claude: "Claude Code", codex: "Codex" }

export type PermissionMode = "ask" | "acceptEdits" | "plan" | "fullAccess"
export const PERMISSION_LABEL: Record<PermissionMode, string> = {
  ask: "Ask",
  acceptEdits: "Accept edits",
  plan: "Plan only",
  fullAccess: "Full access",
}

export type EnvVar = { key: string; value: string }

export type Provider = {
  enabled: boolean
  binary: string
  defaultModel: string
  permissionMode: PermissionMode
  env: EnvVar[]
}

export type Providers = { claude: Provider; codex: Provider; defaultAgent: AgentKind }

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

export type AppInfo = { version: string; platform: "windows" | "linux" | "macos" | "web"; dataDir: string; home: string }

export function locationLabel(l: Location): string {
  return l.kind === "local" ? "This computer" : l.host
}

export type Probe = { found: boolean; path: string | null; version: string | null; error: string | null }
