import { useEffect, useState } from "react"
import { RefreshCw } from "lucide-react"
import { Tip } from "@/components/Tip"
import { ago } from "@/lib/chat"
import { git, type Change, type GraphRow, type Scope } from "@/lib/git"
import { cn } from "@/lib/utils"
import { useGit, useStatus } from "@/store/git"

const COL = 12
const ROW = 28
const COLORS = ["var(--oah-gold)", "var(--oah-cyan)", "var(--oah-green)", "var(--oah-orange)", "#c792ea", "var(--oah-red)"]
const color = (c: number) => COLORS[c % COLORS.length]
const x = (c: number) => c * COL + COL / 2 + 2

/** One row of the graph: straight lines through, lines into the node, lines out of it. */
function Lanes({ row, width }: { row: GraphRow; width: number }) {
  const mid = ROW / 2
  const nx = x(row.lane)
  return (
    <svg width={width * COL + 4} height={ROW} className="shrink-0" aria-hidden="true">
      {row.through.map((c) => (
        <line key={`t${c}`} x1={x(c)} y1={0} x2={x(c)} y2={ROW} stroke={color(c)} strokeWidth={1.5} />
      ))}
      {row.into.map((c) => (
        <path key={`i${c}`} d={c === row.lane ? `M${nx} 0 V${mid}` : `M${x(c)} 0 L${nx} ${mid}`} stroke={color(c)} strokeWidth={1.5} fill="none" />
      ))}
      {row.out.map((c) => (
        <path key={`o${c}`} d={c === row.lane ? `M${nx} ${mid} V${ROW}` : `M${nx} ${mid} L${x(c)} ${ROW}`} stroke={color(c)} strokeWidth={1.5} fill="none" />
      ))}
      {/* Square nodes; merges are hollow. */}
      <rect x={nx - 3.5} y={mid - 3.5} width={7} height={7} fill={row.commit.parents.length > 1 ? "var(--background)" : color(row.lane)} stroke={color(row.lane)} strokeWidth={1.5} />
    </svg>
  )
}

/** The branch's history as a graph; a commit opens to its files. */
export function History({ scope }: { scope: Scope }) {
  const [rows, setRows] = useState<GraphRow[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [openHash, setOpenHash] = useState<string | null>(null)
  const { status } = useStatus(scope)
  const head = status?.head

  const load = () => {
    setError(null)
    git.log(scope, 300).then(setRows, (e) => setError(String(e)))
  }
  useEffect(load, [scope.projectId, scope.cwd, head])

  const width = Math.max(1, ...(rows ?? []).map((r) => r.width))
  return (
    <div className="flex h-full flex-col">
      <div className="flex h-9 shrink-0 items-center border-b px-3 text-[12px] text-muted-foreground">
        {rows ? `${rows.length} commits` : "History"}
        <Tip content="Reload">
          <button className="rounded-md ml-auto grid size-7 place-items-center hover:bg-accent hover:text-foreground" aria-label="Reload history" onClick={load}>
            <RefreshCw className="size-3.5" />
          </button>
        </Tip>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {error ? <div className="rounded-lg m-2 border border-dashed p-3 text-[12px] text-muted-foreground">{error}</div> : null}
        {rows && rows.length === 0 ? <div className="px-3 py-3 text-[12px] text-faint">No commits yet.</div> : null}
        {rows?.map((r) => (
          <div key={r.commit.hash}>
            <button className={cn("mx-1 rounded-md flex w-[calc(100%-0.5rem)] items-center pr-2 text-left hover:bg-accent", openHash === r.commit.hash && "bg-accent")} style={{ height: ROW }} onClick={() => setOpenHash(openHash === r.commit.hash ? null : r.commit.hash)}>
              <Lanes row={r} width={width} />
              <span className="ml-1 flex min-w-0 flex-1 items-center gap-1.5">
                {r.commit.refs.map((ref) => (
                  <RefChip key={ref} name={ref} />
                ))}
                <span className="truncate text-[12.5px]">{r.commit.subject}</span>
              </span>
              <span className="ml-2 shrink-0 font-mono text-[10.5px] text-faint">{ago(r.commit.time * 1000)}</span>
            </button>
            {openHash === r.commit.hash ? <CommitFiles scope={scope} row={r} /> : null}
          </div>
        ))}
      </div>
    </div>
  )
}

function RefChip({ name }: { name: string }) {
  const head = name.startsWith("HEAD -> ")
  const tag = name.startsWith("tag: ")
  const label = name.replace("HEAD -> ", "").replace("tag: ", "")
  const remote = label.includes("/")
  return (
    <span className={cn("rounded-sm shrink-0 border px-1 font-mono text-[10px] leading-4", head ? "border-primary bg-primary text-primary-foreground" : tag ? "border-green/50 text-green" : remote ? "border-cyan/40 text-cyan" : "text-muted-foreground")}>
      {label}
    </span>
  )
}

function CommitFiles({ scope, row }: { scope: Scope; row: GraphRow }) {
  const [files, setFiles] = useState<Change[] | null>(null)
  const open = useGit((s) => s.open)
  useEffect(() => {
    git.commitFiles(scope, row.commit.hash).then(setFiles, () => setFiles([]))
  }, [row.commit.hash])
  return (
    <div className="border-y bg-background/60 py-1 pl-6">
      <div className="px-2 pb-1 font-mono text-[10.5px] text-faint">
        {row.commit.hash.slice(0, 10)} · {row.commit.author} · {new Date(row.commit.time * 1000).toLocaleString()}
      </div>
      {files === null ? <div className="px-2 py-1 text-[12px] text-faint">Loading…</div> : null}
      {files?.map((f) => (
        <button key={f.path} className="rounded-md flex h-6 w-full items-center gap-2 px-2 text-left text-[12px] hover:bg-accent" onClick={() => open({ kind: "commit", hash: row.commit.hash, path: f.path, subject: row.commit.subject })}>
          <span className="truncate">{f.path}</span>
          <span className="ml-auto shrink-0 font-mono text-[10.5px]">
            <span className="text-green">+{f.added}</span> <span className="text-red">−{f.removed}</span>
          </span>
        </button>
      ))}
    </div>
  )
}
