import { useEffect, useRef, useState } from "react"
import { ArrowUp, ChevronDown, Paperclip, Square, X } from "lucide-react"
import { Tip } from "@/components/Tip"
import { AgentGlyph } from "@/components/icons"
import { DropdownMenu, DropdownMenuContent, DropdownMenuLabel, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { AGENTS, AGENT_LABEL, PERMISSION_LABEL, type AgentKind, type PermissionMode } from "@/lib/types"
import { cn } from "@/lib/utils"
import { Dictation } from "./Dictation"

export type ComposerProps = {
  agent: AgentKind
  onAgent?: (a: AgentKind) => void
  model: string
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

function readImage(file: File): Promise<Attachment | null> {
  if (!file.type.startsWith("image/") || file.size > MAX_IMAGE) return Promise.resolve(null)
  return new Promise((resolve) => {
    const r = new FileReader()
    r.onload = () => resolve({ mime: file.type, data: String(r.result).split(",")[1] ?? "", name: file.name || "pasted image" })
    r.onerror = () => resolve(null)
    r.readAsDataURL(file)
  })
}

/** The prompt box: text, then a toolbar with the agent, model and permission chips. */
export function Composer(p: ComposerProps) {
  const [text, setText] = useState("")
  const [images, setImages] = useState<Attachment[]>([])
  const [dragging, setDragging] = useState(false)
  const ref = useRef<HTMLTextAreaElement>(null)
  const picker = useRef<HTMLInputElement>(null)
  const addFiles = async (files: Iterable<File>) => {
    const read = (await Promise.all([...files].map(readImage))).filter((a): a is Attachment => !!a)
    if (read.length) setImages((cur) => [...cur, ...read].slice(0, 8))
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
      className={cn("border border-input bg-card focus-within:border-ring/70", dragging && "border-primary", p.className)}
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
            <div key={i} className="group/img relative size-14 border bg-background">
              <img src={`data:${img.mime};base64,${img.data}`} alt={img.name} className="size-full object-cover" />
              <button
                className="absolute -top-1.5 -right-1.5 hidden size-4 place-items-center bg-foreground text-background group-hover/img:grid"
                aria-label="Remove image"
                onClick={() => setImages((cur) => cur.filter((_, j) => j !== i))}
              >
                <X className="size-3" />
              </button>
            </div>
          ))}
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
        onChange={(e) => setText(e.target.value)}
        onPaste={(e) => {
          const files = [...e.clipboardData.files].filter((f) => f.type.startsWith("image/"))
          if (files.length) {
            e.preventDefault()
            addFiles(files)
          }
        }}
        onKeyDown={(e) => {
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
            <button className="flex h-7 items-center gap-1.5 px-2 text-[12px] font-medium hover:bg-accent disabled:hover:bg-transparent">
              <AgentGlyph agent={p.agent} className="size-3.5" />
              {AGENT_LABEL[p.agent]}
              {p.lockAgent ? null : <ChevronDown className="size-3 text-muted-foreground" />}
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="w-52">
            <DropdownMenuLabel>Agent</DropdownMenuLabel>
            <DropdownMenuRadioGroup value={p.agent} onValueChange={(v) => p.onAgent?.(v as AgentKind)}>
              {AGENTS.map((a) => (
                <DropdownMenuRadioItem key={a} value={a}>
                  <AgentGlyph agent={a} className="size-3.5" /> {AGENT_LABEL[a]}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        <span className="flex h-7 items-center px-2 font-mono text-[11.5px] text-muted-foreground">{p.model || "default model"}</span>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button className={cn("flex h-7 items-center gap-1 px-2 text-[12px] hover:bg-accent", p.permission === "fullAccess" ? "text-red" : p.permission === "plan" ? "text-cyan" : "text-muted-foreground")}>
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
          <Tip content="Attach image">
            <button className="grid size-7 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Attach image" onClick={() => picker.current?.click()}>
              <Paperclip className="size-3.5" />
            </button>
          </Tip>
          <Dictation onText={(t) => window.dispatchEvent(new CustomEvent("oah:insert", { detail: t + " " }))} />
          {p.running ? (
            <Tip content="Stop (Esc)">
              <button className="ml-1 grid size-7 place-items-center bg-foreground text-background hover:opacity-90" aria-label="Stop" onClick={p.onStop}>
                <Square className="size-3 fill-current" />
              </button>
            </Tip>
          ) : (
            <Tip content="Send (Enter)">
              <button
                className="ml-1 grid size-7 place-items-center bg-primary text-primary-foreground hover:bg-primary/90 disabled:bg-muted disabled:text-faint"
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
