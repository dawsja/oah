import { useEffect, useState, type ReactNode } from "react"
import { Folder, Server } from "lucide-react"
import { toast } from "sonner"
import { confirmAction } from "@/components/Prompt"
import { pickRemoteFolder } from "@/components/settings/RemoteFolder"
import { LocationIcon } from "@/components/LocationIcon"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { ipc, pickFolder } from "@/lib/ipc"
import { windowsPath, type Distro } from "@/lib/types"
import { useApp } from "@/store/app"

/** The WSL distros, asked once per session (re-asked on `reload`). */
let distroList: Promise<Distro[]> | null = null
export function useDistros() {
  const [distros, setDistros] = useState<Distro[] | null>(null)
  const load = (fresh = false) => {
    if (fresh || !distroList) distroList = ipc.wslDistros().catch(() => [])
    distroList.then(setDistros)
  }
  useEffect(() => load(), [])
  return { distros, reload: () => load(true) }
}

/** A folder on this computer. One inside WSL (`\\wsl.localhost\…`) becomes a WSL project, so
 *  git and the agent run in the distro instead of reaching into it from Windows. */
export async function addLocalFolder() {
  const path = await pickFolder("Add a project folder")
  if (!path) return
  const wsl = await ipc.wslPath(path).catch(() => null)
  if (wsl) {
    await useApp.getState().addProject({ kind: "wsl", distro: wsl.distro }, wsl.path)
    toast.success(`Added as a WSL project in ${wsl.distro}`, { description: "Chats, git and files run inside the distro." })
    return
  }
  await useApp.getState().addProject({ kind: "local" }, path)
}

/** A folder in a WSL distro, picked in its own filesystem. A Windows drive (`/mnt/c/…`) is
 *  better as a project on this computer: git from Linux is very slow there. */
export async function addWslFolder(distro: string) {
  const path = await pickRemoteFolder({ kind: "wsl", distro })
  if (!path) return
  const win = windowsPath(path)
  if (win) {
    const ok = await confirmAction({
      title: "That folder is on a Windows drive",
      description: `${path} is ${win} on this computer. Git and the agent are much faster on it as a project on this computer.`,
      confirm: "Add it from this computer",
    })
    if (ok) await useApp.getState().addProject({ kind: "local" }, win)
    return
  }
  await useApp.getState().addProject({ kind: "wsl", distro }, path)
  useApp.getState().closeSettings()
}

/** "Add a project" from anywhere: this computer, a WSL distro, or an SSH host. */
export function OpenProjectMenu({ children, align = "start" }: { children: ReactNode; align?: "start" | "end" }) {
  const { distros, reload } = useDistros()
  return (
    <DropdownMenu onOpenChange={(o) => o && reload()}>
      <DropdownMenuTrigger asChild>{children}</DropdownMenuTrigger>
      <DropdownMenuContent align={align} className="w-60">
        <DropdownMenuLabel>Add a project</DropdownMenuLabel>
        <DropdownMenuItem onSelect={() => void addLocalFolder()}>
          <Folder /> Folder on this computer…
        </DropdownMenuItem>
        {distros?.length ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuLabel className="text-[11px] font-normal text-faint">WSL</DropdownMenuLabel>
            {distros.map((d) => (
              <DropdownMenuItem key={d.name} onSelect={() => void addWslFolder(d.name)}>
                <LocationIcon location={{ kind: "wsl", distro: d.name }} />
                <span className="flex-1">Folder in {d.name}…</span>
                <span className="text-[10.5px] text-faint">{d.running ? "running" : "stopped"}</span>
              </DropdownMenuItem>
            ))}
          </>
        ) : null}
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={() => useApp.getState().openSettings("ssh")}>
          <Server /> Folder on an SSH host…
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
