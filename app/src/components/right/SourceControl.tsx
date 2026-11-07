import { useState } from "react"
import { ArrowDown, ArrowUp, Check, ChevronDown, ChevronRight, CloudDownload, FileDiff, LoaderCircle, Minus, Plus, RefreshCw, Sparkles, Undo2 } from "lucide-react"
import { confirmAction } from "@/components/Prompt"
import { toast } from "sonner"
import { Tip } from "@/components/Tip"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { git, KIND_COLOR, KIND_LETTER, type Change, type Scope, type Status } from "@/lib/git"
import { cn } from "@/lib/utils"
import { useGit, useStatus } from "@/store/git"
import { BranchPicker } from "./BranchPicker"

/** Changes in the workspace: stage, discard, commit, and sync with the remote. */
export function SourceControl({ scope }: { scope: Scope }) {
  const { status, error, loading } = useStatus(scope)
  if (loading) return <div className="p-4 text-[12px] text-faint">Loading…</div>
  if (error) return <div className="rounded-lg m-3 border border-red/40 bg-del/40 p-3 text-[12px] text-red">{error}</div>
  if (!status?.isRepo) return <div className="rounded-lg m-3 border border-dashed p-3 text-[12px] leading-relaxed text-muted-foreground">This folder isn't a git repository.</div>
  return <Panel scope={scope} status={status} />
}

function Panel({ scope, status }: { scope: Scope; status: Status }) {
  const run = useGit((s) => s.run)
  const busy = useGit((s) => s.busy)
  const [message, setMessage] = useState("")
  const [writing, setWriting] = useState(false)
  const [writtenBy, setWrittenBy] = useState<string | null>(null)
  const changes = [...status.unstaged, ...status.untracked]
  const nothingStaged = status.staged.length === 0
  const total = status.staged.length + changes.length + status.conflicted.length

  const commit = async (opts: { amend?: boolean; push?: boolean } = {}) => {
    const hash = await run(scope, opts.amend ? "Amending…" : "Committing…", () => git.commit(scope, message, !!opts.amend, nothingStaged))
    if (hash === undefined) return
    setMessage("")
    setWrittenBy(null)
    if (opts.push) await run(scope, "Pushing…", () => git.sync(scope, "push"))
  }

  return (
    <div className="flex flex-col">
      <div className="flex h-9 items-center gap-1.5 border-b px-3 text-[12px]">
        <BranchPicker scope={scope} status={status} />
        <span className="ml-auto flex items-center gap-1 font-mono text-[11px] text-muted-foreground">
          {status.upstream ? (
            <>
              <span className={cn("flex items-center", status.behind && "text-cyan")}>
                <ArrowDown className="size-3" />
                {status.behind}
              </span>
              <span className={cn("flex items-center", status.ahead && "text-gold")}>
                <ArrowUp className="size-3" />
                {status.ahead}
              </span>
            </>
          ) : (
            <span className="font-sans text-faint">not published</span>
          )}
        </span>
        <DropdownMenu>
          <Tip content="Sync">
            <DropdownMenuTrigger asChild>
              <button className="rounded-md grid size-6 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Sync">
                <RefreshCw className={cn("size-3.5", busy && "animate-spin")} />
              </button>
            </DropdownMenuTrigger>
          </Tip>
          <DropdownMenuContent align="end" className="w-44">
            <DropdownMenuItem onSelect={() => run(scope, "Syncing…", () => git.sync(scope, "sync"))}>
              <RefreshCw /> Sync (pull, push)
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => run(scope, "Pulling…", () => git.sync(scope, "pull"))}>
              <ArrowDown /> Pull
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => run(scope, "Pushing…", () => git.sync(scope, "push"))}>
              <ArrowUp /> {status.upstream ? "Push" : "Publish branch"}
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={() => run(scope, "Fetching…", () => git.sync(scope, "fetch"))}>
              <CloudDownload /> Fetch
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      <div className="border-b p-3">
        <div className="relative">
        <textarea
          value={message}
          onChange={(e) => {
            setMessage(e.target.value)
            setWrittenBy(null)
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && e.ctrlKey) {
              e.preventDefault()
              commit()
            }
          }}
          rows={3}
          readOnly={writing}
          placeholder={writing ? "Writing a commit message…" : `Message (Ctrl Enter to commit on ${status.branch ?? "HEAD"})`}
          className={cn("rounded-md block w-full resize-none border border-input bg-background py-2 pr-8 pl-2.5 text-[12.5px] outline-none placeholder:text-faint focus:border-ring/70", writing && "shimmer")}
        />
          <Tip content={writing ? "Writing…" : nothingStaged ? "Write a message from all changes (cheapest model)" : "Write a message from the staged changes (cheapest model)"}>
            <button
              className="rounded-md absolute top-1.5 right-1.5 grid size-6 place-items-center text-muted-foreground hover:bg-accent hover:text-gold disabled:opacity-50"
              aria-label="Write a commit message"
              disabled={writing || total === 0}
              onClick={async () => {
                setWriting(true)
                try {
                  const [msg, by] = await git.suggestMessage(scope)
                  setMessage(msg)
                  setWrittenBy(by)
                } catch (e) {
                  toast.error(String(e))
                } finally {
                  setWriting(false)
                }
              }}
            >
              {writing ? <LoaderCircle className="size-3.5 animate-spin text-gold" /> : <Sparkles className="size-3.5" />}
            </button>
          </Tip>
        </div>
        {writtenBy ? <div className="mt-1 text-[10.5px] text-faint">Written by {writtenBy}. Check it before you commit.</div> : null}
        <div className="mt-2 flex">
          <button
            className="rounded-l-md flex h-7 flex-1 items-center justify-center gap-1.5 bg-primary text-[12.5px] font-medium text-primary-foreground hover:bg-primary/90 disabled:bg-muted disabled:text-faint"
            disabled={!message.trim() || total === 0 || !!busy}
            onClick={() => commit()}
          >
            <Check className="size-3.5" />
            {busy ?? (nothingStaged ? "Commit all" : `Commit ${status.staged.length} staged`)}
          </button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button className="rounded-r-md grid h-7 w-7 place-items-center border-l border-primary-foreground/20 bg-primary text-primary-foreground hover:bg-primary/90 disabled:bg-muted disabled:text-faint" disabled={!!busy} aria-label="More commit actions">
                <ChevronDown className="size-3.5" />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-48">
              <DropdownMenuItem disabled={!message.trim() || total === 0} onSelect={() => commit({ push: true })}>
                Commit and push
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => commit({ amend: true })}>Amend last commit</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      {total === 0 ? <div className="px-3 py-4 text-[12px] text-faint">No changes. The working tree is clean.</div> : null}
      {status.conflicted.length ? <Section scope={scope} title="Merge conflicts" changes={status.conflicted} kind="conflict" /> : null}
      {status.staged.length ? <Section scope={scope} title="Staged" changes={status.staged} kind="staged" /> : null}
      {changes.length ? <Section scope={scope} title="Changes" changes={changes} kind="changes" /> : null}
    </div>
  )
}

function Section({ scope, title, changes, kind }: { scope: Scope; title: string; changes: Change[]; kind: "staged" | "changes" | "conflict" }) {
  const [open, setOpen] = useState(true)
  const run = useGit((s) => s.run)
  const paths = changes.map((c) => c.path)
  const untracked = changes.filter((c) => c.kind === "untracked").map((c) => c.path)
  const tracked = changes.filter((c) => c.kind !== "untracked").map((c) => c.path)
  const added = changes.reduce((n, c) => n + c.added, 0)
  const removed = changes.reduce((n, c) => n + c.removed, 0)

  return (
    <div className="border-b">
      <div className="group flex h-8 items-center gap-1 pr-2 pl-2 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        <button className="flex items-center gap-1 hover:text-foreground" onClick={() => setOpen(!open)}>
          {open ? <ChevronDown className="size-3" /> : <ChevronRight className="size-3" />}
          {title}
        </button>
        <span className="rounded-sm border px-1 font-mono text-[10px] normal-case">{changes.length}</span>
        <span className="font-mono text-[10.5px] normal-case">
          <span className="text-green">+{added}</span> <span className="text-red">−{removed}</span>
        </span>
        <span className="ml-auto flex opacity-0 group-hover:opacity-100">
          {kind === "staged" ? (
            <RowButton label="Unstage all" onClick={() => run(scope, "Unstaging…", () => git.unstage(scope, paths))}>
              <Minus />
            </RowButton>
          ) : (
            <>
              <RowButton label="Discard all" onClick={() =>
                  void confirmAction({ title: `Discard all ${changes.length} changes?`, description: "This can't be undone.", confirm: "Discard", danger: true }).then(
                    (ok) => void (ok && run(scope, "Discarding…", () => git.discard(scope, tracked, untracked))),
                  )
                }>
                <Undo2 />
              </RowButton>
              <RowButton label="Stage all" onClick={() => run(scope, "Staging…", () => git.stage(scope, paths))}>
                <Plus />
              </RowButton>
            </>
          )}
        </span>
      </div>
      {open ? changes.map((c) => <ChangeRow key={`${kind}-${c.path}`} scope={scope} change={c} staged={kind === "staged"} />) : null}
    </div>
  )
}

function ChangeRow({ scope, change, staged }: { scope: Scope; change: Change; staged: boolean }) {
  const run = useGit((s) => s.run)
  const viewer = useGit((s) => s.viewer)
  const open = useGit((s) => s.open)
  const slash = change.path.lastIndexOf("/")
  const name = change.path.slice(slash + 1)
  const dir = slash > 0 ? change.path.slice(0, slash) : ""
  const untracked = change.kind === "untracked"
  const active = viewer?.kind === "diff" && viewer.path === change.path && viewer.staged === staged

  return (
    <div
      className={cn("mx-1 rounded-md group flex h-7 cursor-pointer items-center gap-2 pr-2 pl-6 text-[12.5px] hover:bg-accent", active && "row-active")}
      onClick={() => open({ kind: "diff", path: change.path, staged, untracked })}
    >
      <span className="truncate">{name}</span>
      <span className="min-w-0 flex-1 truncate text-[11px] text-faint">{change.origPath ? `${change.origPath} →` : dir}</span>
      <span className="flex shrink-0 opacity-0 group-hover:opacity-100">
        <RowButton label="Open diff" onClick={() => open({ kind: "diff", path: change.path, staged, untracked })}>
          <FileDiff />
        </RowButton>
        {staged ? (
          <RowButton label="Unstage" onClick={() => run(scope, "Unstaging…", () => git.unstage(scope, [change.path]))}>
            <Minus />
          </RowButton>
        ) : (
          <>
            <RowButton label="Discard" onClick={() =>
                void confirmAction({ title: `Discard changes to ${name}?`, description: untracked ? "The file will be deleted. This can't be undone." : "This can't be undone.", confirm: "Discard", danger: true }).then(
                  (ok) => void (ok && run(scope, "Discarding…", () => git.discard(scope, untracked ? [] : [change.path], untracked ? [change.path] : []))),
                )
              }>
              <Undo2 />
            </RowButton>
            <RowButton label="Stage" onClick={() => run(scope, "Staging…", () => git.stage(scope, [change.path]))}>
              <Plus />
            </RowButton>
          </>
        )}
      </span>
      {change.added || change.removed ? (
        <span className="shrink-0 font-mono text-[10.5px] group-hover:hidden">
          <span className="text-green">+{change.added}</span> <span className="text-red">−{change.removed}</span>
        </span>
      ) : null}
      <span className={cn("w-3 shrink-0 text-center font-mono text-[11px] font-semibold", KIND_COLOR[change.kind])}>{KIND_LETTER[change.kind]}</span>
    </div>
  )
}

function RowButton({ label, onClick, children }: { label: string; onClick: () => void; children: React.ReactNode }) {
  return (
    <Tip content={label}>
      <button
        className="rounded-md grid size-6 place-items-center text-muted-foreground hover:bg-background hover:text-foreground [&_svg]:size-3.5"
        aria-label={label}
        onClick={(e) => {
          e.stopPropagation()
          onClick()
        }}
      >
        {children}
      </button>
    </Tip>
  )
}

