import { useEffect, useState } from "react"
import { Check, CircleDashed, ExternalLink, GitMerge, GitPullRequest, GitPullRequestDraft, Loader2, Minus, RefreshCw, X } from "lucide-react"
import { Tip } from "@/components/Tip"
import { git, type PrState, type PullRequest as PR, type Scope } from "@/lib/git"
import { inTauri } from "@/lib/ipc"
import { cn } from "@/lib/utils"
import { useGit, useStatus } from "@/store/git"
import { toast } from "sonner"

function openUrl(url: string) {
  if (inTauri) import("@tauri-apps/plugin-opener").then((m) => m.openUrl(url))
  else window.open(url, "_blank")
}

/** The branch's pull request on GitHub (through `gh`): its state and checks, or a form to open one. */
export function PullRequest({ scope }: { scope: Scope }) {
  const [state, setState] = useState<PrState | null>(null)
  const { status } = useStatus(scope)
  const load = () => {
    setState(null)
    git.prState(scope).then(setState, (e) => setState({ kind: "unavailable", reason: String(e) }))
  }
  useEffect(load, [scope.projectId, scope.cwd, status?.branch])
  // Checks move on their own: look again every 30s while a PR is open.
  useEffect(() => {
    if (state?.kind !== "open") return
    const t = setInterval(() => git.prState(scope).then(setState, () => {}), 30_000)
    return () => clearInterval(t)
  }, [state?.kind, scope.projectId, scope.cwd])

  return (
    <div className="flex h-full flex-col">
      <div className="flex h-9 shrink-0 items-center border-b px-3 text-[12px] text-muted-foreground">
        {state?.kind === "open" ? `#${state.pr.number}` : "GitHub"}
        <Tip content="Reload">
          <button className="rounded-md ml-auto grid size-7 place-items-center hover:bg-accent hover:text-foreground" aria-label="Reload pull request" onClick={load}>
            <RefreshCw className="size-3.5" />
          </button>
        </Tip>
      </div>
      {state === null ? <div className="p-3 text-[12px] text-faint">Loading…</div> : null}
      {state?.kind === "unavailable" ? <div className="rounded-lg m-3 border border-dashed p-3 text-[12px] leading-relaxed text-muted-foreground">{state.reason}</div> : null}
      {state?.kind === "none" ? <CreateForm scope={scope} branch={state.branch} base={state.base} onCreated={load} /> : null}
      {state?.kind === "open" ? <Open pr={state.pr} /> : null}
    </div>
  )
}

function Open({ pr }: { pr: PR }) {
  const merged = pr.state === "MERGED"
  const closed = pr.state === "CLOSED"
  const Icon = merged ? GitMerge : pr.draft ? GitPullRequestDraft : GitPullRequest
  const tone = merged ? "text-[#c792ea] border-[#c792ea]/50" : closed ? "text-red border-red/50" : pr.draft ? "text-muted-foreground" : "text-green border-green/50"
  const passed = pr.checks.filter((c) => c.state === "success").length
  const failed = pr.checks.filter((c) => ["failure", "error", "timed_out", "cancelled", "action_required"].includes(c.state)).length
  const pending = pr.checks.filter((c) => c.state === "pending").length

  return (
    <div className="overflow-y-auto">
      <div className="border-b p-3">
        <div className="flex items-start gap-2">
          <Icon className={cn("mt-0.5 size-4 shrink-0", tone.split(" ")[0])} />
          <button className="text-left text-[13px] leading-snug font-semibold hover:underline" onClick={() => openUrl(pr.url)}>
            {pr.title}
          </button>
        </div>
        <div className="mt-2 flex flex-wrap items-center gap-1.5 text-[11px]">
          <span className={cn("rounded-sm border px-1.5 py-px font-medium uppercase", tone)}>{merged ? "merged" : closed ? "closed" : pr.draft ? "draft" : "open"}</span>
          <span className="font-mono text-muted-foreground">
            {pr.head} → {pr.base}
          </span>
          <span className="font-mono">
            <span className="text-green">+{pr.additions}</span> <span className="text-red">−{pr.deletions}</span>
          </span>
        </div>
        <div className="overflow-hidden rounded-md mt-2 grid grid-cols-2 gap-px border bg-border text-[11.5px]">
          <div className="bg-sidebar px-2 py-1.5">
            <div className="text-faint">Review</div>
            <div className={pr.review === "APPROVED" ? "text-green" : pr.review === "CHANGES_REQUESTED" ? "text-red" : "text-muted-foreground"}>{(pr.review || "none").replace(/_/g, " ").toLowerCase()}</div>
          </div>
          <div className="bg-sidebar px-2 py-1.5">
            <div className="text-faint">Merge</div>
            <div className={pr.mergeState === "CLEAN" ? "text-green" : pr.mergeState === "DIRTY" ? "text-red" : "text-muted-foreground"}>{(pr.mergeState || "unknown").toLowerCase()}</div>
          </div>
        </div>
        <button className="rounded-md mt-2 flex h-7 w-full items-center justify-center gap-1.5 border border-input text-[12px] hover:bg-accent" onClick={() => openUrl(pr.url)}>
          <ExternalLink className="size-3.5" /> Open on GitHub
        </button>
      </div>
      <div className="flex h-8 items-center gap-2 px-3 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
        Checks
        <span className="ml-auto font-mono normal-case">
          {passed ? <span className="text-green">{passed} passed</span> : null} {failed ? <span className="text-red">{failed} failed</span> : null} {pending ? <span className="text-warn">{pending} running</span> : null}
        </span>
      </div>
      {pr.checks.length === 0 ? <div className="px-3 pb-3 text-[12px] text-faint">No checks on this pull request.</div> : null}
      {pr.checks.map((c, i) => (
        <button key={`${c.name}-${i}`} className="mx-1 rounded-md flex h-7 w-[calc(100%-0.5rem)] items-center gap-2 px-3 text-left text-[12.5px] hover:bg-accent disabled:hover:bg-transparent" disabled={!c.url} onClick={() => c.url && openUrl(c.url)}>
          <CheckIcon state={c.state} />
          <span className="truncate">{c.name}</span>
          {c.url ? <ExternalLink className="ml-auto size-3 shrink-0 text-faint" /> : null}
        </button>
      ))}
    </div>
  )
}

function CheckIcon({ state }: { state: string }) {
  if (state === "success") return <Check className="size-3.5 shrink-0 text-green" />
  if (state === "pending") return <Loader2 className="size-3.5 shrink-0 animate-spin text-warn" />
  if (state === "skipped" || state === "neutral") return <Minus className="size-3.5 shrink-0 text-faint" />
  if (["failure", "error", "timed_out", "cancelled", "action_required"].includes(state)) return <X className="size-3.5 shrink-0 text-red" />
  return <CircleDashed className="size-3.5 shrink-0 text-faint" />
}

function CreateForm({ scope, branch, base, onCreated }: { scope: Scope; branch: string; base: string; onCreated: () => void }) {
  const run = useGit((s) => s.run)
  const [title, setTitle] = useState("")
  const [body, setBody] = useState("")
  const [target, setTarget] = useState(base)
  const [draft, setDraft] = useState(false)
  const [busy, setBusy] = useState(false)
  const onBase = branch === base

  useEffect(() => {
    // Start from the last commit's subject.
    git.log(scope, 1).then((r) => r[0] && setTitle((t) => t || r[0].commit.subject), () => {})
  }, [scope.projectId, scope.cwd])

  const create = async () => {
    setBusy(true)
    const url = await run(scope, "Opening pull request…", () => git.prCreate(scope, title, body, target, draft))
    setBusy(false)
    if (url) {
      toast.success("Pull request opened", { action: { label: "Open", onClick: () => openUrl(url) } })
      onCreated()
    }
  }

  return (
    <div className="flex flex-col gap-2 p-3">
      <div className="text-[12px] text-muted-foreground">
        No pull request for <span className="font-mono text-foreground">{branch}</span> yet.
      </div>
      {onBase ? <div className="rounded-md border border-warn/40 p-2 text-[11.5px] text-warn">You're on {base}. Make a branch or a worktree first.</div> : null}
      <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Title" className="rounded-md h-8 border border-input bg-background px-2.5 text-[12.5px] outline-none focus:border-ring/70" />
      <textarea value={body} onChange={(e) => setBody(e.target.value)} rows={6} placeholder="Description (markdown)" className="rounded-md resize-none border border-input bg-background px-2.5 py-2 text-[12.5px] outline-none focus:border-ring/70" />
      <div className="flex items-center gap-2 text-[12px]">
        <span className="text-faint">into</span>
        <input value={target} onChange={(e) => setTarget(e.target.value)} className="rounded-md h-7 w-32 border border-input bg-background px-2 font-mono text-[12px] outline-none focus:border-ring/70" />
        <label className="ml-auto flex cursor-pointer items-center gap-1.5 text-muted-foreground">
          <input type="checkbox" checked={draft} onChange={(e) => setDraft(e.target.checked)} className="size-3.5 accent-[var(--primary)]" />
          Draft
        </label>
      </div>
      <button className="rounded-md flex h-8 items-center justify-center gap-1.5 bg-primary text-[12.5px] font-medium text-primary-foreground hover:bg-primary/90 disabled:bg-muted disabled:text-faint" disabled={!title.trim() || busy || onBase} onClick={create}>
        <GitPullRequest className="size-3.5" /> {busy ? "Opening…" : "Push and open pull request"}
      </button>
    </div>
  )
}
