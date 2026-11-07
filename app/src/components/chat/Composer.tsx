import { useEffect, useRef, useState } from "react"
import { ArrowUp, ChevronDown, Mic, Paperclip, Square } from "lucide-react"
import { Tip } from "@/components/Tip"
import { AgentGlyph } from "@/components/icons"
import { DropdownMenu, DropdownMenuContent, DropdownMenuLabel, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { AGENTS, AGENT_LABEL, PERMISSION_LABEL, type AgentKind, type PermissionMode } from "@/lib/types"
import { cn } from "@/lib/utils"

export type ComposerProps = {
  agent: AgentKind
  onAgent?: (a: AgentKind) => void
  model: string
  permission: PermissionMode
  onPermission?: (p: PermissionMode) => void
  running?: boolean
  disabled?: boolean
  placeholder?: string
  onSend?: (text: string) => void
  onStop?: () => void
  className?: string
  autoFocus?: boolean
  /** The agent can't change once a chat started. */
  lockAgent?: boolean
}

const PERMISSIONS: PermissionMode[] = ["ask", "acceptEdits", "plan", "fullAccess"]

/** The prompt box: text, then a toolbar with the agent, model and permission chips. */
export function Composer(p: ComposerProps) {
  const [text, setText] = useState("")
  const ref = useRef<HTMLTextAreaElement>(null)
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
    if (!t || p.disabled) return
    p.onSend?.(t)
    setText("")
  }

  return (
    <div className={cn("border border-input bg-card focus-within:border-ring/70", p.className)}>
      <textarea
        ref={ref}
        autoFocus={p.autoFocus}
        value={text}
        rows={3}
        placeholder={p.placeholder ?? "Ask anything, @ to add files"}
        className="block max-h-72 min-h-[76px] w-full resize-none bg-transparent px-3.5 pt-3 text-[length:var(--oah-chat-size,14px)] leading-relaxed outline-none placeholder:text-faint"
        onChange={(e) => setText(e.target.value)}
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
            <button className="grid size-7 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Attach image">
              <Paperclip className="size-3.5" />
            </button>
          </Tip>
          <Tip content="Dictate (hold Ctrl Space)">
            <button className="grid size-7 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Dictate">
              <Mic className="size-3.5" />
            </button>
          </Tip>
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
                disabled={!text.trim() || p.disabled}
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
