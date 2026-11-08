import { useEffect, useRef, useState } from "react"
import { isTauri } from "@tauri-apps/api/core"
import { LoaderCircle, Mic, Square, X } from "lucide-react"
import { toast } from "sonner"
import { initVoice, useVoice, voice } from "@/lib/voice"

import { MAX_SECONDS, PhoneDictation, type Phase } from "./dictation"

/** A recording belongs to this composer. Leaving it or backgrounding the phone cancels it. */
export function usePhoneVoice(onText: (text: string) => void) {
  const state = useVoice((s) => s.state)
  const [phase, setPhase] = useState<Phase>("idle")
  const [seconds, setSeconds] = useState(0)
  const [changing, setChanging] = useState(false)
  const callback = useRef(onText)
  callback.current = onText
  const [session] = useState(() => new PhoneDictation(
    (p) => { if (p === "recording") setSeconds(0); setPhase(p) },
    (text) => {
      if (text) callback.current(text)
      else toast.info("Didn't catch any speech. Try again a little closer to the mic.")
    },
    (e) => toast.error("Voice input failed", { description: String(e) }),
  ))
  const cancel = () => session.cancel()
  const start = () => state.state === "ready" ? session.start() : undefined
  const stop = () => session.stop()

  useEffect(() => {
    if (isTauri()) void initVoice().catch((e) => toast.error("Couldn't load voice settings", { description: String(e) }))
    const onHide = () => {
      if (document.hidden && (session.phase === "recording" || session.phase === "starting")) session.cancel()
    }
    document.addEventListener("visibilitychange", onHide)
    return () => {
      document.removeEventListener("visibilitychange", onHide)
      session.cancel(false)
    }
  }, [session])

  useEffect(() => {
    if (phase !== "recording") return
    const start = Date.now()
    const timer = setInterval(() => setSeconds(Math.min(MAX_SECONDS, Math.floor((Date.now() - start) / 1000))), 250)
    const limit = setTimeout(() => void stop(), MAX_SECONDS * 1000)
    return () => { clearInterval(timer); clearTimeout(limit) }
  }, [phase])

  const change = async (enable: boolean) => {
    if (changing || session.phase !== "idle") return
    setChanging(true)
    try {
      await (enable ? voice.enable() : voice.disable())
    } catch (e) {
      toast.error("Couldn't change voice settings", { description: String(e) })
    } finally { setChanging(false) }
  }

  const busy = phase !== "idle"
  return {
    busy,
    button: state.state === "ready" || busy ? (
      <button
        className={`rounded-md grid size-10 shrink-0 place-items-center active:bg-accent ${phase === "recording" ? "bg-red text-white" : "text-muted-foreground"}`}
        aria-label={phase === "recording" ? "Stop dictation" : "Dictate on this phone"}
        disabled={phase === "starting" || phase === "working" || changing}
        onClick={() => void (phase === "recording" ? stop() : start())}
      >
        {phase === "working" || phase === "starting" ? <LoaderCircle className="size-5 animate-spin" /> : phase === "recording" ? <Square className="size-4 fill-current" /> : <Mic className="size-5" />}
      </button>
    ) : null,
    status: busy ? (
      <div className="mb-2 flex min-h-10 items-center gap-2 px-2 text-[13px]" role="status">
        <span className="flex-1">{phase === "recording" ? `Recording ${seconds}s / ${MAX_SECONDS}s · tap stop to transcribe` : phase === "working" ? "Transcribing on this phone…" : "Opening microphone…"}</span>
        {phase !== "working" ? <button className="grid size-10 place-items-center rounded-md active:bg-accent" aria-label="Cancel dictation" onClick={cancel}><X className="size-4" /></button> : null}
      </div>
    ) : null,
    settings: (
      <section className="px-4 pb-4">
        <h3 className="mb-1.5 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">Voice on this phone</h3>
        <p className="mb-2 text-[13px] text-muted-foreground">English dictation, processed on your phone. Download the 731 MB model once, then dictate offline. Record up to 30 seconds at a time.</p>
        {!isTauri() ? <p className="text-[13px] text-muted-foreground">Available in the Android app.</p> : state.state === "unsupported" ? <p className="text-[13px] text-muted-foreground">{state.error}</p> : (
          <>
            {state.state === "error" ? <p className="mb-2 text-[13px] text-red" role="alert">{state.error}</p> : null}
            {state.state === "downloading" ? <p className="mb-2 text-[13px]" role="status">Downloading… {state.percent}%</p> : null}
            <button
              className="h-10 rounded-md border px-3 text-[13px] active:bg-accent disabled:opacity-50"
              disabled={busy || changing}
              onClick={() => void change(state.state !== "ready" && state.state !== "downloading")}
            >
              {changing ? "Please wait…" : state.state === "ready" ? "Turn off and remove model" : state.state === "downloading" ? "Cancel download" : state.state === "error" ? "Retry download" : "Download voice model"}
            </button>
          </>
        )}
      </section>
    ),
  }
}
