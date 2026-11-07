import { useEffect, useState } from "react"
import { ArrowUp, Folder, House, LoaderCircle } from "lucide-react"
import { create } from "zustand"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { ipc } from "@/lib/ipc"

type Pick = { host: string; resolve: (path: string | null) => void }
const usePick = create<{ pick: Pick | null }>(() => ({ pick: null }))

/** Browses folders on an SSH host; resolves with the chosen path, or null. */
export function pickRemoteFolder(host: string): Promise<string | null> {
  return new Promise((resolve) => usePick.setState({ pick: { host, resolve } }))
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
    ipc.sshListDir(pick.host, p).then(
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
  const up = path.replace(/\/[^/]+\/?$/, "") || "/"

  return (
    <Dialog open={!!pick} onOpenChange={(o) => !o && done(null)}>
      <DialogContent className="gap-3 sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="text-[15px]">Choose a folder on {pick?.host}</DialogTitle>
        </DialogHeader>
        <div className="flex gap-1">
          <Button variant="outline" size="icon-sm" aria-label="Home" onClick={() => go("")}>
            <House />
          </Button>
          <Button variant="outline" size="icon-sm" aria-label="Up" onClick={() => go(up)} disabled={path === "/"}>
            <ArrowUp />
          </Button>
          <input
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && go(typed)}
            className="rounded-md h-8 min-w-0 flex-1 border border-input bg-background px-2.5 font-mono text-[12px] outline-none focus:border-ring/70"
          />
        </div>
        <div className="rounded-lg h-72 overflow-y-auto border bg-background">
          {dirs === null ? (
            <div className="flex items-center gap-2 p-3 text-[12px] text-faint">
              <LoaderCircle className="size-3.5 animate-spin" /> Connecting…
            </div>
          ) : null}
          {error ? <div className="p-3 text-[12px] text-red">{error}</div> : null}
          {dirs?.length === 0 && !error ? <div className="p-3 text-[12px] text-faint">No folders here.</div> : null}
          {dirs?.map((d) => (
            <button key={d} className="mx-1 rounded-md flex h-7 w-[calc(100%-0.5rem)] items-center gap-2 px-3 text-left text-[12.5px] hover:bg-accent" onDoubleClick={() => go(`${path.replace(/\/$/, "")}/${d}`)} onClick={() => setTyped(`${path.replace(/\/$/, "")}/${d}`)}>
              <Folder className="size-3.5 text-muted-foreground" />
              <span className={d.startsWith(".") ? "text-faint" : undefined}>{d}</span>
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
