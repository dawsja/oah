import { useState } from "react"
import { ChevronDown, ChevronRight, FileDiff as FileDiffIcon } from "lucide-react"
import type { FileDiff } from "@/lib/chat"
import { cn } from "@/lib/utils"
import { useRelative } from "./cwd"
import { useTouch } from "./touch"

type Line = { cls: "a" | "d" | "h" | "c"; text: string; old?: number; new?: number }

/** Unified diff lines with old/new line numbers. */
export function parsePatch(patch: string): Line[] {
  const out: Line[] = []
  let o = 0
  let n = 0
  for (const raw of patch.split("\n")) {
    if (raw.startsWith("---") || raw.startsWith("+++") || raw.startsWith("diff ") || raw.startsWith("index ")) continue
    const h = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)$/.exec(raw)
    if (h) {
      o = Number(h[1])
      n = Number(h[2])
      out.push({ cls: "h", text: raw })
    } else if (raw.startsWith("+")) out.push({ cls: "a", text: raw.slice(1), new: n++ })
    else if (raw.startsWith("-")) out.push({ cls: "d", text: raw.slice(1), old: o++ })
    else if (raw !== "" || out.length) out.push({ cls: "c", text: raw.slice(1), old: o++, new: n++ })
  }
  while (out.length && out[out.length - 1].cls === "c" && out[out.length - 1].text === "") out.pop()
  return out
}

export function DiffLines({ patch, className }: { patch: string; className?: string }) {
  const lines = parsePatch(patch)
  return (
    <div className={cn("overflow-auto font-mono text-[12px] leading-[1.65]", className)}>
      <table className="w-full border-collapse">
        <tbody>
          {lines.map((l, i) => (
            <tr key={i} className={l.cls === "a" ? "bg-add" : l.cls === "d" ? "bg-del" : undefined}>
              <td className="w-10 pr-2 text-right align-top text-faint select-none">{l.cls === "h" ? "" : (l.old ?? "")}</td>
              <td className="w-10 pr-2 text-right align-top text-faint select-none">{l.cls === "h" ? "" : (l.new ?? "")}</td>
              <td className={cn("w-4 align-top select-none", l.cls === "a" ? "text-green" : l.cls === "d" ? "text-red" : "text-faint")}>
                {l.cls === "a" ? "+" : l.cls === "d" ? "−" : ""}
              </td>
              <td className={cn("pr-4 whitespace-pre select-text", l.cls === "h" ? "py-0.5 text-cyan/80" : l.cls === "a" ? "text-green" : l.cls === "d" ? "text-red" : "text-muted-foreground")}>
                {l.text || " "}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

export function DiffStat({ added, removed }: { added: number; removed: number }) {
  return (
    <span className="font-mono text-[11px]">
      <span className="text-green">+{added}</span> <span className="text-red">−{removed}</span>
    </span>
  )
}

/** A file's diff in the chat: header with the path and counts, lines below (collapsible). */
export function DiffCard({ diff, defaultOpen = true }: { diff: FileDiff; defaultOpen?: boolean }) {
  const [open, setOpen] = useState(defaultOpen)
  const rel = useRelative()
  const touch = useTouch()
  return (
    <div className="overflow-hidden rounded-lg border bg-card">
      <button className={cn("flex w-full items-center gap-2 px-3 text-left hover:bg-accent", touch ? "h-11 text-[13px]" : "h-8 text-[12px]")} onClick={() => setOpen(!open)}>
        {open ? <ChevronDown className="size-3 text-faint" /> : <ChevronRight className="size-3 text-faint" />}
        <FileDiffIcon className="size-3.5 text-muted-foreground" />
        <span className="min-w-0 truncate font-mono">{rel(diff.path)}</span>
        <span className="ml-auto shrink-0 pl-1">
          <DiffStat added={diff.added} removed={diff.removed} />
        </span>
      </button>
      {open ? <DiffLines patch={diff.patch} className="max-h-80 border-t py-1" /> : null}
    </div>
  )
}
