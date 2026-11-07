// The permission modes' icons and colours, shared by every agent (Claude Code, Codex, OpenCode, added ACP agents).
import { FilePen, Hand, ListChecks, ShieldOff, type LucideIcon } from "lucide-react"
import type { PermissionMode } from "@/lib/types"

export const PERMISSION_ICON: Record<PermissionMode, LucideIcon> = {
  ask: Hand,
  acceptEdits: FilePen,
  plan: ListChecks,
  fullAccess: ShieldOff,
}

/** Text colour for a mode; Accept edits has none (neutral). */
export const PERMISSION_TONE: Record<PermissionMode, string> = {
  ask: "text-green",
  acceptEdits: "",
  plan: "text-cyan",
  fullAccess: "text-red",
}
