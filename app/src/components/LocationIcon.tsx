import { Folder, Server, SquareTerminal } from "lucide-react"
import type { Location } from "@/lib/types"
import { cn } from "@/lib/utils"

/** Where a project lives: a folder here, an SSH host, or a WSL distro. */
export function LocationIcon({ location, className }: { location: Location; className?: string }) {
  const Icon = location.kind === "ssh" ? Server : location.kind === "wsl" ? SquareTerminal : Folder
  const tone = location.kind === "ssh" ? "text-cyan" : location.kind === "wsl" ? "text-green" : "text-muted-foreground"
  return <Icon className={cn("size-3.5 shrink-0", tone, className)} />
}
