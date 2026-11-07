import { useEffect, useMemo, useState } from "react"
import { AtSign, ChevronDown, ChevronRight, File, Folder, RefreshCw, Search, TextSearch } from "lucide-react"
import { Tip } from "@/components/Tip"
import { git, KIND_COLOR, type ChangeKind, type GrepHit, type Scope } from "@/lib/git"
import { cn } from "@/lib/utils"
import { useGit, useStatus } from "@/store/git"

type Node = { name: string; path: string; dir: boolean; children: Node[] }

function tree(paths: string[]): Node[] {
  const root: Node = { name: "", path: "", dir: true, children: [] }
  for (const p of paths) {
    let cur = root
    const parts = p.split("/")
    parts.forEach((part, i) => {
      const leaf = i === parts.length - 1
      let next = cur.children.find((c) => c.name === part && c.dir === !leaf)
      if (!next) {
        next = { name: part, path: parts.slice(0, i + 1).join("/"), dir: !leaf, children: [] }
        cur.children.push(next)
      }
      cur = next
    })
  }
  const sort = (n: Node) => {
    n.children.sort((a, b) => (a.dir === b.dir ? a.name.localeCompare(b.name) : a.dir ? -1 : 1))
    n.children.forEach(sort)
  }
  sort(root)
  return root.children
}

/** Puts `@path` into the composer. */
export function addToChat(path: string) {
  window.dispatchEvent(new CustomEvent("oah:insert", { detail: `@${path} ` }))
}

/** The workspace's files: a tree with git colors, a name filter, and a search inside files. */
export function Files({ scope }: { scope: Scope }) {
  const [files, setFiles] = useState<string[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [filter, setFilter] = useState("")
  const [mode, setMode] = useState<"tree" | "search">("tree")
  const { status } = useStatus(scope)
  const k = `${scope.projectId}|${scope.cwd}`

  const load = () => {
    setError(null)
    git.files(scope).then(setFiles, (e) => setError(String(e)))
  }
  useEffect(load, [k])

  const kinds = useMemo(() => {
    const m = new Map<string, ChangeKind>()
    for (const c of [...(status?.untracked ?? []), ...(status?.unstaged ?? []), ...(status?.staged ?? []), ...(status?.conflicted ?? [])]) m.set(c.path, c.kind)
    return m
  }, [status])

  const shown = useMemo(() => {
    if (!files) return []
    const q = filter.trim().toLowerCase()
    return q ? files.filter((f) => f.toLowerCase().includes(q)) : files
  }, [files, filter])

  return (
    <div className="flex h-full flex-col">
      <div className="flex h-9 shrink-0 items-center gap-1 border-b px-2">
        <div className="rounded-md flex h-7 flex-1 items-center gap-1.5 border border-input bg-background px-2 focus-within:border-ring/70">
          {mode === "tree" ? <Search className="size-3.5 text-faint" /> : <TextSearch className="size-3.5 text-faint" />}
          <input
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder={mode === "tree" ? "Filter files" : "Search in files"}
            className="min-w-0 flex-1 bg-transparent text-[12px] outline-none placeholder:text-faint"
          />
        </div>
        <Tip content={mode === "tree" ? "Search in files" : "Back to the tree"}>
          <button className={cn("rounded-md grid size-7 place-items-center hover:bg-accent", mode === "search" ? "text-gold" : "text-muted-foreground")} aria-label="Search in files" onClick={() => setMode(mode === "tree" ? "search" : "tree")}>
            <TextSearch className="size-3.5" />
          </button>
        </Tip>
        <Tip content="Reload">
          <button className="rounded-md grid size-7 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Reload files" onClick={load}>
            <RefreshCw className="size-3.5" />
          </button>
        </Tip>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto py-1">
        {error ? <div className="rounded-lg m-2 border border-dashed p-3 text-[12px] text-muted-foreground">{error.includes("not a git repository") ? "Files are listed for git repositories." : error}</div> : null}
        {!files && !error ? <div className="px-3 py-2 text-[12px] text-faint">Loading…</div> : null}
        {mode === "search" ? (
          <SearchResults scope={scope} query={filter} />
        ) : filter.trim() ? (
          shown.slice(0, 400).map((f) => <FileRow key={f} path={f} label={f} depth={0} kind={kinds.get(f)} />)
        ) : (
          tree(shown).map((n) => <TreeNode key={n.path} node={n} depth={0} kinds={kinds} />)
        )}
      </div>
    </div>
  )
}

function TreeNode({ node, depth, kinds }: { node: Node; depth: number; kinds: Map<string, ChangeKind> }) {
  const [open, setOpen] = useState(depth === 0 && node.children.length < 12)
  if (!node.dir) return <FileRow path={node.path} label={node.name} depth={depth} kind={kinds.get(node.path)} />
  const changed = [...kinds.keys()].some((k) => k.startsWith(node.path + "/"))
  return (
    <>
      <button className="mx-1 rounded-md flex h-6 w-[calc(100%-0.5rem)] items-center gap-1 pr-2 text-left text-[12.5px] hover:bg-accent" style={{ paddingLeft: 8 + depth * 12 }} onClick={() => setOpen(!open)}>
        {open ? <ChevronDown className="size-3 text-faint" /> : <ChevronRight className="size-3 text-faint" />}
        <Folder className="size-3.5 text-muted-foreground" />
        <span className={cn("truncate", changed && "text-warn")}>{node.name}</span>
      </button>
      {open ? node.children.map((c) => <TreeNode key={c.path} node={c} depth={depth + 1} kinds={kinds} />) : null}
    </>
  )
}

function FileRow({ path, label, depth, kind, line }: { path: string; label: string; depth: number; kind?: ChangeKind; line?: number }) {
  const open = useGit((s) => s.open)
  const viewer = useGit((s) => s.viewer)
  const active = viewer?.kind === "file" && viewer.path === path
  return (
    <div
      className={cn("mx-1 rounded-md group flex h-6 cursor-pointer items-center gap-1.5 pr-1 text-[12.5px] hover:bg-accent", active && "row-active")}
      style={{ paddingLeft: 20 + depth * 12 }}
      onClick={() => open({ kind: "file", path, line })}
    >
      <File className="size-3.5 shrink-0 text-faint" />
      <span className={cn("truncate", kind ? KIND_COLOR[kind] : "text-foreground/90")}>{label}</span>
      <Tip content="Add to chat">
        <button
          className="rounded-md ml-auto hidden size-5 shrink-0 place-items-center text-muted-foreground group-hover:grid hover:text-gold"
          aria-label="Add to chat"
          onClick={(e) => {
            e.stopPropagation()
            addToChat(path)
          }}
        >
          <AtSign className="size-3" />
        </button>
      </Tip>
    </div>
  )
}

function SearchResults({ scope, query }: { scope: Scope; query: string }) {
  const [hits, setHits] = useState<GrepHit[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const open = useGit((s) => s.open)
  useEffect(() => {
    const q = query.trim()
    if (q.length < 2) {
      setHits(null)
      return
    }
    const t = setTimeout(() => {
      setError(null)
      git.grep(scope, q).then(setHits, (e) => setError(String(e)))
    }, 250)
    return () => clearTimeout(t)
  }, [query, scope.projectId, scope.cwd])

  if (query.trim().length < 2) return <div className="px-3 py-2 text-[12px] text-faint">Type at least two characters.</div>
  if (error) return <div className="px-3 py-2 text-[12px] text-red">{error}</div>
  if (!hits) return <div className="px-3 py-2 text-[12px] text-faint">Searching…</div>
  if (!hits.length) return <div className="px-3 py-2 text-[12px] text-faint">No matches.</div>
  const byFile = new Map<string, GrepHit[]>()
  for (const h of hits) byFile.set(h.path, [...(byFile.get(h.path) ?? []), h])
  return (
    <>
      <div className="px-3 pb-1 text-[11px] text-faint">
        {hits.length} matches in {byFile.size} files
      </div>
      {[...byFile].map(([path, hs]) => (
        <div key={path}>
          <div className="flex h-6 items-center gap-1.5 px-3 text-[12px] font-medium">
            <File className="size-3.5 text-faint" />
            <span className="truncate">{path}</span>
            <span className="rounded-sm ml-auto border px-1 font-mono text-[10px] text-muted-foreground">{hs.length}</span>
          </div>
          {hs.map((h) => (
            <button key={h.line} className="mx-1 rounded-md flex w-[calc(100%-0.5rem)] items-baseline gap-2 py-0.5 pr-2 pl-8 text-left hover:bg-accent" onClick={() => open({ kind: "file", path, line: h.line })}>
              <span className="w-7 shrink-0 text-right font-mono text-[10.5px] text-faint">{h.line}</span>
              <span className="truncate font-mono text-[11.5px] text-muted-foreground">{highlight(h.text, query.trim())}</span>
            </button>
          ))}
        </div>
      ))}
    </>
  )
}

function highlight(text: string, q: string) {
  const i = text.toLowerCase().indexOf(q.toLowerCase())
  if (i < 0) return text
  return (
    <>
      {text.slice(0, i)}
      <mark className="rounded-sm bg-primary/30 text-foreground">{text.slice(i, i + q.length)}</mark>
      {text.slice(i + q.length)}
    </>
  )
}
