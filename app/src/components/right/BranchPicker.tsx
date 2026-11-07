import { useState } from "react"
import { Check, ChevronDown, GitBranch, Plus } from "lucide-react"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { git, type Branch, type Scope, type Status } from "@/lib/git"
import { cn } from "@/lib/utils"
import { useGit } from "@/store/git"

/** The current branch; opens a list to switch to another or create one. */
export function BranchPicker({ scope, status }: { scope: Scope; status: Status }) {
  const run = useGit((s) => s.run)
  const [open, setOpen] = useState(false)
  const [branches, setBranches] = useState<Branch[] | null>(null)
  const [query, setQuery] = useState("")

  const load = () => {
    setBranches(null)
    git.branches(scope).then(setBranches, () => setBranches([]))
  }
  const q = query.trim().toLowerCase()
  const local = (branches ?? []).filter((b) => !b.remote && b.name.toLowerCase().includes(q))
  const remote = (branches ?? []).filter((b) => b.remote && b.name.toLowerCase().includes(q) && !local.some((l) => b.name.endsWith(`/${l.name}`)))
  const exists = (branches ?? []).some((b) => b.name === query.trim())

  const go = async (name: string, create: boolean) => {
    setOpen(false)
    // A remote branch checks out as a local branch tracking it.
    const target = !create && name.includes("/") && remote.some((b) => b.name === name) ? name.slice(name.indexOf("/") + 1) : name
    await run(scope, "Switching…", () => git.switch(scope, target, create))
  }

  return (
    <Popover
      open={open}
      onOpenChange={(o) => {
        setOpen(o)
        if (o) {
          setQuery("")
          load()
        }
      }}
    >
      <PopoverTrigger asChild>
        <button className="rounded-md flex h-6 min-w-0 items-center gap-1.5 px-1 font-mono text-[12px] hover:bg-accent">
          <GitBranch className="size-3.5 shrink-0 text-muted-foreground" />
          <span className="truncate">{status.branch ?? `detached ${status.head ?? ""}`}</span>
          <ChevronDown className="size-3 shrink-0 text-faint" />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-72 p-0">
        <input
          autoFocus
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && query.trim() && go(query.trim(), !exists)}
          placeholder="Switch to or create a branch…"
          className="h-9 w-full border-b bg-transparent px-3 text-[12.5px] outline-none placeholder:text-faint"
        />
        <div className="max-h-72 overflow-y-auto py-1">
          {query.trim() && !exists ? (
            <button className="mx-1 rounded-md flex h-7 w-[calc(100%-0.5rem)] items-center gap-2 px-3 text-left text-[12.5px] hover:bg-accent" onClick={() => go(query.trim(), true)}>
              <Plus className="size-3.5 text-gold" /> Create <span className="font-mono">{query.trim()}</span>
            </button>
          ) : null}
          {branches === null ? <div className="px-3 py-2 text-[12px] text-faint">Loading…</div> : null}
          {local.map((b) => (
            <BranchRow key={b.name} b={b} onPick={() => go(b.name, false)} />
          ))}
          {remote.length ? <div className="mt-1 border-t px-3 pt-2 pb-1 text-[10.5px] tracking-wide text-faint uppercase">Remote</div> : null}
          {remote.map((b) => (
            <BranchRow key={b.name} b={b} onPick={() => go(b.name, false)} />
          ))}
        </div>
      </PopoverContent>
    </Popover>
  )
}

function BranchRow({ b, onPick }: { b: Branch; onPick: () => void }) {
  return (
    <button className={cn("mx-1 rounded-md flex h-7 w-[calc(100%-0.5rem)] items-center gap-2 px-3 text-left font-mono text-[12px] hover:bg-accent", b.current && "text-gold")} onClick={onPick} disabled={b.current}>
      {b.current ? <Check className="size-3.5" /> : <GitBranch className="size-3.5 text-faint" />}
      <span className="truncate">{b.name}</span>
    </button>
  )
}
