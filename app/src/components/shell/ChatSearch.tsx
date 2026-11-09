import { useEffect, useMemo, useState } from "react"
import { MessageSquareText } from "lucide-react"
import { create } from "zustand"
import { AgentGlyph } from "@/components/icons"
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from "@/components/ui/command"
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog"
import { ago, type ChatMeta, type SearchHit } from "@/lib/chat"
import { ipc } from "@/lib/ipc"
import { cn } from "@/lib/utils"
import { termsOf, titleMatches } from "@/lib/search"
import { useApp } from "@/store/app"
import { needsYou, useChats } from "@/store/chats"

const useSearch = create<{ open: boolean }>(() => ({ open: false }))

/** Ctrl+K: find a chat by its title, or by anything said or run in it. */
export const openChatSearch = () => useSearch.setState({ open: true })

/** `text` with the query's words marked. */
function Marked({ text, terms }: { text: string; terms: string[] }) {
  if (!terms.length) return <>{text}</>
  const re = new RegExp(`(${terms.map((t) => t.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})`, "gi")
  return (
    <>
      {text.split(re).map((part, i) =>
        i % 2 ? (
          <mark key={i} className="rounded-[3px] bg-primary/25 text-foreground">
            {part}
          </mark>
        ) : (
          part
        ),
      )}
    </>
  )
}

export function ChatSearch() {
  const open = useSearch((s) => s.open)
  const setOpen = (open: boolean) => useSearch.setState({ open })
  const [query, setQuery] = useState("")
  const [hits, setHits] = useState<{ query: string; hits: SearchHit[] } | null>(null)
  const metas = useChats((s) => s.metas)
  const waitingIds = useChats((s) => (open ? needsYou(s).join(" ") : ""))
  const projects = useApp((s) => s.settings?.projects)
  const projectName = (id: string) => projects?.find((p) => p.id === id)?.name ?? ""
  const terms = termsOf(query)

  useEffect(() => {
    if (!open) {
      setQuery("")
      setHits(null)
    }
  }, [open])

  // Full text, after a pause in typing; an answer to an older query is dropped.
  useEffect(() => {
    const q = query.trim()
    if (!open || !q) return setHits(null)
    let current = true
    const t = window.setTimeout(() => {
      ipc
        .chatSearch(q)
        .then((h) => current && setHits({ query: q, hits: h }))
        .catch(() => current && setHits({ query: q, hits: [] }))
    }, 160)
    return () => {
      current = false
      clearTimeout(t)
    }
  }, [query, open])

  const byTitle = useMemo(() => titleMatches(metas, projectName, query).slice(0, 8), [metas, query, projects])
  const inText = (hits?.hits ?? []).filter((h) => !byTitle.some((m) => m.id === h.chatId))
  const waitingSet = new Set(waitingIds.split(" ").filter(Boolean))
  const waiting = metas.filter((m) => waitingSet.has(m.id))
  const recent = metas.filter((m) => !waitingSet.has(m.id)).slice(0, 8)
  const searching = !!query.trim() && hits?.query !== query.trim()

  const go = (id: string) => {
    setOpen(false)
    const chats = useChats.getState()
    const meta = chats.metas.find((m) => m.id === id)
    if (!meta) return
    if (needsYou(chats).includes(id)) return void chats.answerChat(id)
    useApp.getState().selectProject(meta.projectId)
    void chats.open(id)
  }

  const row = (m: ChatMeta, extra?: React.ReactNode) => (
    <div className="flex min-w-0 flex-1 items-center gap-2">
      <AgentGlyph agent={m.agent} className="size-3.5 shrink-0 opacity-70" />
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13px]">
          <Marked text={m.title || "New chat"} terms={terms} />
        </div>
        {extra}
      </div>
      <span className="shrink-0 text-[11px] text-faint">{projectName(m.projectId)}</span>
      <span className="w-7 shrink-0 text-right font-mono text-[10.5px] text-faint">{ago(m.updatedAt)}</span>
    </div>
  )

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className="top-[18%] translate-y-0 gap-0 overflow-hidden p-0 sm:max-w-xl" showCloseButton={false}>
        <DialogTitle className="sr-only">Search chats</DialogTitle>
        <DialogDescription className="sr-only">Find a chat by its title or by anything said or run in it.</DialogDescription>
        <Command shouldFilter={false} className="[&_[cmdk-group-heading]]:text-[11px] [&_[cmdk-item]]:py-2">
          <CommandInput value={query} onValueChange={setQuery} placeholder="Search chats: titles, messages, commands, files…" className="text-[13px]" />
          <CommandList className="max-h-[min(60vh,420px)]">
            {query.trim() ? (
              <>
                {!searching && !byTitle.length && !inText.length ? <CommandEmpty className="py-8 text-center text-[12.5px] text-muted-foreground">No chat mentions “{query.trim()}”.</CommandEmpty> : null}
                {byTitle.length ? (
                  <CommandGroup heading="Chats">
                    {byTitle.map((m) => (
                      <CommandItem key={m.id} value={`title:${m.id}`} onSelect={() => go(m.id)}>
                        {row(m)}
                      </CommandItem>
                    ))}
                  </CommandGroup>
                ) : null}
                {inText.length ? (
                  <CommandGroup heading="In conversations">
                    {inText.map((h) => {
                      const m = metas.find((x) => x.id === h.chatId)
                      return m ? (
                        <CommandItem key={h.chatId} value={`text:${h.chatId}`} onSelect={() => go(h.chatId)}>
                          {row(
                            m,
                            <div className="flex min-w-0 items-baseline gap-1.5 text-[11.5px] leading-snug text-muted-foreground">
                              <span className="shrink-0 text-faint">{h.part === "user" ? "You:" : h.part === "tool" ? "Ran:" : "Agent:"}</span>
                              <span className={cn("line-clamp-2 break-words", h.part === "tool" && "font-mono text-[11px]")}>
                                <Marked text={h.snippet} terms={terms} />
                              </span>
                            </div>,
                          )}
                        </CommandItem>
                      ) : null
                    })}
                  </CommandGroup>
                ) : null}
                {searching ? <div className="px-3 py-2 text-[11.5px] text-faint">Searching conversations…</div> : null}
              </>
            ) : (
              <>
                {waiting.length ? (
                  <CommandGroup heading="Needs you">
                    {waiting.map((m) => (
                      <CommandItem key={m.id} value={`wait:${m.id}`} onSelect={() => go(m.id)}>
                        {row(m)}
                      </CommandItem>
                    ))}
                  </CommandGroup>
                ) : null}
                {recent.length ? (
                  <CommandGroup heading="Recent">
                    {recent.map((m) => (
                      <CommandItem key={m.id} value={`recent:${m.id}`} onSelect={() => go(m.id)}>
                        {row(m)}
                      </CommandItem>
                    ))}
                  </CommandGroup>
                ) : (
                  <div className="flex flex-col items-center gap-2 py-8 text-[12.5px] text-muted-foreground">
                    <MessageSquareText className="size-5 text-faint" />
                    No chats yet.
                  </div>
                )}
              </>
            )}
          </CommandList>
        </Command>
      </DialogContent>
    </Dialog>
  )
}
