// Message box and everything that takes its place: the permission prompt, the
// model's questions, the API-key field, plus the slash-command palette. Keys
// follow the TUI (src/ui/App.tsx).
import { ArrowUp, ChevronDown, Copy, ExternalLink, KeyRound, Loader2, Paperclip, ShieldAlert, Square } from "lucide-react"
import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Card } from "@/components/ui/card"
import { Command, CommandGroup, CommandItem, CommandList } from "@/components/ui/command"
import { Input } from "@/components/ui/input"
import { Kbd } from "@/components/ui/kbd"
import { Textarea } from "@/components/ui/textarea"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { useClient } from "@/lib/client"
import { matchCommands, type Command as Cmd } from "@/lib/format"
import { cn } from "@/lib/utils"
import { modKey } from "./hooks"
import { DiffLines } from "./Transcript"

const PERM = {
  auto: { label: "auto", className: "text-green" },
  ask: { label: "always ask", className: "text-warn" },
  allow: { label: "always allow", className: "text-red" },
} as const
const EFFORT_CLASS: Record<string, string> = { high: "text-primary", xhigh: "text-orange", max: "text-orange", low: "text-muted-foreground", minimal: "text-muted-foreground" }

function Palette({ items, sel, onPick }: { items: Cmd[]; sel: number; onPick: (c: Cmd) => void }) {
  return (
    <Command shouldFilter={false} value={items[sel]?.name ?? ""} className="h-auto rounded-xl border shadow-2xl shadow-black/50">
      <CommandList className="max-h-72">
        <CommandGroup heading="Commands">
          {items.map((c) => (
            <CommandItem key={c.name} value={c.name} onMouseDown={(e) => e.preventDefault()} onSelect={() => onPick(c)} className="group gap-3">
              <span className="min-w-28 font-mono text-[13px] text-foreground group-data-[selected=true]:text-primary">/{c.name}</span>
              {c.args ? <span className="font-mono text-xs text-faint">{c.args}</span> : null}
              <span className="truncate text-muted-foreground">{c.desc}</span>
            </CommandItem>
          ))}
        </CommandGroup>
      </CommandList>
    </Command>
  )
}

function PermissionCard() {
  const c = useClient()
  const p = c.state!.permission!
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => ref.current?.focus(), [p.title])
  const answer = (reply: "once" | "always" | "deny") => c.send({ op: "permission", reply })
  const onKey = (e: KeyboardEvent) => {
    const k = e.key.toLowerCase()
    if (k === "y" || k === "enter") answer("once")
    else if (k === "a") answer("always")
    else if (k === "n" || k === "escape") answer("deny")
    else return
    e.preventDefault()
    e.stopPropagation()
  }
  return (
    <Card ref={ref} tabIndex={-1} onKeyDown={onKey} role="alertdialog" aria-label="Permission request" className="gap-3 border-warn/60 px-4 py-3.5 outline-none">
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <ShieldAlert className="size-4 text-warn" />
        <span className="font-semibold text-warn">Permission</span>· {p.kind}
        {c.state!.waitingPrompts ? <span>· {c.state!.waitingPrompts} more waiting</span> : null}
      </div>
      <p className="font-semibold select-text [overflow-wrap:anywhere]">{p.title}</p>
      {p.preview.length ? <DiffLines lines={p.preview} max={40} /> : null}
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" onClick={() => answer("once")}><Kbd className="bg-primary-foreground/15 text-primary-foreground">Y</Kbd>Allow once</Button>
        <Button size="sm" variant="secondary" onClick={() => answer("always")}><Kbd>A</Kbd>Always allow</Button>
        <Button size="sm" variant="secondary" onClick={() => answer("deny")}><Kbd>N</Kbd>Deny</Button>
        <span className="text-xs text-faint">Esc denies</span>
      </div>
    </Card>
  )
}

function QuestionCard({ typing }: { typing: boolean }) {
  const c = useClient()
  const q = c.state!.question!
  const [sel, setSel] = useState(0)
  const [checked, setChecked] = useState<Set<number>>(new Set())
  useEffect(() => {
    setSel(0)
    setChecked(new Set())
  }, [q.question])
  const toggle = (i: number) => {
    const n = new Set(checked)
    n.has(i) ? n.delete(i) : n.add(i)
    setChecked(n)
  }
  const choice = () => {
    const picked = q.multiSelect && checked.size ? [...checked].sort((a, b) => a - b) : [sel]
    return picked.map((i) => q.options[i]?.label ?? "").filter(Boolean).join(", ")
  }
  const answer = (text: string) => c.send({ op: "answer", answer: text })
  // Option keys work while the message box is empty (like the TUI).
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (typing || !q.options.length || e.defaultPrevented) return
      const n = q.options.length
      if (e.key === "ArrowUp" || e.key === "ArrowDown") setSel((s) => (s + (e.key === "ArrowUp" ? n - 1 : 1)) % n)
      else if (e.key === " " && q.multiSelect) toggle(sel)
      else if (/^[1-9]$/.test(e.key) && Number(e.key) <= n) {
        const i = Number(e.key) - 1
        setSel(i)
        if (q.multiSelect) toggle(i)
        else answer(q.options[i]!.label)
      } else if (e.key === "Enter" && !e.shiftKey) answer(choice())
      else return
      e.preventDefault()
    }
    window.addEventListener("keydown", onKey, true)
    return () => window.removeEventListener("keydown", onKey, true)
  })
  return (
    <Card role="dialog" aria-label="oah has a question" className="gap-3 border-primary/50 px-4 py-3.5">
      <div className="text-xs font-semibold text-primary">oah has a question</div>
      <p className="font-semibold select-text">{q.question}</p>
      {q.options.length ? (
        <div className="flex flex-col gap-1.5">
          {q.options.map((o, i) => (
            <Button
              key={i}
              variant="outline"
              className={cn("h-auto justify-start gap-2.5 bg-card py-2 text-left font-normal whitespace-normal", i === sel && !typing && "border-primary")}
              onClick={() => (q.multiSelect ? (setSel(i), toggle(i)) : answer(o.label))}
            >
              {q.multiSelect ? <span className="font-mono text-primary">{checked.has(i) ? "☒" : "☐"}</span> : null}
              <span className="font-mono text-xs text-muted-foreground">{i + 1}.</span>
              <span className="font-medium">{o.label}</span>
              {o.description ? <span className="text-xs text-muted-foreground">{o.description}</span> : null}
            </Button>
          ))}
        </div>
      ) : null}
      <div className="flex flex-wrap items-center gap-2">
        {q.multiSelect && q.options.length ? <Button size="sm" onClick={() => answer(choice())}>Submit</Button> : null}
        <Button size="sm" variant="secondary" onClick={() => answer("")}>Dismiss</Button>
        <span className="text-xs text-faint">
          {q.options.length ? (q.multiSelect ? "↑↓ move · space or 1-9 tick · enter submit · " : "↑↓ choose · 1-9 pick · enter submit · ") : ""}or type your own answer below
        </span>
      </div>
    </Card>
  )
}

function LoginCard({ url }: { url: string }) {
  const c = useClient()
  return (
    <Alert className="border-primary/30 bg-card">
      <Loader2 className="animate-spin text-primary" />
      <AlertTitle>Waiting for sign-in in your browser</AlertTitle>
      <AlertDescription className="gap-2">
        <span className="block w-full truncate font-mono text-xs select-text" title={url}>{url}</span>
        <div className="flex flex-wrap gap-2">
          <Button size="sm" variant="outline" onClick={() => void c.t.copy(url)}><Copy />Copy link</Button>
          <Button size="sm" variant="outline" onClick={() => void c.t.openUrl(url)}><ExternalLink />Open</Button>
          <Button size="sm" variant="ghost" onClick={() => c.send({ op: "interrupt" })}>Cancel</Button>
        </div>
      </AlertDescription>
    </Alert>
  )
}

export function Composer() {
  const c = useClient()
  const s = c.state!
  const ta = useRef<HTMLTextAreaElement>(null)
  const [text, setText] = useState("")
  const [palIdx, setPalIdx] = useState(0)
  const [histIdx, setHistIdx] = useState<number | null>(null)
  const draft = useRef("")
  const syncTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const masked = s.inputMode !== "prompt"
  const keyProvider = masked ? s.auth.providers.find((p) => `key:${p.id}` === s.inputMode)?.label ?? "API" : ""

  const set = (v: string, sync = true) => {
    setText(v)
    if (!sync) return
    if (syncTimer.current) clearTimeout(syncTimer.current)
    syncTimer.current = setTimeout(() => c.send({ op: "input", text: v }), 150)
  }

  // The engine can replace the text (history restore, /skills) or insert at the cursor ([Image #N]).
  useEffect(() => {
    c.onInput = (v) => {
      set(v, false)
      requestAnimationFrame(() => ta.current?.setSelectionRange(v.length, v.length))
    }
    c.onInsert = (v) => {
      const el = ta.current
      const cur = el?.value ?? ""
      const a = el?.selectionStart ?? cur.length
      const b = el?.selectionEnd ?? cur.length
      set(cur.slice(0, a) + v + cur.slice(b))
      requestAnimationFrame(() => {
        el?.focus()
        el?.setSelectionRange(a + v.length, a + v.length)
      })
    }
    return () => {
      c.onInput = null
      c.onInsert = null
    }
  }, [c])

  // Grow with the text.
  useLayoutEffect(() => {
    const el = ta.current
    if (!el) return
    el.style.height = "0px"
    el.style.height = `${Math.min(el.scrollHeight, window.innerHeight * 0.4)}px`
  }, [text])

  // Focus the box when a prompt closes or the session changes.
  useEffect(() => {
    if (!s.permission && !s.picker) ta.current?.focus()
  }, [s.permission, s.picker, s.session.id, masked])

  const palette = !masked && !s.question && text.startsWith("/") && !/\s/.test(text) && c.hello ? matchCommands(text.slice(1), c.hello.commands, s.skills) : []
  const pal = Math.min(palIdx, Math.max(0, palette.length - 1))

  const submit = (now = false) => {
    if (masked) {
      c.send({ op: "submit", text })
      return set("", false)
    }
    c.submit(text, now)
    set("", false)
    setHistIdx(null)
  }

  const runCommand = (name: string) => {
    set("", false)
    setPalIdx(0)
    c.submit(`/${name}`)
  }

  const onKey = (e: KeyboardEvent<HTMLTextAreaElement | HTMLInputElement>) => {
    // Already handled (a question's option keys).
    if (e.defaultPrevented) return
    const el = e.currentTarget
    if (palette.length) {
      if (e.key === "ArrowUp" || e.key === "ArrowDown") {
        e.preventDefault()
        return setPalIdx((pal + (e.key === "ArrowUp" ? palette.length - 1 : 1)) % palette.length)
      }
      if (e.key === "Tab" && !e.shiftKey) {
        e.preventDefault()
        return set(`/${palette[pal]!.name} `)
      }
      if (e.key === "Escape") {
        e.preventDefault()
        e.stopPropagation()
        return set("")
      }
      if (e.key === "Enter" && !e.shiftKey && !e.altKey) {
        e.preventDefault()
        return runCommand(palette[pal]!.name)
      }
    }
    if (e.key === "Enter" && !e.shiftKey && !e.altKey) {
      e.preventDefault()
      return submit(e.ctrlKey || e.metaKey)
    }
    if (e.key === "Escape" && masked) {
      e.preventDefault()
      e.stopPropagation()
      set("", false)
      return c.send({ op: "cancelKey" })
    }
    if (masked) return
    if (e.key === "Tab") {
      e.preventDefault()
      return e.shiftKey
        ? c.send({ op: "permissionLevel", value: s.settings.permission === "auto" ? "ask" : s.settings.permission === "ask" ? "allow" : "auto" })
        : c.send({ op: "mode", value: s.settings.mode === "plan" ? "build" : "plan" })
    }
    // ↑ / ↓ walk prompt history from the first / last line.
    const v = el.value
    const atFirst = !v.slice(0, el.selectionStart ?? 0).includes("\n")
    const atLast = !v.slice(el.selectionEnd ?? v.length).includes("\n")
    const h = c.history
    if (e.key === "ArrowUp" && atFirst && h.length && !s.question) {
      e.preventDefault()
      if (histIdx === null) draft.current = text
      const i = histIdx === null ? h.length - 1 : Math.max(0, histIdx - 1)
      setHistIdx(i)
      return set(h[i]!)
    }
    if (e.key === "ArrowDown" && atLast && histIdx !== null) {
      e.preventDefault()
      if (histIdx + 1 >= h.length) {
        setHistIdx(null)
        return set(draft.current)
      }
      setHistIdx(histIdx + 1)
      return set(h[histIdx + 1]!)
    }
  }

  const onPaste = (e: React.ClipboardEvent) => {
    // An image on the clipboard (and no text): attach it, like ctrl+v in the TUI.
    const types = [...e.clipboardData.types]
    if (!e.clipboardData.getData("text/plain") && (types.includes("Files") || [...e.clipboardData.items].some((i) => i.type.startsWith("image/")))) {
      e.preventDefault()
      c.send({ op: "pasteImage" })
    }
  }

  const attach = async () => {
    for (const path of await c.t.pickImages()) c.send({ op: "attach", path })
  }

  const busy = s.busy
  const placeholder = masked
    ? "Paste the key and press Enter · Esc to cancel"
    : s.question
      ? "Or type your own answer and press Enter"
      : busy
        ? busy.kind === "turn"
          ? `Enter to queue · ${modKey}Enter to send now · Esc to interrupt`
          : "Type to queue a message · Esc to interrupt"
        : s.settings.mode === "plan"
          ? "Describe what to plan… (Tab for build mode)"
          : "Ask anything · / for commands · # to remember"
  const canSend = !!text.trim() || (!!busy && s.queue.length > 0)
  const perm = PERM[s.settings.permission]

  if (s.permission) return <PermissionCard />

  return (
    <>
      {s.loginUrl ? <LoginCard url={s.loginUrl} /> : null}
      {s.question ? <QuestionCard typing={text.length > 0} /> : null}
      {palette.length ? <Palette items={palette} sel={pal} onPick={(cmd) => runCommand(cmd.name)} /> : null}
      <div
        className={cn(
          "flex flex-col gap-2.5 rounded-2xl border bg-card px-4 pt-3 pb-2.5 shadow-lg shadow-black/30 transition-colors",
          masked ? "border-warn/70" : s.settings.mode === "plan" ? "border-cyan/25 focus-within:border-cyan/50" : "border-primary/20 focus-within:border-primary/45",
        )}
      >
        {masked ? (
          <div className="flex items-center gap-3 py-1">
            <KeyRound className="size-4 text-warn" />
            <span className="text-xs font-semibold text-warn">{keyProvider} key</span>
            <Input type="password" autoFocus aria-label={`${keyProvider} API key`} placeholder={placeholder} value={text} onChange={(e) => set(e.target.value, false)} onKeyDown={onKey} className="h-8 border-0 bg-transparent px-0 font-mono shadow-none focus-visible:ring-0 dark:bg-transparent" />
          </div>
        ) : (
          <>
            <label className="sr-only" htmlFor="composer">Message</label>
            <Textarea
              id="composer"
              ref={ta}
              rows={1}
              placeholder={placeholder}
              value={text}
              onChange={(e) => (set(e.target.value), setHistIdx(null), setPalIdx(0))}
              onKeyDown={onKey}
              onPaste={onPaste}
              spellCheck
              className="max-h-[40vh] min-h-6 resize-none border-0 bg-transparent p-0 text-sm shadow-none placeholder:text-faint focus-visible:ring-0 dark:bg-transparent"
            />
            <div className="flex flex-wrap items-center gap-1.5">
              <ToggleGroup type="single" variant="outline" size="sm" value={s.settings.mode} onValueChange={(v) => v && c.send({ op: "mode", value: v as "plan" | "build" })} aria-label="Mode">
                <ToggleGroupItem value="plan" className="px-3 text-xs data-[state=on]:bg-cyan data-[state=on]:text-background" title="Plan: read-only research (Tab)">Plan</ToggleGroupItem>
                <ToggleGroupItem value="build" className="px-3 text-xs data-[state=on]:bg-primary data-[state=on]:text-primary-foreground" title="Build: edit files and run commands (Tab)">Build</ToggleGroupItem>
              </ToggleGroup>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button variant="outline" size="sm" className={cn("max-w-56 bg-transparent text-xs", !s.model.ready && "text-warn")} onClick={() => c.submit("/model")}>
                    <span className="truncate">{s.model.name}</span>
                    <ChevronDown className="opacity-60" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>{s.model.providerLabel} · {modKey}T</TooltipContent>
              </Tooltip>
              {s.model.efforts.length ? (
                <Button variant="outline" size="sm" className="bg-transparent text-xs" onClick={() => c.submit("/effort")} title="Reasoning effort">
                  <span className={EFFORT_CLASS[s.settings.effort]}>{s.settings.effort}</span>
                  <ChevronDown className="opacity-60" />
                </Button>
              ) : null}
              <Button variant="outline" size="sm" className={cn("bg-transparent text-xs", perm.className)} onClick={() => c.submit("/permissions")} title="Permissions (Shift+Tab cycles)">
                ● {perm.label}
              </Button>
              <div className="flex-1" />
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button variant="ghost" size="icon" className="size-8 text-muted-foreground" aria-label="Attach images" onClick={() => void attach()}>
                    <Paperclip />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>Attach images (or paste / drop them)</TooltipContent>
              </Tooltip>
              {busy && !text.trim() ? (
                <Button variant="secondary" size="icon" className="size-8 rounded-lg" aria-label="Stop" title="Stop (Esc)" onClick={() => c.send({ op: "interrupt" })}>
                  <Square className="fill-current" />
                </Button>
              ) : (
                <Button size="icon" className="size-8 rounded-lg" aria-label="Send" title="Send (Enter)" disabled={!canSend} onClick={() => submit()}>
                  <ArrowUp />
                </Button>
              )}
            </div>
          </>
        )}
      </div>
    </>
  )
}
