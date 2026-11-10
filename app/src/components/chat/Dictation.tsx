import { useEffect, useRef, useState } from "react"
import { LoaderCircle, Mic, Square } from "lucide-react"
import { toast } from "sonner"
import { Tip } from "@/components/Tip"
import { inTerminal } from "@/lib/terminal"
import { cn } from "@/lib/utils"
import { keyCapture, matchesKey, record, useVoice, voice, type Recording } from "@/lib/voice"
import { useApp } from "@/store/app"

const MAX_SECONDS = 180
const BARS = 16

/** The composer's mic, shown only once voice is turned on in Settings: click to start and stop,
 *  or hold the hold-to-talk key. What was said is transcribed on this computer and handed to
 *  `onText`. Esc throws a recording away. */
export function Dictation({ onText }: { onText: (text: string) => void }) {
  const state = useVoice((s) => s.state)
  const settings = useApp((s) => s.settings?.voice)
  const [phase, setPhase] = useState<"idle" | "recording" | "working">("idle")
  const [seconds, setSeconds] = useState(0)
  const [levels, setLevels] = useState<number[]>(() => Array(BARS).fill(0))
  const rec = useRef<Recording | null>(null)
  const live = useRef({ phase, state, settings, onText })
  live.current = { phase, state, settings, onText }

  const start = async () => {
    const { phase, state, settings } = live.current
    if (phase !== "idle") return
    if (state.state !== "ready") {
      toast.info(state.state === "downloading" ? `The voice model is still downloading (${state.percent}%).` : "Turn on voice in Settings → Voice first.", {
        action: { label: "Settings", onClick: () => useApp.getState().openSettings("voice") },
      })
      return
    }
    try {
      rec.current = await record(settings?.microphone ?? "")
      setSeconds(0)
      setLevels(Array(BARS).fill(0))
      setPhase("recording")
    } catch (e) {
      toast.error("oah can't use the microphone", { description: `${e}. Check your system's microphone privacy settings.` })
    }
  }

  const stop = async () => {
    const r = rec.current
    rec.current = null
    if (!r) return
    const pcm = r.stop()
    if (pcm.length < 1600) return setPhase("idle")
    setPhase("working")
    try {
      const text = await voice.transcribe(pcm)
      if (text) live.current.onText(text)
      else toast.info("Didn't catch anything. Try again a little closer to the mic.")
    } catch (e) {
      toast.error("Couldn't transcribe", { description: String(e) })
    } finally {
      setPhase("idle")
    }
  }

  const cancel = () => {
    rec.current?.cancel()
    rec.current = null
    setPhase("idle")
  }

  // Hold to talk, and Esc to cancel.
  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      const { phase, state, settings } = live.current
      if (keyCapture.on || (inTerminal(e.target) && !rec.current)) return
      if (e.key === "Escape" && rec.current) {
        e.preventDefault()
        e.stopPropagation()
        cancel()
      } else if (state.state === "ready" && settings?.holdKey && matchesKey(e, settings.holdKey) && !e.repeat && phase === "idle") {
        e.preventDefault()
        start()
      }
    }
    const up = (e: KeyboardEvent) => {
      const hold = live.current.settings?.holdKey ?? ""
      const key = hold.split("+").pop()?.trim().toLowerCase()
      const name = e.key === " " ? "space" : e.key.toLowerCase()
      if (rec.current && (name === key || (hold.toLowerCase().includes("ctrl") && e.key === "Control"))) stop()
    }
    window.addEventListener("keydown", down, true)
    window.addEventListener("keyup", up, true)
    return () => {
      window.removeEventListener("keydown", down, true)
      window.removeEventListener("keyup", up, true)
      rec.current?.cancel()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Silence stop, the time limit, and the level bars.
  useEffect(() => {
    if (phase !== "recording") return
    let quiet = 0
    const clock = setInterval(() => setSeconds((s) => s + 1), 1000)
    const bars = setInterval(() => {
      const l = rec.current?.level() ?? 0
      setLevels((v) => [...v.slice(1), l])
      const after = live.current.settings?.silenceStopSecs ?? 0
      quiet = l < 0.04 ? quiet + 70 : 0
      if (after > 0 && quiet > after * 1000) stop()
    }, 70)
    return () => {
      clearInterval(clock)
      clearInterval(bars)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase])
  useEffect(() => {
    if (phase === "recording" && seconds >= MAX_SECONDS) stop()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase, seconds])

  // Voice is opt-in (Settings → Voice): no mic until it's on.
  if (state.state !== "ready" && state.state !== "downloading" && phase === "idle") return null

  return (
    <>
      {phase === "recording" ? (
        <div className="rounded-full flex h-7 items-center gap-2 border border-red/40 bg-del/40 px-2 text-[11.5px]">
          <span className="rounded-full size-1.5 animate-pulse bg-red" />
          <span className="font-mono tabular-nums">
            {Math.floor(seconds / 60)}:{String(seconds % 60).padStart(2, "0")}
          </span>
          <span aria-hidden className="flex h-4 items-end gap-px">
            {levels.map((l, i) => (
              <span key={i} className="w-[3px] bg-red transition-[height] duration-75" style={{ height: `${Math.max(2, Math.round(2 + l * 14))}px` }} />
            ))}
          </span>
          <span className="text-muted-foreground">Esc to cancel</span>
        </div>
      ) : phase === "working" ? (
        <span className="flex h-7 items-center gap-1.5 px-2 text-[11.5px] text-muted-foreground">
          <LoaderCircle className="size-3.5 animate-spin" /> Transcribing…
        </span>
      ) : null}
      <Tip content={phase === "recording" ? "Stop and transcribe" : `Dictate${settings?.holdKey ? ` (hold ${settings.holdKey})` : ""}`}>
        <button
          className={cn("rounded-full grid size-7 place-items-center hover:bg-accent", phase === "recording" ? "bg-red text-white hover:bg-red/90" : "text-muted-foreground hover:text-foreground")}
          aria-label={phase === "recording" ? "Stop dictation" : "Dictate"}
          disabled={phase === "working"}
          onClick={() => (phase === "recording" ? stop() : start())}
        >
          {phase === "recording" ? <Square className="size-3 fill-current" /> : <Mic className="size-3.5" />}
        </button>
      </Tip>
    </>
  )
}
