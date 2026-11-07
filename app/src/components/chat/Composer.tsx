import { useEffect, useRef, useState } from "react"
import { toast } from "sonner"
import { ArrowUp, Check, ChevronDown, File, Paperclip, Square, X } from "lucide-react"
import { Tip } from "@/components/Tip"
import { AgentGlyph } from "@/components/icons"
import { promptText } from "@/components/Prompt"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { agentKey, agentName, parseAgentKey, type AgentRef } from "@/lib/agents"
import type { ModelOption } from "@/lib/chat"
import { fuzzyPaths, mention, mentionAt } from "@/lib/fuzzy"
import { git } from "@/lib/git"
import { PERMISSION_LABEL, type PermissionMode } from "@/lib/types"
import { useApp } from "@/store/app"
import { useScope } from "@/store/git"
import { cn } from "@/lib/utils"
import { Dictation } from "./Dictation"

export type ComposerProps = {
  agent: AgentRef
  /** Agents to pick from (a new chat). */
  agents?: AgentRef[]
  onAgent?: (a: AgentRef) => void
  model: string
  /** The agent's models; empty while unknown. */
  models?: ModelOption[]
  onModel?: (id: string) => void
  /** Called when the model menu opens, to load the list. */
  onModelsOpen?: () => void
  permission: PermissionMode
  onPermission?: (p: PermissionMode) => void
  running?: boolean
  disabled?: boolean
  placeholder?: string
  onSend?: (text: string, images: Attachment[]) => void
  onStop?: () => void
  className?: string
  autoFocus?: boolean
  /** The agent can't change once a chat started. */
  lockAgent?: boolean
}

const PERMISSIONS: PermissionMode[] = ["ask", "acceptEdits", "plan", "fullAccess"]

/** An image to send with the prompt: its type and base64 bytes. */
export type Attachment = { mime: string; data: string; name: string }

const MAX_IMAGE = 5 * 1024 * 1024
const MAX_IMAGES = 8

function readImage(file: File): Promise<Attachment | null> {
  if (!file.type.startsWith("image/") || file.size > MAX_IMAGE) return Promise.resolve(null)
  return new Promise((resolve) => {
    const r = new FileReader()
    r.onload = () => resolve({ mime: file.type, data: String(r.result).split(",")[1] ?? "", name: file.name || "pasted image" })
    r.onerror = () => resolve(null)
    r.readAsDataURL(file)
  })
}

/** Files for the @ menu, per folder, kept briefly. */
const fileCache = new Map<string, { at: number; files: Promise<string[]> }>()

/** The prompt box: text, then a toolbar with the agent, model and permission chips. */
export function Composer(p: ComposerProps) {
  const settings = useApp((s) => s.settings)
  const scope = useScope()
  const [menu, setMenu] = useState<{ start: number; query: string; items: string[]; index: number } | null>(null)
  const updateMenu = async (value: string, caret: number) => {
    const m = mentionAt(value, caret)
    if (!m || !scope) return setMenu(null)
    const key = `${scope.projectId}|${scope.cwd}`
    let entry = fileCache.get(key)
    if (!entry || Date.now() - entry.at > 30_000) {
      entry = { at: Date.now(), files: git.files(scope).catch(() => [] as string[]) }
      fileCache.set(key, entry)
    }
    const items = fuzzyPaths(await entry.files, m.query, 20)
    setMenu(items.length ? { start: m.start, query: m.query, items, index: 0 } : null)
  }
  const pick = (path: string) => {
    if (!menu) return
    const el = ref.current
    const end = menu.start + 1 + menu.query.length
    const insert = mention(path) + " "
    setText((t) => t.slice(0, menu.start) + insert + t.slice(end))
    setMenu(null)
    requestAnimationFrame(() => {
      el?.focus()
      el?.setSelectionRange(menu.start + insert.length, menu.start + insert.length)
    })
  }
  const [text, setText] = useState("")
  const [images, setImages] = useState<Attachment[]>([])
  const [dragging, setDragging] = useState(false)
  const ref = useRef<HTMLTextAreaElement>(null)
  const picker = useRef<HTMLInputElement>(null)
  const addFiles = async (files: Iterable<File>) => {
    const all = [...files]
    const tooBig = all.filter((f) => f.type.startsWith("image/") && f.size > MAX_IMAGE)
    if (tooBig.length) toast.error(`${tooBig.map((f) => f.name || "That image").join(", ")} is over 5 MB`)
    if (all.some((f) => !f.type.startsWith("image/"))) toast.info("Only images can be attached (PNG, JPEG, GIF, WebP)")
    const read = (await Promise.all(all.map(readImage))).filter((a): a is Attachment => !!a)
    if (read.length) setImages((cur) => [...cur, ...read].slice(0, MAX_IMAGES))
  }
  // "Add to chat" from the side panel puts `@path ` at the cursor.
  useEffect(() => {
    const onInsert = (e: Event) => {
      const add = (e as CustomEvent<string>).detail
      const el = ref.current
      const at = el?.selectionStart ?? text.length
      setText((t) => t.slice(0, at) + (at > 0 && !/\s$/.test(t.slice(0, at)) ? " " : "") + add + t.slice(at))
      requestAnimationFrame(() => el?.focus())
    }
    window.addEventListener("oah:insert", onInsert)
    return () => window.removeEventListener("oah:insert", onInsert)
  }, [text.length])
  const send = () => {
    const t = text.trim()
    if ((!t && !images.length) || p.disabled) return
    p.onSend?.(t || "(see the attached image)", images)
    setText("")
    setImages([])
  }

  return (
    <div
      className={cn("rounded-xl border border-input bg-card focus-within:border-ring/70", dragging && "border-primary", p.className)}
      onDragOver={(e) => {
        if ([...e.dataTransfer.items].some((i) => i.type.startsWith("image/"))) {
          e.preventDefault()
          setDragging(true)
        }
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={(e) => {
        setDragging(false)
        if (e.dataTransfer.files.length) {
          e.preventDefault()
          addFiles(e.dataTransfer.files)
        }
      }}
    >
      {images.length ? (
        <div className="flex flex-wrap gap-1.5 px-3 pt-3">
          {images.map((img, i) => (
            <div key={i} className="rounded-md group/img relative size-14 border bg-background">
              <img src={`data:${img.mime};base64,${img.data}`} alt={img.name} className="size-full object-cover" />
              <button
                className="rounded-full absolute -top-1.5 -right-1.5 hidden size-4 place-items-center bg-foreground text-background group-hover/img:grid"
                aria-label="Remove image"
                onClick={() => setImages((cur) => cur.filter((_, j) => j !== i))}
              >
                <X className="size-3" />
              </button>
            </div>
          ))}
        </div>
      ) : null}
      {menu ? (
        <div className="relative">
          <div className="absolute bottom-0 left-2 z-20 mb-1 max-h-72 w-[min(32rem,calc(100%-1rem))] overflow-y-auto rounded-lg border bg-popover p-1 shadow-xl" onMouseDown={(e) => e.preventDefault()}>
            {menu.items.map((f, i) => {
              const slash = f.lastIndexOf("/")
              return (
                <button key={f} className={cn("flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-[12.5px]", i === menu.index ? "bg-accent text-foreground" : "text-muted-foreground")} onMouseEnter={() => setMenu({ ...menu, index: i })} onClick={() => pick(f)}>
                  <File className="size-3.5 shrink-0 text-faint" />
                  <span className="truncate text-foreground">{f.slice(slash + 1)}</span>
                  <span className="min-w-0 truncate text-[11px] text-faint">{slash > 0 ? f.slice(0, slash) : ""}</span>
                </button>
              )
            })}
          </div>
        </div>
      ) : null}
      <input ref={picker} type="file" accept="image/*" multiple hidden onChange={(e) => e.target.files && addFiles(e.target.files)} />
      <textarea
        ref={ref}
        autoFocus={p.autoFocus}
        value={text}
        rows={3}
        placeholder={p.placeholder ?? "Ask anything, @ to add files"}
        className="block max-h-72 min-h-[76px] w-full resize-none bg-transparent px-3.5 pt-3 text-[length:var(--oah-chat-size,14px)] leading-relaxed outline-none placeholder:text-faint"
        onChange={(e) => {
          setText(e.target.value)
          updateMenu(e.target.value, e.target.selectionStart ?? e.target.value.length)
        }}
        onBlur={() => setTimeout(() => setMenu(null), 150)}
        onPaste={(e) => {
          const files = [...e.clipboardData.files].filter((f) => f.type.startsWith("image/"))
          if (files.length) {
            e.preventDefault()
            addFiles(files)
          }
        }}
        onKeyDown={(e) => {
          if (menu && !e.nativeEvent.isComposing) {
            if (e.key === "ArrowDown" || e.key === "ArrowUp") {
              e.preventDefault()
              const d = e.key === "ArrowDown" ? 1 : -1
              setMenu({ ...menu, index: (menu.index + d + menu.items.length) % menu.items.length })
              return
            }
            if ((e.key === "Enter" || e.key === "Tab") && !e.shiftKey) {
              e.preventDefault()
              pick(menu.items[menu.index])
              return
            }
            if (e.key === "Escape") {
              e.preventDefault()
              e.stopPropagation()
              setMenu(null)
              return
            }
          }
          if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
            e.preventDefault()
            send()
          } else if (e.key === "Escape" && p.running) {
            e.preventDefault()
            p.onStop?.()
          }
        }}
      />
      <div className="flex items-center gap-1 px-1.5 pb-1.5">
        <DropdownMenu>
          <DropdownMenuTrigger asChild disabled={p.lockAgent}>
            <button className="rounded-md flex h-7 items-center gap-1.5 px-2 text-[12px] font-medium hover:bg-accent disabled:hover:bg-transparent">
              <AgentGlyph agent={p.agent.kind} className="size-3.5" />
              {settings ? agentName(settings, p.agent) : ""}
              {p.lockAgent ? null : <ChevronDown className="size-3 text-muted-foreground" />}
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="w-56">
            <DropdownMenuLabel>Agent</DropdownMenuLabel>
            <DropdownMenuRadioGroup value={agentKey(p.agent)} onValueChange={(v) => p.onAgent?.(parseAgentKey(v))}>
              {(p.agents ?? [p.agent]).map((a) => (
                <DropdownMenuRadioItem key={agentKey(a)} value={agentKey(a)}>
                  <AgentGlyph agent={a.kind} className="size-3.5" /> {settings ? agentName(settings, a) : ""}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        <DropdownMenu onOpenChange={(o) => o && p.onModelsOpen?.()}>
          <DropdownMenuTrigger asChild disabled={!p.onModel}>
            <button className="rounded-md flex h-7 max-w-56 items-center gap-1 px-2 font-mono text-[11.5px] text-muted-foreground hover:bg-accent disabled:hover:bg-transparent">
              <span className="truncate">{p.models?.find((m) => m.id === p.model)?.name ?? (p.model || "default model")}</span>
              {p.onModel ? <ChevronDown className="size-3 shrink-0" /> : null}
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="max-h-80 w-72 overflow-y-auto">
            <DropdownMenuLabel>Model</DropdownMenuLabel>
            <DropdownMenuItem onSelect={() => p.onModel?.("")}>
              <span className="w-3.5">{!p.model ? <Check className="size-3.5" /> : null}</span>
              The agent's default
            </DropdownMenuItem>
            {p.models === undefined || p.models.length === 0 ? <div className="px-2 py-1.5 text-[11.5px] text-faint">{p.models === undefined ? "Loading models…" : "The agent didn't list its models."}</div> : null}
            {p.models?.map((m) => (
              <DropdownMenuItem key={m.id} onSelect={() => p.onModel?.(m.id)}>
                <span className="w-3.5">{m.id === p.model ? <Check className="size-3.5" /> : null}</span>
                <span className="min-w-0">
                  <span className="block truncate">{m.name}</span>
                  {m.description ? <span className="block truncate text-[11px] text-faint">{m.description}</span> : null}
                </span>
              </DropdownMenuItem>
            ))}
            <DropdownMenuSeparator />
            <DropdownMenuItem
              onSelect={async () => {
                const id = await promptText({ title: "Use a model", description: "Any model id the agent accepts.", value: p.model, placeholder: "model id", confirm: "Use" })
                if (id) p.onModel?.(id)
              }}
            >
              <span className="w-3.5" /> Other model…
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button className={cn("rounded-md flex h-7 items-center gap-1 px-2 text-[12px] hover:bg-accent", p.permission === "fullAccess" ? "text-red" : p.permission === "plan" ? "text-cyan" : "text-muted-foreground")}>
              {PERMISSION_LABEL[p.permission]}
              <ChevronDown className="size-3" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="w-48">
            <DropdownMenuLabel>Permissions</DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuRadioGroup value={p.permission} onValueChange={(v) => p.onPermission?.(v as PermissionMode)}>
              {PERMISSIONS.map((m) => (
                <DropdownMenuRadioItem key={m} value={m}>
                  {PERMISSION_LABEL[m]}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        <div className="ml-auto flex items-center gap-0.5">
          <Tip content="Attach images (up to 5 MB each, 8 at a time)">
            <button className="rounded-md grid size-7 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Attach image" onClick={() => picker.current?.click()}>
              <Paperclip className="size-3.5" />
            </button>
          </Tip>
          <Dictation onText={(t) => window.dispatchEvent(new CustomEvent("oah:insert", { detail: t + " " }))} />
          {p.running ? (
            <Tip content="Stop (Esc)">
              <button className="rounded-md ml-1 grid size-7 place-items-center bg-foreground text-background hover:opacity-90" aria-label="Stop" onClick={p.onStop}>
                <Square className="size-3 fill-current" />
              </button>
            </Tip>
          ) : (
            <Tip content="Send (Enter)">
              <button
                className="rounded-md ml-1 grid size-7 place-items-center bg-primary text-primary-foreground hover:bg-primary/90 disabled:bg-muted disabled:text-faint"
                aria-label="Send"
                disabled={(!text.trim() && !images.length) || p.disabled}
                onClick={send}
              >
                <ArrowUp className="size-4" />
              </button>
            </Tip>
          )}
        </div>
      </div>
    </div>
  )
}
