import { useEffect, useRef, useState } from "react"
import { ArrowUp, Check, ChevronDown, ImagePlus, ListPlus, SlidersHorizontal, Square, X, Zap } from "lucide-react"
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import { effortLabel, type ModelOption } from "@/lib/chat"
import { PERMISSION_LABEL, type PermissionMode } from "@/lib/types"
import { cn } from "@/lib/utils"
import { readImage, type Image } from "./rpc"

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
  onCompact?: () => void
  onClear?: () => void
}

/** The phone's message box: text, photos, send or stop, and a sheet for the model, permission
 *  and `/compact` / `/clear`. */
export function MobileComposer(p: Props) {
  const [text, setText] = useState("")
  const [images, setImages] = useState<Image[]>([])
  const dictation = usePhoneVoice((words) => setText((draft) => draft ? `${draft}${/\s$/.test(draft) ? "" : " "}${words}` : words))
  const [sheet, setSheet] = useState(false)
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

  const has = text.trim().length > 0 || images.length > 0
  const send = (how: "queue" | "now") => {
    if (!has || p.disabled || dictation.busy) return
    p.onSend(text.trim(), images, how)
    setText("")
    setImages([])
  }
  const current = p.models.find((m) => m.id === p.model)

  return (
    <div className="border-t bg-background px-2.5 pt-2 pb-[max(0.5rem,env(safe-area-inset-bottom))]">
      {dictation.status}
      {images.length ? (
        <div className="mb-2 flex gap-2.5 overflow-x-auto px-1 pt-1.5">
          {images.map((im, i) => (
            <div key={i} className="relative size-16 shrink-0">
              <img src={`data:${im.mime};base64,${im.data}`} alt={`Attachment ${i + 1}`} className="rounded-md size-full border object-cover" />
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
          className="block max-h-[150px] min-h-11 w-full resize-none bg-transparent px-3.5 pt-3 pb-1 text-[15px] leading-snug outline-none placeholder:text-faint"
          aria-label="Message"
        />
        <input
          ref={file}
          type="file"
          accept="image/*"
          multiple
          className="hidden"
          onChange={async (e) => {
            const files = [...(e.target.files ?? [])]
            e.target.value = ""
            const read = await Promise.all(files.map(readImage))
            setImages((a) => [...a, ...read])
          }}
        />
        {/* Photo and options on the left; stop, send now and send/queue on the right. */}
        <div className="flex items-center gap-1 p-1">
          <button className="rounded-md grid size-10 shrink-0 place-items-center text-muted-foreground active:bg-accent" aria-label="Add a photo" onClick={() => file.current?.click()}>
            <ImagePlus className="size-5" />
          </button>
          <button
            className="rounded-md flex h-10 min-w-10 shrink items-center justify-center gap-1.5 px-2 text-[13px] text-muted-foreground active:bg-accent"
            aria-label="Chat options: model, effort, permissions"
            onClick={() => (setSheet(true), p.onModelsOpen?.())}
          >
            <SlidersHorizontal className="size-4 shrink-0" />
            {/* While a turn runs and there's a draft, the send buttons need the room. */}
            {p.running && has ? null : (
              <>
                <span className="truncate">{current?.name ?? (p.model || "Options")}</span>
                <ChevronDown className="size-3.5 shrink-0" />
              </>
            )}
          </button>
          {dictation.button}
          <span className="flex-1" />
          {p.running && p.onStop ? (
            <button
              className={cn("rounded-md flex h-10 shrink-0 items-center justify-center gap-1.5 border border-input text-[13px] active:bg-accent", has ? "w-10" : "px-3")}
              aria-label="Stop the turn"
              onClick={p.onStop}
            >
              <Square className="size-3.5 fill-current" />
              {has ? null : "Stop"}
            </button>
          ) : null}
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

      <Sheet open={sheet} onOpenChange={setSheet}>
        <SheetContent side="bottom" onOpenAutoFocus={(e) => e.preventDefault()} className="max-h-[85vh] gap-0 overflow-y-auto rounded-t-lg pb-[max(1rem,env(safe-area-inset-bottom))]">
          <SheetHeader>
            <SheetTitle>Chat options</SheetTitle>
          </SheetHeader>
          {dictation.settings}
          {p.onModel ? (
            <section className="px-4 pb-4">
              <h3 className="mb-1.5 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">Model</h3>
              {p.models.length === 0 ? <div className="py-2 text-[13px] text-muted-foreground">Loading models…</div> : null}
              <div className="divide-y rounded-lg border">
                {p.models.map((m) => (
                  <button key={m.id} className="flex w-full items-center gap-3 px-3 py-2.5 text-left active:bg-accent" onClick={() => p.onModel?.(m.id, m.efforts.includes(p.effort) ? p.effort : "")}>
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-[14px]">{m.name}</div>
                      {m.description ? <div className="truncate text-[11.5px] text-muted-foreground">{m.description}</div> : null}
                    </div>
                    {m.id === p.model ? <Check className="size-4 shrink-0 text-primary" /> : null}
                  </button>
                ))}
              </div>
              {current?.efforts.length ? (
                <>
                  <h3 className="mt-4 mb-1.5 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">Effort</h3>
                  <div className="flex flex-wrap gap-1.5">
                    {["", ...current.efforts].map((e) => (
                      <button
                        key={e || "default"}
                        className={cn("rounded-md h-10 border px-3 text-[13px]", (p.effort || "") === e ? "border-primary bg-primary text-primary-foreground" : "active:bg-accent")}
                        onClick={() => p.onModel?.(p.model, e)}
                      >
                        {e ? effortLabel(e) : "Default"}
                      </button>
                    ))}
                  </div>
                </>
              ) : null}
            </section>
          ) : null}
          {p.onPermission ? (
            <section className="px-4 pb-4">
              <h3 className="mb-1.5 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">Permissions</h3>
              <div className="grid grid-cols-2 gap-1.5">
                {MODES.map((m) => (
                  <button
                    key={m}
                    className={cn("rounded-md h-10 border px-3 text-[13px]", p.permission === m ? "border-primary bg-primary text-primary-foreground" : "active:bg-accent")}
                    onClick={() => p.onPermission?.(m)}
                  >
                    {PERMISSION_LABEL[m]}
                  </button>
                ))}
              </div>
            </section>
          ) : null}
          {p.onCompact || p.onClear ? (
            <section className="flex gap-1.5 px-4 pb-2">
              {p.onCompact ? (
                <button className="rounded-md h-10 flex-1 border text-[13px] active:bg-accent" onClick={() => (p.onCompact?.(), setSheet(false))}>
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
                    setSheet(false)
                  }}
                >
                  {clearArmed ? "Tap again to clear" : "Clear context"}
                </button>
              ) : null}
            </section>
          ) : null}
        </SheetContent>
      </Sheet>
    </div>
  )
}
