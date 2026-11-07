import { useEffect, useState } from "react"
import { ArrowUp, Folder, HardDrive, House, LoaderCircle, Slash } from "lucide-react"
import { create } from "zustand"
import { LocationIcon } from "@/components/LocationIcon"
import { Tip } from "@/components/Tip"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { ipc } from "@/lib/ipc"
import { locationLabel, type Location } from "@/lib/types"

type Pick = { location: Location; resolve: (path: string | null) => void }
const usePick = create<{ pick: Pick | null }>(() => ({ pick: null }))

/** Browses folders on an SSH host or in a WSL distro, in that system's own filesystem; resolves
 *  with the chosen Linux path, or null. */
export function pickRemoteFolder(location: Location): Promise<string | null> {
  return new Promise((resolve) => usePick.setState({ pick: { location, resolve } }))
}

export function RemoteFolderHost() {
  const pick = usePick((s) => s.pick)
  const [path, setPath] = useState("")
  const [dirs, setDirs] = useState<string[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [typed, setTyped] = useState("")

  const go = (p: string) => {
    if (!pick) return
    setDirs(null)
    setError(null)
    ipc.hostListDir(pick.location, p).then(
      (l) => {
        setPath(l.path)
        setTyped(l.path)
        setDirs(l.dirs)
      },
      (e) => {
        setError(String(e))
        setDirs([])
      },
    )
  }
  useEffect(() => {
    if (pick) go("")
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pick])

  const done = (v: string | null) => {
    pick?.resolve(v)
    usePick.setState({ pick: null })
  }
  const join = (d: string) => `${path.replace(/\/$/, "")}/${d}`
  const up = path.replace(/\/[^/]+\/?$/, "") || "/"
  const wsl = pick?.location.kind === "wsl"
  const name = pick ? locationLabel(pick.location) : ""
  // WSL mounts Windows drives as /mnt/<letter>.
  const isDrive = (d: string) => wsl && path === "/mnt" && /^[a-z]$/.test(d)

  return (
    <Dialog open={!!pick} onOpenChange={(o) => !o && done(null)}>
      <DialogContent className="gap-3 sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-[15px]">
            {pick ? <LocationIcon location={pick.location} className="size-4" /> : null}
            Choose a folder {wsl ? "in" : "on"} {name}
          </DialogTitle>
          <DialogDescription className="text-[12px]">{wsl ? `${name}'s own Linux filesystem. Chats, git and files for this project run inside ${name}.` : "The agent, git and files for this project run on that machine."}</DialogDescription>
        </DialogHeader>
        <div className="flex gap-1">
          <Tip content="Home folder">
            <Button variant="outline" size="icon-sm" aria-label="Home" onClick={() => go("")}>
              <House />
            </Button>
          </Tip>
          <Tip content="The filesystem root (/)">
            <Button variant="outline" size="icon-sm" aria-label="Root" onClick={() => go("/")} disabled={path === "/"}>
              <Slash />
            </Button>
          </Tip>
          <Tip content="Up one folder">
            <Button variant="outline" size="icon-sm" aria-label="Up" onClick={() => go(up)} disabled={path === "/"}>
              <ArrowUp />
            </Button>
          </Tip>
          <input
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && go(typed)}
            aria-label="Folder path"
            className="rounded-md h-8 min-w-0 flex-1 border border-input bg-background px-2.5 font-mono text-[12px] outline-none focus:border-ring/70"
          />
        </div>
        <div className="rounded-lg h-72 overflow-y-auto border bg-background py-1">
          {dirs === null ? (
            <div className="flex items-center gap-2 p-3 text-[12px] text-faint">
              <LoaderCircle className="size-3.5 animate-spin" /> {wsl ? `Opening ${name}… (a stopped distro takes a few seconds to start)` : "Connecting…"}
            </div>
          ) : null}
          {error ? <div className="p-3 text-[12px] text-red">{error}</div> : null}
          {dirs?.length === 0 && !error ? <div className="p-3 text-[12px] text-faint">No folders here.</div> : null}
          {dirs?.map((d) => (
            <button
              key={d}
              className="mx-1 rounded-md flex h-7 w-[calc(100%-0.5rem)] items-center gap-2 px-3 text-left text-[12.5px] hover:bg-accent focus-visible:bg-accent focus-visible:outline-none"
              onDoubleClick={() => go(join(d))}
              onClick={() => setTyped(join(d))}
            >
              {isDrive(d) ? <HardDrive className="size-3.5 text-muted-foreground" /> : <Folder className="size-3.5 text-muted-foreground" />}
              <span className={d.startsWith(".") ? "text-faint" : undefined}>{d}</span>
              {isDrive(d) ? <span className="rounded-sm border px-1 text-[10px] text-faint">Windows drive {d.toUpperCase()}:</span> : null}
            </button>
          ))}
        </div>
        <DialogFooter>
          <span className="mr-auto self-center text-[11.5px] text-faint">Double-click to open a folder.</span>
          <Button variant="ghost" size="sm" onClick={() => done(null)}>
            Cancel
          </Button>
          <Button size="sm" disabled={!typed.trim() || !!error} onClick={() => done(typed.trim())}>
            Use this folder
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
