import { useEffect, useRef, useState, type ReactNode } from "react"
import { ArrowUp, Check, ChevronDown, Cpu, ImagePlus, ListPlus, Plus, Square, X, Zap } from "lucide-react"
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { PERMISSION_ICON } from "@/components/permission"
import { effortLabel, type ModelOption, type Transcript } from "@/lib/chat"
import { PERMISSION_LABEL, type PermissionMode } from "@/lib/types"
import { cn } from "@/lib/utils"
import { type Image } from "./rpc"
import { attachmentMime, MAX_IMAGE, readAttachment } from "@/lib/attachments"
import { openImages } from "@/components/ImageView"
import { toast } from "sonner"

import { usePhoneVoice } from "./Voice"

const MODES: PermissionMode[] = ["ask", "acceptEdits", "plan", "fullAccess"]

type Props = {
  running: boolean
  disabled?: boolean
  placeholder?: string
  /** `now`: into the running turn; otherwise it waits for the turn to end. */
  onSend: (text: string, images: Image[], how: "queue" | "now") => void
  onStop?: () => void
  models: ModelOption[]
  model: string
  effort: string
  onModel?: (model: string, effort: string) => void
  onModelsOpen?: () => void
  permission: PermissionMode | null
  onPermission?: (m: PermissionMode) => void
  /** How full the agent's context window is, when it has said. */
  context?: Transcript["context"]
  onCompact?: () => void
  onClear?: () => void
}

/** A dropdown sized for a thumb. */
const TRIGGER = "w-full text-[14px] data-[size=default]:h-11"
const ITEM = "py-2.5 text-[14px]"
/** Radix Select can't hold "", so the agent's default effort goes by this. */
const DEFAULT_EFFORT = "default"

function Heading({ children, first }: { children: ReactNode; first?: boolean }) {
  return <h3 className={cn("mb-1.5 text-[11px] font-medium tracking-wide text-muted-foreground uppercase", !first && "mt-5")}>{children}</h3>
}

/** `61.4k` */
function tokens(n: number) {
  return n >= 1000 ? `${(n / 1000).toFixed(n >= 100_000 ? 0 : 1)}k` : String(n)
}

/** The phone's message box: text, send or stop, the model, and a "+" sheet for photos, effort,
 *  permissions and the context (`/compact`, `/clear`). */
export function MobileComposer(p: Props) {
  const [text, setText] = useState("")
  const [images, setImages] = useState<Image[]>([])
  const dictation = usePhoneVoice((words) => setText((draft) => draft ? `${draft}${/\s$/.test(draft) ? "" : " "}${words}` : words))
  const [sheet, setSheet] = useState<"more" | "model" | null>(null)
  const [clearArmed, setClearArmed] = useState(false)
  useEffect(() => {
    if (!clearArmed) return
    const t = setTimeout(() => setClearArmed(false), 3000)
    return () => clearTimeout(t)
  }, [clearArmed])
  const box = useRef<HTMLTextAreaElement>(null)
  const file = useRef<HTMLInputElement>(null)

  // Grows with the text, up to about six lines.
  useEffect(() => {
    const el = box.current
    if (!el) return
    el.style.height = "auto"
    el.style.height = `${Math.min(el.scrollHeight, 150)}px`
  }, [text])

  const addImages = async (files: File[]) => {
    const valid = files.filter((f) => attachmentMime(f) && f.size <= MAX_IMAGE)
    if (files.some((f) => !attachmentMime(f))) toast.error("Only PNG, JPEG, GIF and WebP images can be attached")
    if (files.some((f) => f.size > MAX_IMAGE)) toast.error("Images must be 5 MB or smaller")
    const results = await Promise.allSettled(valid.slice(0, 8).map(readAttachment))
    if (results.some((r) => r.status === "rejected")) toast.error("Could not read an image")
    const read = results.flatMap((r) => r.status === "fulfilled" ? [r.value] : [])
    setImages((a) => [...a, ...read].slice(0, 8))
  }
  const has = text.trim().length > 0 || images.length > 0
  const send = (how: "queue" | "now") => {
    if (!has || p.disabled || dictation.busy) return
    p.onSend(text.trim() || "(see the attached image)", images, how)
    setText("")
    setImages([])
  }
  const current = p.models.find((m) => m.id === p.model)

  return (
    <div onDragOver={(e) => e.preventDefault()} onDrop={(e) => { e.preventDefault(); void addImages(Array.from(e.dataTransfer.files)) }} className="border-t bg-background px-2.5 pt-2 pb-[max(0.5rem,env(safe-area-inset-bottom))]">
      {dictation.status}
      {images.length ? (
        <div className="mb-2 flex gap-2.5 overflow-x-auto px-1 pt-1.5">
          {images.map((im, i) => (
            <div key={i} className="relative size-16 shrink-0">
              <button className="size-full" aria-label={`View attachment ${i + 1}`} onClick={() => openImages(images.map((x, j) => ({ src: `data:${x.mime};base64,${x.data}`, name: `Attachment ${j + 1}` })), i)}>
                <img src={`data:${im.mime};base64,${im.data}`} alt={`Attachment ${i + 1}`} className="rounded-md size-full border object-cover" />
              </button>
              <button className="rounded-full absolute -top-1 -right-1 grid size-7 place-items-center border bg-background" aria-label="Remove image" onClick={() => setImages((a) => a.filter((_, j) => j !== i))}>
                <X className="size-3" />
              </button>
            </div>
          ))}
        </div>
      ) : null}
      <div className="rounded-lg border bg-card">
        <textarea
          ref={box}
          rows={1}
          value={text}
          placeholder={p.placeholder ?? "Message"}
          onChange={(e) => setText(e.target.value)}
          onPaste={(e) => {
            const files = Array.from(e.clipboardData.files)
            if (files.length) { e.preventDefault(); void addImages(files) }
          }}
          className="block max-h-[150px] min-h-11 w-full resize-none bg-transparent px-3.5 pt-3 pb-1 text-[15px] leading-snug outline-none placeholder:text-faint"
          aria-label="Message"
        />
        <input
          ref={file}
          type="file"
          accept="image/png,image/jpeg,image/gif,image/webp,.png,.jpg,.jpeg,.gif,.webp"
          multiple
          className="hidden"
          onChange={async (e) => {
            const files = [...(e.target.files ?? [])]
            e.target.value = ""
            await addImages(files)
          }}
        />
        {/* "+" and the model on the left; stop, the mic, then send now and send/queue on the right. */}
        <div className="flex items-center gap-1 p-1">
          <button className="rounded-md grid size-10 shrink-0 place-items-center text-muted-foreground active:bg-accent" aria-label="Attach, effort, permissions and context" onClick={() => setSheet("more")}>
            <Plus className="size-5" />
          </button>
          <button
            className="rounded-md flex h-10 min-w-10 shrink items-center justify-center gap-1.5 px-2 text-[13px] text-muted-foreground active:bg-accent"
            aria-label="Model"
            onClick={() => (setSheet("model"), p.onModelsOpen?.())}
          >
            <Cpu className="size-4 shrink-0" />
            {/* While a turn runs and there's a draft, the send buttons need the room. */}
            {p.running && has ? null : (
              <>
                <span className="truncate">{current?.name ?? (p.model || "Model")}</span>
                <ChevronDown className="size-3.5 shrink-0" />
              </>
            )}
          </button>
          <span className="flex-1" />
          {p.running && p.onStop ? (
            <button className="rounded-md grid size-10 shrink-0 place-items-center border border-input active:bg-accent" aria-label="Stop the turn" onClick={p.onStop}>
              <Square className="size-3.5 fill-current" />
            </button>
          ) : null}
          {dictation.button}
          {p.running && has ? (
            <>
              <button className="rounded-md flex h-10 shrink-0 items-center gap-1 border border-input px-2.5 text-[13px] active:bg-accent" disabled={p.disabled || dictation.busy} aria-label="Send now, into the running turn" onClick={() => send("now")}>
                <Zap className="size-4" /> Now
              </button>
              <button className="rounded-md flex h-10 shrink-0 items-center gap-1.5 bg-primary px-3 text-[13px] font-medium text-primary-foreground" disabled={p.disabled || dictation.busy} aria-label="Queue: send when the turn ends" onClick={() => send("queue")}>
                <ListPlus className="size-4" /> Queue
              </button>
            </>
          ) : p.running ? null : (
            <button
              className="rounded-md grid size-10 shrink-0 place-items-center bg-primary text-primary-foreground disabled:bg-muted disabled:text-faint"
              aria-label="Send"
              disabled={!has || p.disabled || dictation.busy}
              onClick={() => send("queue")}
            >
              <ArrowUp className="size-5" />
            </button>
          )}
        </div>
      </div>

      <Sheet open={sheet === "more"} onOpenChange={(o) => setSheet(o ? "more" : null)}>
        <SheetContent side="bottom" onOpenAutoFocus={(e) => e.preventDefault()} className="max-h-[85vh] gap-0 overflow-y-auto rounded-t-lg pb-[max(1rem,env(safe-area-inset-bottom))]">
          <SheetHeader>
            <SheetTitle>Chat options</SheetTitle>
          </SheetHeader>
          <div className="px-4 pb-2">
            <Heading first>Attach</Heading>
            <button
              className="rounded-md flex h-11 w-full items-center gap-2.5 border px-3 text-[14px] active:bg-accent"
              onClick={() => {
                // Opens the picker while the tap still counts as one, then closes the sheet.
                file.current?.click()
                setSheet(null)
              }}
            >
              <ImagePlus className="size-4.5 text-muted-foreground" /> Attach images
            </button>
            {p.onModel && current?.efforts.length ? (
              <>
                <Heading>Effort</Heading>
                <Select value={p.effort || DEFAULT_EFFORT} onValueChange={(e) => p.onModel?.(p.model, e === DEFAULT_EFFORT ? "" : e)}>
                  <SelectTrigger className={TRIGGER} aria-label="Effort">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {[DEFAULT_EFFORT, ...current.efforts].map((e) => (
                      <SelectItem key={e} value={e} className={ITEM}>
                        {e === DEFAULT_EFFORT ? "Default" : effortLabel(e)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </>
            ) : null}
            {p.onPermission ? (
              <>
                <Heading>Permissions</Heading>
                <Select value={p.permission ?? undefined} onValueChange={(m) => p.onPermission?.(m as PermissionMode)}>
                  <SelectTrigger className={TRIGGER} aria-label="Permissions">
                    <SelectValue placeholder="Choose…" />
                  </SelectTrigger>
                  <SelectContent>
                    {MODES.map((m) => {
                      const Icon = PERMISSION_ICON[m]
                      return (
                        <SelectItem key={m} value={m} className={ITEM}>
                          <Icon /> {PERMISSION_LABEL[m]}
                        </SelectItem>
                      )
                    })}
                  </SelectContent>
                </Select>
              </>
            ) : null}
            {p.onCompact || p.onClear ? (
              <>
                <Heading>Context</Heading>
                {p.context && p.context.window > 0 ? (
                  <div className="mb-2.5">
                    <div className="mb-1 flex text-[12.5px] text-muted-foreground">
                      <span className="flex-1">{Math.min(100, Math.round((p.context.used / p.context.window) * 100))}% used</span>
                      <span className="font-mono">{tokens(p.context.used)} / {tokens(p.context.window)}</span>
                    </div>
                    <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                      <div className="h-full rounded-full bg-primary" style={{ width: `${Math.min(100, Math.round((p.context.used / p.context.window) * 100))}%` }} />
                    </div>
                  </div>
                ) : null}
                <div className="flex gap-1.5">
                  {p.onCompact ? (
                    <button className="rounded-md h-10 flex-1 border text-[13px] active:bg-accent" onClick={() => (p.onCompact?.(), setSheet(null))}>
                      Compact context
                    </button>
                  ) : null}
                  {p.onClear ? (
                    <button
                      className={cn("rounded-md h-10 flex-1 border text-[13px] text-red active:bg-red/10", clearArmed && "border-red bg-red/10")}
                      onClick={() => {
                        // Clearing can't be undone: the first tap asks, the second clears.
                        if (!clearArmed) return setClearArmed(true)
                        p.onClear?.()
                        setSheet(null)
                      }}
                    >
                      {clearArmed ? "Tap again to clear" : "Clear context"}
                    </button>
                  ) : null}
                </div>
              </>
            ) : null}
          </div>
        </SheetContent>
      </Sheet>

      <Sheet open={sheet === "model"} onOpenChange={(o) => setSheet(o ? "model" : null)}>
        <SheetContent side="bottom" onOpenAutoFocus={(e) => e.preventDefault()} className="max-h-[85vh] gap-0 overflow-y-auto rounded-t-lg pb-[max(1rem,env(safe-area-inset-bottom))]">
          <SheetHeader>
            <SheetTitle>Model</SheetTitle>
          </SheetHeader>
          <div className="px-4 pb-2">
            {p.models.length === 0 ? <div className="py-2 text-[13px] text-muted-foreground">Loading models…</div> : null}
            <div className="divide-y rounded-lg border">
              {p.models.map((m) => (
                <button
                  key={m.id}
                  className="flex w-full items-center gap-3 px-3 py-2.5 text-left active:bg-accent"
                  onClick={() => (p.onModel?.(m.id, m.efforts.includes(p.effort) ? p.effort : ""), setSheet(null))}
                >
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-[14px]">{m.name}</div>
                    {m.description ? <div className="truncate text-[11.5px] text-muted-foreground">{m.description}</div> : null}
                  </div>
                  {m.id === p.model ? <Check className="size-4 shrink-0 text-primary" /> : null}
                </button>
              ))}
            </div>
          </div>
        </SheetContent>
      </Sheet>
    </div>
  )
}
