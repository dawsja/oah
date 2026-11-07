import { useEffect, useRef, useState } from "react"
import { ArrowLeft, AtSign, Minus, Plus } from "lucide-react"
import { git, isImagePath, type Scope } from "@/lib/git"
import { ImageStage } from "@/components/ImageView"
import { useGit, type Viewer as V } from "@/store/git"
import { addToChat } from "@/components/right/Files"
import { DiffLines, DiffStat, parsePatch } from "./DiffView"

/** A diff or a file from the side panel, read-only, in place of the chat. */
export function Viewer({ scope, viewer }: { scope: Scope; viewer: V }) {
  const open = useGit((s) => s.open)
  const run = useGit((s) => s.run)
  const [text, setText] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const status = useGit((s) => s.status[`${scope.projectId}|${scope.cwd ?? ""}`])

  useEffect(() => {
    setText(null)
    setError(null)
    const p =
      viewer.kind === "diff"
        ? git.diff(scope, viewer.path, viewer.staged, viewer.untracked)
        : viewer.kind === "commit"
          ? git.commitDiff(scope, viewer.hash, viewer.path)
          : isImagePath(viewer.path)
            ? git.readImage(scope, viewer.path)
            : git.read(scope, viewer.path)
    p.then(setText, (e) => setError(String(e)))
    // Diffs follow the working tree.
  }, [viewer, scope.projectId, scope.cwd, viewer.kind === "diff" ? status : null])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && !(e.target instanceof HTMLTextAreaElement) && open(null)
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [open])

  const lines = viewer.kind !== "file" && text ? parsePatch(text) : []
  const added = lines.filter((l) => l.cls === "a").length
  const removed = lines.filter((l) => l.cls === "d").length

  return (
    <div className="flex h-full min-w-0 flex-col bg-background">
      <div className="flex h-10 shrink-0 items-center gap-2 border-b px-3">
        <button className="rounded-md flex h-7 items-center gap-1.5 px-2 text-[12px] text-muted-foreground hover:bg-accent hover:text-foreground" onClick={() => open(null)}>
          <ArrowLeft className="size-3.5" /> Chat
        </button>
        <span className="h-4 w-px bg-border" />
        <span className="truncate font-mono text-[12.5px]">{viewer.path}</span>
        {viewer.kind === "diff" ? <span className="rounded-sm border px-1.5 text-[10.5px] text-muted-foreground">{viewer.untracked ? "new file" : viewer.staged ? "staged" : "working tree"}</span> : null}
        {viewer.kind === "commit" ? <span className="truncate text-[11.5px] text-faint">{viewer.hash.slice(0, 8)} · {viewer.subject}</span> : null}
        <span className="ml-auto flex items-center gap-1">
          {viewer.kind !== "file" && text ? <DiffStat added={added} removed={removed} /> : null}
          {viewer.kind === "diff" ? (
            viewer.staged ? (
              <ActionButton onClick={() => run(scope, "Unstaging…", () => git.unstage(scope, [viewer.path])).then(() => open({ ...viewer, staged: false }))}>
                <Minus /> Unstage
              </ActionButton>
            ) : (
              <ActionButton onClick={() => run(scope, "Staging…", () => git.stage(scope, [viewer.path])).then(() => open({ ...viewer, staged: true, untracked: false }))}>
                <Plus /> Stage
              </ActionButton>
            )
          ) : null}
          <ActionButton onClick={() => addToChat(viewer.path)}>
            <AtSign /> Add to chat
          </ActionButton>
        </span>
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        {error ? <div className="rounded-lg m-4 border border-red/40 bg-del/40 p-3 text-[12.5px] text-red">{error}</div> : null}
        {text === null && !error ? <div className="p-4 text-[12px] text-faint">Loading…</div> : null}
        {text !== null && viewer.kind !== "file" ? text.trim() ? <DiffLines patch={text} className="py-2" /> : <div className="p-4 text-[12px] text-faint">No differences.</div> : null}
        {text !== null && viewer.kind === "file" ? (
          isImagePath(viewer.path) ? (
            <ImageStage image={{ src: text, name: viewer.path.split("/").pop() ?? viewer.path }} className="h-full" />
          ) : (
            <FileText text={text} line={viewer.line} />
          )
        ) : null}
      </div>
    </div>
  )
}

function ActionButton({ onClick, children }: { onClick: () => void; children: React.ReactNode }) {
  return (
    <button className="rounded-md flex h-7 items-center gap-1.5 border border-input px-2.5 text-[12px] hover:bg-accent [&_svg]:size-3.5" onClick={onClick}>
      {children}
    </button>
  )
}

function FileText({ text, line }: { text: string; line?: number }) {
  const target = useRef<HTMLTableRowElement>(null)
  useEffect(() => target.current?.scrollIntoView({ block: "center" }), [line, text])
  const lines = text.replace(/\n$/, "").split("\n")
  return (
    <table className="w-full border-collapse font-mono text-[12px] leading-[1.65]">
      <tbody>
        {lines.map((l, i) => (
          <tr key={i} ref={i + 1 === line ? target : undefined} className={i + 1 === line ? "bg-primary/15" : undefined}>
            <td className="w-12 pr-3 text-right align-top text-faint select-none">{i + 1}</td>
            <td className="pr-4 whitespace-pre text-foreground/90 select-text">{l || " "}</td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}
