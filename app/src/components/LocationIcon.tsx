import { Folder, Server } from "lucide-react"
import { WslLogo } from "@/components/icons"
import type { Location } from "@/lib/types"
import { cn } from "@/lib/utils"

/** Where a project lives: a folder here, an SSH host, or a WSL distro. */
export function LocationIcon({ location, className }: { location: Location; className?: string }) {
  if (location.kind === "wsl") return <WslLogo className={cn("size-3.5 shrink-0", className)} />
  const Icon = location.kind === "ssh" ? Server : Folder
  const tone = location.kind === "ssh" ? "text-cyan" : "text-muted-foreground"
  return <Icon className={cn("size-3.5 shrink-0", tone, className)} />
}
