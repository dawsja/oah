import { useEffect, useRef, useState } from "react"
import { toast } from "sonner"
import { ArrowUp, Check, ChevronDown, File, Paperclip, Slash, Sparkles, Square, X } from "lucide-react"
import { Tip } from "@/components/Tip"
import { openImages } from "@/components/ImageView"
import { AgentGlyph } from "@/components/icons"
import { promptText } from "@/components/Prompt"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { agentKey, agentName, parseAgentKey, type AgentRef } from "@/lib/agents"
import { effortLabel, type ContextFill, type ModelOption, type SlashCommand } from "@/lib/chat"
import { ContextRing } from "./ContextRing"
import { filterCommands, menuCommands, ownCommand, slashQuery, type MenuCommand, type OwnCommand } from "@/lib/commands"
import { fuzzyPaths, mention, mentionAt } from "@/lib/fuzzy"
import { git } from "@/lib/git"
import { PERMISSION_LABEL, type PermissionMode } from "@/lib/types"
import { PERMISSION_ICON, PERMISSION_TONE } from "@/components/permission"
import { useApp } from "@/store/app"
import { useScope } from "@/store/git"
import { drafts } from "@/store/drafts"
import { cn } from "@/lib/utils"
import { Dictation } from "./Dictation"

export type ComposerProps = {
  agent: AgentRef
  /** Agents to pick from (a new chat). */
  agents?: AgentRef[]
  onAgent?: (a: AgentRef) => void
  model: string
  /** Reasoning effort; empty: the model's default. */
  effort?: string
  /** How full the chat's context is (an open chat). */
  context?: ContextFill | null
  /** The agent's models; empty while unknown. */
  models?: ModelOption[]
  /** Picks a model and its effort (empty: the model's default). */
  onModel?: (id: string, effort: string) => void
  /** Called when the model menu or the / menu opens, to load the agent's lists. */
  onModelsOpen?: () => void
  /** The agent's slash commands (the / menu adds oah's own). */
  commands?: SlashCommand[]
  /** `/clear` and `/compact`, in a chat that has started. */
  onClear?: () => void
  onCompact?: (instructions: string) => void
  permission: PermissionMode
  onPermission?: (p: PermissionMode) => void
  running?: boolean
  disabled?: boolean
  placeholder?: string
  /** `steer` (Ctrl/⌘+Enter): send it now, into the turn that's running. */
  onSend?: (text: string, images: Attachment[], how: "send" | "steer") => void
  onStop?: () => void
  /** Ctrl/⌘+Enter with nothing typed while the agent works: the first queued message goes now. */
  onSteerQueued?: () => void
  /** Where what's typed is kept until it's sent (see `draftKey`); mount one composer per key. */
  draftKey?: string
  className?: string
  autoFocus?: boolean
  /** The agent can't change once a chat started. */
  lockAgent?: boolean
}

const PERMISSIONS: PermissionMode[] = ["ask", "acceptEdits", "plan", "fullAccess"]

/** An image to send with the prompt: its type and base64 bytes. */
export type Attachment = { mime: string; data: string; name: string }

const MAX_IMAGE = 5 * 1024 * 1024
const MAC = /Mac|iPhone|iPad/.test(navigator.userAgent)
/** The send-now chord's modifier, as shown. */
export const STEER_KEY = MAC ? "⌘" : "Ctrl"

/** Puts a message back in the composer to edit (a queued one taken off the queue). */
export function editInComposer(text: string, images: Attachment[]) {
  window.dispatchEvent(new CustomEvent("oah:edit", { detail: { text, images } }))
}
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
  const PermissionIcon = PERMISSION_ICON[p.permission]
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
  const inChat = !!p.onClear
  const [slashState, setSlash] = useState<{ query: string; index: number } | null>(null)
  // Esc closes the / menu until the prompt changes.
  const [slashDismissed, setSlashDismissed] = useState<string | null>(null)
  const updateSlash = (value: string, caret: number) => {
    const q = slashQuery(value, caret)
    if (q === null || slashDismissed === value) return setSlash(null)
    if (q === "") p.onModelsOpen?.()
    setSlash({ query: q, index: 0 })
  }
  // Worked out on each render, so the list fills in when the agent's commands arrive.
  const slashItems = slashState ? filterCommands(menuCommands(p.agent.kind, p.commands ?? [], inChat), slashState.query).slice(0, 60) : []
  const slash = slashState && slashItems.length ? { items: slashItems, index: Math.min(slashState.index, slashItems.length - 1) } : null
  const [modelOpen, setModelOpen] = useState(false)
  const runOwn = (own: OwnCommand, args: string) => {
    if (own === "model") {
      if (!args) {
        p.onModelsOpen?.()
        setModelOpen(true)
        return
      }
      const a = args.toLowerCase()
      const m =
        p.models?.find((x) => x.id.toLowerCase() === a || x.name.toLowerCase() === a) ??
        p.models?.find((x) => x.id.toLowerCase().includes(a) || x.name.toLowerCase().includes(a))
      p.onModel?.(m?.id ?? args, p.effort ?? "")
      toast.success(`Model: ${m?.name ?? args}`)
    } else if (own === "plan") {
      p.onPermission?.("plan")
      toast.success("Plan mode: the agent proposes a plan before changing anything")
    } else if (own === "clear") p.onClear?.()
    else if (own === "compact") p.onCompact?.(args)
  }
  /** Enter runs a command that needs nothing more; Tab (or a command that takes arguments)
   *  puts `/name ` in the prompt to finish. */
  const pickCommand = (c: MenuCommand, run: boolean) => {
    const el = ref.current
    setSlash(null)
    if (run && c.own) {
      setText("")
      runOwn(c.own, "")
      return
    }
    if (run && !c.skill && !c.hint) {
      setText("")
      p.onSend?.(`/${c.name}`, [], "send")
      return
    }
    // Codex skills are mentioned with $ in the message; they don't run by themselves.
    const insert = c.skill && p.agent.kind === "codex" ? `$${c.name} ` : `/${c.name} `
    setText(insert)
    requestAnimationFrame(() => {
      el?.focus()
      el?.setSelectionRange(insert.length, insert.length)
    })
  }
  const [text, setText] = useState(() => (p.draftKey ? drafts.get(p.draftKey).text : ""))
  const [images, setImages] = useState<Attachment[]>(() => (p.draftKey ? drafts.get(p.draftKey).images : []))
  useEffect(() => {
    if (p.draftKey) drafts.set(p.draftKey, text, images)
  }, [p.draftKey, text, images])
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
  // A queued message back to edit: it goes before whatever is typed already.
  useEffect(() => {
    const onEdit = (e: Event) => {
      const back = (e as CustomEvent<{ text: string; images: Attachment[] }>).detail
      setText((t) => (t.trim() ? `${back.text}\n\n${t}` : back.text))
      setImages((cur) => [...back.images, ...cur].slice(0, MAX_IMAGES))
      requestAnimationFrame(() => {
        const el = ref.current
        el?.focus()
        el?.setSelectionRange(back.text.length, back.text.length)
      })
    }
    window.addEventListener("oah:edit", onEdit)
    return () => window.removeEventListener("oah:edit", onEdit)
  }, [])
  const send = (how: "send" | "steer" = "send") => {
    const t = text.trim()
    if (!t && !images.length && how === "steer" && p.running) return p.onSteerQueued?.()
    if ((!t && !images.length) || p.disabled) return
    const own = images.length ? null : ownCommand(t, inChat)
    if (own) {
      runOwn(own.own, own.args)
      setText("")
      return
    }
    p.onSend?.(t || "(see the attached image)", images, how)
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
              <button className="size-full" aria-label={`View ${img.name}`} onClick={() => openImages(images.map((x) => ({ src: `data:${x.mime};base64,${x.data}`, name: x.name })), i)}>
                <img src={`data:${img.mime};base64,${img.data}`} alt={img.name} className="size-full object-cover" />
              </button>
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
      {slash ? (
        <div className="relative">
          <div className="absolute bottom-0 left-2 z-20 mb-1 max-h-80 w-[min(36rem,calc(100%-1rem))] overflow-y-auto rounded-lg border bg-popover p-1 shadow-xl" onMouseDown={(e) => e.preventDefault()} role="listbox" aria-label="Commands">
            {slash.items.map((c, i) => (
              <button
                key={`${c.skill ? "$" : "/"}${c.name}`}
                ref={i === slash.index ? (el) => el?.scrollIntoView({ block: "nearest" }) : undefined}
                role="option"
                aria-selected={i === slash.index}
                className={cn("flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[12.5px]", i === slash.index ? "bg-accent text-foreground" : "text-muted-foreground")}
                onMouseEnter={() => setSlash({ query: slashState!.query, index: i })}
                onClick={() => pickCommand(c, true)}
              >
                {c.skill ? <Sparkles className="size-3.5 shrink-0 text-faint" /> : <Slash className={cn("size-3.5 shrink-0", c.own ? "text-foreground" : "text-faint")} />}
                <span className="shrink-0 font-mono text-foreground">{c.name}</span>
                {c.hint ? <span className="max-w-40 shrink-0 truncate font-mono text-[11px] text-faint">{c.hint}</span> : null}
                <span className="min-w-0 flex-1 truncate text-[11.5px] text-faint">{c.description}</span>
                {c.skill ? <span className="shrink-0 rounded-sm border px-1 text-[10px] text-faint">skill</span> : null}
              </button>
            ))}
          </div>
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
        placeholder={p.placeholder ?? "Ask anything, @ to add files, / for commands"}
        className="block max-h-72 min-h-[76px] w-full resize-none bg-transparent px-3.5 pt-3 text-[length:var(--oah-chat-size,14px)] leading-relaxed outline-none placeholder:text-faint"
        onChange={(e) => {
          setText(e.target.value)
          const caret = e.target.selectionStart ?? e.target.value.length
          updateMenu(e.target.value, caret)
          updateSlash(e.target.value, caret)
        }}
        onBlur={() =>
          setTimeout(() => {
            setMenu(null)
            setSlash(null)
          }, 150)
        }
        onPaste={(e) => {
          const files = [...e.clipboardData.files].filter((f) => f.type.startsWith("image/"))
          if (files.length) {
            e.preventDefault()
            addFiles(files)
          }
        }}
        onKeyDown={(e) => {
          // An approval or question card above took this key.
          if (e.defaultPrevented) return
          if (slash && !e.nativeEvent.isComposing) {
            if (e.key === "ArrowDown" || e.key === "ArrowUp") {
              e.preventDefault()
              const d = e.key === "ArrowDown" ? 1 : -1
              setSlash({ query: slashState!.query, index: (slash.index + d + slash.items.length) % slash.items.length })
              return
            }
            if ((e.key === "Enter" || e.key === "Tab") && !e.shiftKey) {
              e.preventDefault()
              pickCommand(slash.items[slash.index], e.key === "Enter")
              return
            }
            if (e.key === "Escape") {
              e.preventDefault()
              e.stopPropagation()
              setSlash(null)
              setSlashDismissed(text)
              return
            }
          }
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
            send((MAC ? e.metaKey : e.ctrlKey) ? "steer" : "send")
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
        <DropdownMenu
          open={modelOpen}
          onOpenChange={(o) => {
            setModelOpen(o)
            if (o) p.onModelsOpen?.()
          }}
        >
          <DropdownMenuTrigger asChild disabled={!p.onModel}>
            <button className="rounded-md flex h-7 max-w-56 items-center gap-1 px-2 font-mono text-[11.5px] text-muted-foreground hover:bg-accent disabled:hover:bg-transparent">
              <span className="truncate">{p.models?.find((m) => m.id === p.model)?.name ?? (p.model || "default model")}</span>
              {p.effort ? <span className="shrink-0 text-faint">· {effortLabel(p.effort)}</span> : null}
              {p.onModel ? <ChevronDown className="size-3 shrink-0" /> : null}
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="max-h-80 w-72 overflow-y-auto">
            <DropdownMenuLabel>Model{p.models?.some((m) => m.efforts?.length) ? <span className="ml-1.5 font-normal text-faint">→ for effort</span> : null}</DropdownMenuLabel>
            <DropdownMenuItem onSelect={() => p.onModel?.("", "")}>
              <span className="w-3.5">{!p.model ? <Check className="size-3.5" /> : null}</span>
              The agent's default
            </DropdownMenuItem>
            {p.models === undefined || p.models.length === 0 ? <div className="px-2 py-1.5 text-[11.5px] text-faint">{p.models === undefined ? "Loading models…" : "The agent didn't list its models."}</div> : null}
            {p.models?.map((m) =>
              m.efforts?.length ? (
                <ModelWithEffort key={m.id} model={m} selected={m.id === p.model} effort={m.id === p.model ? (p.effort ?? "") : ""} onPick={(e) => p.onModel?.(m.id, e)} />
              ) : (
                <DropdownMenuItem key={m.id} onSelect={() => p.onModel?.(m.id, "")}>
                  <span className="w-3.5">{m.id === p.model ? <Check className="size-3.5" /> : null}</span>
                  <ModelText model={m} />
                </DropdownMenuItem>
              ),
            )}
            <DropdownMenuSeparator />
            <DropdownMenuItem
              onSelect={async () => {
                const id = await promptText({ title: "Use a model", description: "Any model id the agent accepts.", value: p.model, placeholder: "model id", mono: true, confirm: "Use" })
                if (id) p.onModel?.(id, "")
              }}
            >
              <span className="w-3.5" /> Other model…
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button className={cn("rounded-md flex h-7 items-center gap-1 px-2 text-[12px] hover:bg-accent", PERMISSION_TONE[p.permission] || "text-muted-foreground")}>
              <PermissionIcon className="size-3.5" />
              {PERMISSION_LABEL[p.permission]}
              <ChevronDown className="size-3" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="w-48">
            <DropdownMenuLabel>Permissions</DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuRadioGroup value={p.permission} onValueChange={(v) => p.onPermission?.(v as PermissionMode)}>
              {PERMISSIONS.map((m) => {
                const Icon = PERMISSION_ICON[m]
                return (
                  <DropdownMenuRadioItem key={m} value={m}>
                    <Icon className={cn("size-3.5", PERMISSION_TONE[m] || "text-muted-foreground")} />
                    <span className={cn(m === p.permission && PERMISSION_TONE[m])}>{PERMISSION_LABEL[m]}</span>
                  </DropdownMenuRadioItem>
                )
              })}
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        <div className="ml-auto flex items-center gap-0.5">
          <ContextRing context={p.context ?? null} />
          <Tip content="Attach images (up to 5 MB each, 8 at a time)">
            <button className="rounded-md grid size-7 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Attach image" onClick={() => picker.current?.click()}>
              <Paperclip className="size-3.5" />
            </button>
          </Tip>
          <Dictation onText={(t) => window.dispatchEvent(new CustomEvent("oah:insert", { detail: t + " " }))} />
          {p.running && (text.trim() || images.length) ? (
            <Tip content={`Queue: sends when the agent finishes (Enter) · Send now (${STEER_KEY}+Enter)`}>
              <button className="rounded-md ml-1 grid size-7 place-items-center bg-primary text-primary-foreground hover:bg-primary/90" aria-label="Queue" onClick={() => send()}>
                <ArrowUp className="size-4" />
              </button>
            </Tip>
          ) : null}
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
                onClick={() => send()}
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

function ModelText({ model }: { model: ModelOption }) {
  return (
    <span className="min-w-0 flex-1">
      <span className="block truncate">{model.name}</span>
      {model.description ? <span className="block truncate text-[11px] text-faint">{model.description}</span> : null}
    </span>
  )
}

/** A model whose effort can be picked: hover or click it, and the levels open to the right. */
function ModelWithEffort({ model, selected, effort, onPick }: { model: ModelOption; selected: boolean; effort: string; onPick: (effort: string) => void }) {
  return (
    <DropdownMenuSub>
      <DropdownMenuSubTrigger className="gap-2">
        <span className="w-3.5">{selected ? <Check className="size-3.5" /> : null}</span>
        <ModelText model={model} />
        {selected && effort ? <span className="shrink-0 text-[11px] text-muted-foreground">{effortLabel(effort)}</span> : null}
      </DropdownMenuSubTrigger>
      <DropdownMenuSubContent className="w-48">
        <DropdownMenuLabel className="text-[11px] font-normal text-faint">{model.name} · effort</DropdownMenuLabel>
        <DropdownMenuItem onSelect={() => onPick("")}>
          <span className="w-3.5">{selected && !effort ? <Check className="size-3.5" /> : null}</span>
          <span className="flex-1">Default</span>
          {model.defaultEffort ? <span className="text-[11px] text-faint">{effortLabel(model.defaultEffort)}</span> : null}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        {model.efforts.map((e) => (
          <DropdownMenuItem key={e} onSelect={() => onPick(e)}>
            <span className="w-3.5">{selected && effort === e ? <Check className="size-3.5" /> : null}</span>
            <span className="flex-1">{effortLabel(e)}</span>
            <EffortBars level={model.efforts.indexOf(e)} of={model.efforts.length} />
          </DropdownMenuItem>
        ))}
      </DropdownMenuSubContent>
    </DropdownMenuSub>
  )
}

/** A small rising meter: how much thinking this level asks for. */
function EffortBars({ level, of }: { level: number; of: number }) {
  return (
    <span className="flex h-3 items-end gap-px" aria-hidden>
      {Array.from({ length: of }, (_, i) => (
        <span key={i} className={cn("w-[3px]", i <= level ? "bg-foreground/70" : "bg-muted")} style={{ height: `${4 + (8 * (i + 1)) / of}px` }} />
      ))}
    </span>
  )
}
