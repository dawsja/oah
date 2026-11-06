// Working on another machine: the location menu (this computer, WSL distros,
// SSH hosts), the dialogs for connecting and installing oah there, and a
// folder browser for picking a folder on that machine.
import { Check, ChevronDown, ChevronUp, CornerDownLeft, Folder, Loader2, Monitor, Plus, Server, SquareTerminal } from "lucide-react"
import { useEffect, useState } from "react"
import { Button } from "@/components/ui/button"
import { Command, CommandEmpty, CommandGroup, CommandItem, CommandList } from "@/components/ui/command"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import { useClient } from "@/lib/client"
import { LOCAL, sameTarget, targetLabel, type Listing, type Target } from "@/lib/protocol"
import { cn } from "@/lib/utils"

export const TargetIcon = ({ t, className }: { t: Target; className?: string }) =>
  t.kind === "local" ? <Monitor className={className} /> : t.kind === "wsl" ? <SquareTerminal className={className} /> : <Server className={className} />

/** Sidebar: where oah works. */
export function LocationMenu() {
  const c = useClient()
  const [adding, setAdding] = useState(false)
  const current = c.remote?.target ?? c.target
  const options: Target[] = [LOCAL, ...c.targets.wsl.map((distro) => ({ kind: "wsl", distro }) as Target), ...c.sshHosts().map((host) => ({ kind: "ssh", host }) as Target)]
  return (
    <>
      <DropdownMenu onOpenChange={(o) => o && void c.loadTargets()}>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" className="h-8 w-full justify-start gap-2 px-2 text-[13px] font-medium text-muted-foreground hover:text-foreground" aria-label="Where oah works">
            <TargetIcon t={current} className={cn(current.kind !== "local" && "text-cyan")} />
            <span className="truncate">{targetLabel(current)}</span>
            {c.remote?.checking ? <Loader2 className="ml-auto animate-spin" /> : <ChevronDown className="ml-auto opacity-60" />}
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-(--radix-dropdown-menu-trigger-width) min-w-60">
          <DropdownMenuLabel className="text-xs text-muted-foreground">Work on</DropdownMenuLabel>
          {options.map((t, i) => (
            <div key={i}>
              {i === 1 && c.targets.wsl.length ? <DropdownMenuSeparator /> : null}
              {t.kind === "ssh" && (i === 1 || options[i - 1]?.kind !== "ssh") ? <DropdownMenuSeparator /> : null}
              <DropdownMenuItem onSelect={() => void c.switchTarget(t)}>
                <TargetIcon t={t} />
                <span className="truncate">{targetLabel(t)}</span>
                {sameTarget(t, c.target) ? <Check className="ml-auto" /> : null}
              </DropdownMenuItem>
            </div>
          ))}
          <DropdownMenuSeparator />
          <DropdownMenuItem onSelect={() => setAdding(true)}>
            <Plus />
            Add SSH host…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <AddHostDialog open={adding} onOpenChange={setAdding} />
    </>
  )
}

function AddHostDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const c = useClient()
  const [host, setHost] = useState("")
  const valid = /^[^\s-][^\s]*$/.test(host.trim())
  const add = () => {
    if (!valid) return
    onOpenChange(false)
    void c.switchTarget({ kind: "ssh", host: host.trim() })
    setHost("")
  }
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Add an SSH host</DialogTitle>
          <DialogDescription>
            oah connects with your SSH client, keys and <code className="font-mono text-xs">~/.ssh/config</code>. Password prompts aren't supported: use a key or an agent.
          </DialogDescription>
        </DialogHeader>
        <form
          onSubmit={(e) => {
            e.preventDefault()
            add()
          }}
        >
          <Input autoFocus placeholder="user@host or a Host from ~/.ssh/config" value={host} onChange={(e) => setHost(e.target.value)} className="font-mono" aria-label="SSH host" />
        </form>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button disabled={!valid} onClick={add}>
            Connect
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/** Connecting to a remote target, or what it needs: oah installed, updated, or the connection fixed. */
export function RemoteDialog() {
  const c = useClient()
  const r = c.remote
  const open = !!r && (!r.checking || !c.started)
  if (!r) return null
  const name = targetLabel(r.target)
  const old = r.check?.version && !r.check.ok
  return (
    <Dialog open={open} onOpenChange={(o) => !o && !r.installing && c.cancelRemote()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <TargetIcon t={r.target} className="size-4 text-cyan" />
            {r.checking ? `Connecting to ${name}…` : r.error && !r.check ? `Couldn't connect to ${name}` : old ? `oah on ${name} needs an update` : `oah isn't installed on ${name}`}
          </DialogTitle>
          <DialogDescription>
            {r.checking
              ? "Checking that oah is installed there."
              : r.error && !r.check
                ? r.target.kind === "ssh"
                  ? "Check that the host is reachable and that your SSH key works without a password prompt (ssh -o BatchMode=yes)."
                  : "Check that the distro is installed and starts (wsl -d <distro>)."
                : `The desktop app runs the oah CLI on ${name}${old ? ` (it has ${r.check!.version}; ${r.check!.minVersion} or newer is needed)` : ""}. Install it with the official install script, into ~/.config/oah there.`}
          </DialogDescription>
        </DialogHeader>
        {r.error ? <pre className="max-h-40 overflow-auto rounded-md border bg-card p-3 font-mono text-xs whitespace-pre-wrap text-red">{r.error}</pre> : null}
        <DialogFooter>
          <Button variant="outline" disabled={r.installing} onClick={() => c.cancelRemote()}>
            {c.status.kind === "ready" && !sameTarget(r.target, c.target) ? "Cancel" : "Use this computer"}
          </Button>
          {r.checking ? null : r.check ? (
            <Button disabled={r.installing} onClick={() => void c.installRemote()}>
              {r.installing ? <Loader2 className="animate-spin" /> : null}
              {r.installing ? "Installing…" : old ? "Update oah" : "Install oah"}
            </Button>
          ) : (
            <Button onClick={() => void c.switchTarget(r.target, { startup: true })}>Try again</Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/** Pick a folder on the remote target. */
export function FolderBrowser() {
  const c = useClient()
  const open = c.browsing
  const [listing, setListing] = useState<Listing | null>(null)
  const [path, setPath] = useState("")
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)

  const go = async (p: string) => {
    setLoading(true)
    setError(null)
    try {
      const l = await c.t.remote.ls(c.target, p)
      setListing(l)
      setPath(l.path)
    } catch (e: any) {
      setError(e?.message ?? String(e))
    }
    setLoading(false)
  }
  useEffect(() => {
    if (open) void go(c.started?.cwd ?? "")
  }, [open])

  const join = (dir: string) => `${listing!.path.replace(/\/+$/, "")}/${dir}`
  const parent = listing && listing.path !== "/" ? listing.path.replace(/\/[^/]+\/?$/, "") || "/" : null
  const pick = (p: string) => {
    c.setBrowsing(false)
    c.newSession(p)
  }

  return (
    <Dialog open={open} onOpenChange={(o) => c.setBrowsing(o)}>
      <DialogContent className="gap-3 sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <TargetIcon t={c.target} className="size-4 text-cyan" />
            Open a folder on {targetLabel(c.target)}
          </DialogTitle>
          <DialogDescription>Pick a folder to start a session in.</DialogDescription>
        </DialogHeader>
        <form
          className="flex gap-2"
          onSubmit={(e) => {
            e.preventDefault()
            void go(path)
          }}
        >
          <Input value={path} onChange={(e) => setPath(e.target.value)} className="font-mono text-xs" aria-label="Folder path" />
          <Button type="submit" variant="outline" size="icon" aria-label="Go to this folder" disabled={loading}>
            {loading ? <Loader2 className="animate-spin" /> : <CornerDownLeft />}
          </Button>
        </form>
        {error ? <p className="text-xs text-red">{error}</p> : null}
        <Command className="h-72 rounded-md border bg-card" shouldFilter>
          <CommandList className="max-h-none">
            <CommandEmpty>{loading ? "Loading…" : "No subfolders"}</CommandEmpty>
            <CommandGroup>
              {parent ? (
                <CommandItem value=".." onSelect={() => void go(parent)}>
                  <ChevronUp />
                  ..
                </CommandItem>
              ) : null}
              {listing?.dirs.map((d) => (
                <CommandItem key={d} value={d} onSelect={() => void go(join(d))}>
                  <Folder />
                  <span className="truncate">{d}</span>
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
        <DialogFooter>
          <Button variant="outline" onClick={() => c.setBrowsing(false)}>
            Cancel
          </Button>
          <Button disabled={!listing || loading} onClick={() => listing && pick(listing.path)}>
            Open {listing ? listing.path.split("/").filter(Boolean).pop() ?? "/" : "folder"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
