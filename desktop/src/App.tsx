import { ArrowDown, FolderOpen, Loader2, RotateCw, TriangleAlert } from "lucide-react"
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from "@/components/ui/card"
import { Kbd } from "@/components/ui/kbd"
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { Composer } from "./components/Composer"
import { mod, modKey, useTick } from "./components/hooks"
import { Mark, SPINNER } from "./components/icons"
import { Inspector } from "./components/Inspector"
import { Picker } from "./components/Picker"
import { Settings } from "./components/Settings"
import { AppSidebar } from "./components/Sidebar"
import { TitleBar } from "./components/TitleBar"
import { TodoList } from "./components/Todos"
import { Transcript } from "./components/Transcript"
import { useClient } from "./lib/client"
import { fmtMs } from "./lib/format"
import { cn } from "./lib/utils"

function Welcome() {
  const c = useClient()
  const s = c.state!
  const signedIn = s.auth.available.length > 0
  const tips: [string, string, string][] = s.updateAvailable ? [["/update", `oah ${s.updateAvailable} is available (you have ${c.hello?.version})`, "/update"]] : []
  tips.push(
    ...((signedIn
      ? [
          ["/model", `Pick a model · ${modKey}T`, "/model"],
          ["Tab", "Plan ↔ build", ""],
          ["Shift+Tab", "auto → always ask → always allow", ""],
          ["/goal", "Work until it's done", "/goal"],
          ["/session", "Resume a saved session", "/session"],
          ["/help", "Keys & commands", "/help"],
        ]
      : [
          ["/auth", "Continue with ChatGPT, or add a key (Anthropic, OpenAI, Gemini, Grok, DeepSeek, OpenRouter, Kimi, Z.ai, MiniMax…)", "/auth"],
          ["/help", "Keys & commands", "/help"],
        ]) as [string, string, string][]),
  )
  return (
    <div className="mx-auto mt-[8vh] flex max-w-xl flex-col items-center gap-4 text-center">
      <Mark size={64} />
      <h1 className="font-mono text-4xl font-extrabold tracking-tight text-primary">oah</h1>
      <p className="-mt-2 text-muted-foreground">open agent harness · v{c.hello?.version}</p>
      <Button variant="outline" size="sm" className="rounded-full font-mono text-xs text-muted-foreground" onClick={() => void c.openFolder()} title="Open another folder">
        <FolderOpen />
        {c.started?.cwdShort}
      </Button>
      <div className="mt-3 grid w-full gap-2 sm:grid-cols-2">
        {tips.map(([k, d, cmd]) => (
          <button
            key={k}
            disabled={!cmd}
            onClick={() => cmd && c.submit(cmd)}
            className={cn("flex flex-col items-start gap-0.5 rounded-xl border bg-card px-4 py-3 text-left transition-colors enabled:hover:border-primary/40 enabled:hover:bg-accent/40", k === "/update" && "border-primary/40")}
          >
            <span className="font-mono text-[13px] font-semibold text-primary">{k}</span>
            <span className="text-xs text-muted-foreground">{d}</span>
          </button>
        ))}
      </div>
    </div>
  )
}

function Activity() {
  const c = useClient()
  const s = c.state!
  const b = s.busy
  const tick = useTick(90, !!b)
  if (!b && !s.queue.length && !s.steering) return null
  return (
    <div className="flex min-h-7 items-center gap-2.5 text-[13px] text-muted-foreground">
      {b ? (
        <>
          <span className={cn("w-3 font-mono", b.kind === "compact" ? "text-cyan" : b.kind === "login" ? "text-warn" : "text-primary")}>{SPINNER[tick % SPINNER.length]}</span>
          <span className="shimmer">{s.activity || "Working"}</span>
          <span className="font-mono text-xs">{fmtMs(Date.now() - b.started)}</span>
          <Button variant="ghost" size="sm" className="h-6 px-2 text-xs" onClick={() => c.send({ op: "interrupt" })}>
            Stop <Kbd>Esc</Kbd>
          </Button>
        </>
      ) : null}
      {s.queue.length || s.steering ? (
        <span className="ml-auto flex items-center gap-2 text-xs">
          {[s.steering && `${s.steering} sending`, s.queue.length && `${s.queue.length} queued`].filter(Boolean).join(" · ")}
          {s.queue.length ? (
            <Button variant="link" size="sm" className="h-auto p-0 text-xs" onClick={() => c.send({ op: "sendQueued" })}>Send now</Button>
          ) : null}
        </span>
      ) : null}
    </div>
  )
}

function TasksInline() {
  const s = useClient().state!
  const todos = s.session.todos
  if (!todos.length || todos.every((x) => x.status === "completed")) return null
  const done = todos.filter((x) => x.status === "completed").length
  return (
    <div className="flex flex-col gap-2 rounded-lg border bg-card px-4 py-3">
      <span className="text-xs text-muted-foreground">Tasks {done}/{todos.length}</span>
      <TodoList />
    </div>
  )
}

function Chat() {
  const c = useClient()
  const s = c.state!
  const scroller = useRef<HTMLDivElement>(null)
  const stick = useRef(true)
  const [away, setAway] = useState(false)
  const inspector = c.prefs.inspector !== false
  const jump = () => {
    const el = scroller.current
    if (!el) return
    stick.current = true
    setAway(false)
    el.scrollTo({ top: el.scrollHeight, behavior: "smooth" })
  }
  // Follow new output while scrolled to the bottom (like the TUI's sticky scroll).
  useLayoutEffect(() => {
    const el = scroller.current
    if (el && stick.current) el.scrollTop = el.scrollHeight
  })
  useEffect(() => {
    stick.current = true
    setAway(false)
  }, [s.session.id])
  // Ctrl+End jumps back to the newest output, like the TUI.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "End" && (e.ctrlKey || e.metaKey)) {
        e.preventDefault()
        jump()
      }
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [])
  return (
    <div className="flex min-h-0 flex-1">
      <div className="relative flex min-w-0 flex-1 flex-col">
        <div className="relative flex min-h-0 flex-1 flex-col">
          <div
            ref={scroller}
            className="min-h-0 flex-1 overflow-y-auto px-8 pt-7 pb-3"
            onScroll={(e) => {
              const el = e.currentTarget
              const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40
              stick.current = atBottom
              if (atBottom === away) setAway(!atBottom)
            }}
          >
            <div className="mx-auto flex w-full max-w-3xl flex-col gap-6">
              {c.transcript.length === 0 ? <Welcome /> : <Transcript />}
              {!inspector ? <TasksInline /> : null}
              <Activity />
            </div>
          </div>
          {away ? (
            <Button variant="secondary" size="sm" className="absolute bottom-3 left-1/2 -translate-x-1/2 rounded-full border shadow-lg shadow-black/50" onClick={jump}>
              <ArrowDown />
              Jump to bottom
              <Kbd>{modKey}End</Kbd>
            </Button>
          ) : null}
        </div>
        <div className="px-8 pt-2 pb-4">
          <div className="mx-auto flex w-full max-w-3xl flex-col gap-2">
            <Composer />
          </div>
        </div>
      </div>
      {inspector ? <Inspector /> : null}
    </div>
  )
}

function Stopped() {
  const c = useClient()
  const st = c.status
  if (st.kind !== "error" && st.kind !== "exited") return null
  return (
    <div className="grid flex-1 place-items-center p-8">
      <Card className="w-full max-w-lg">
        <CardHeader>
          <TriangleAlert className="mb-2 size-5 text-red" />
          <CardTitle>{st.kind === "error" ? "oah couldn't start" : `The engine stopped${st.code !== null ? ` (exit code ${st.code})` : ""}`}</CardTitle>
          <CardDescription className="select-text">{st.kind === "error" ? st.message : "Your session was saved as of the last change. Restart to continue it."}</CardDescription>
        </CardHeader>
        {st.kind === "exited" && st.stderr ? (
          <CardContent>
            <pre className="max-h-48 overflow-auto rounded-md border bg-background p-3 font-mono text-xs whitespace-pre-wrap text-muted-foreground select-text">{st.stderr}</pre>
          </CardContent>
        ) : null}
        <CardFooter className="gap-2">
          <Button onClick={() => c.restart()}>
            <RotateCw />
            {st.kind === "error" ? "Try again" : "Restart engine"}
          </Button>
          {st.kind === "error" ? <Button variant="outline" onClick={() => void c.t.openUrl("https://oah.dev")}>Get the oah CLI</Button> : null}
        </CardFooter>
      </Card>
    </div>
  )
}

export function App() {
  const c = useClient()
  const s = c.state

  useEffect(() => {
    void c.init()
  }, [])

  // The engine's notices (model switched, copied, errors…) as toasts.
  const toastAt = s?.toast?.at
  useEffect(() => {
    if (!s?.toast) return
    const t = s.toast
    ;(t.error ? toast.error : toast.success)(t.text, { id: `engine-${t.at}`, duration: 4000 })
  }, [toastAt])

  // Shortcuts that work anywhere (the composer handles its own keys; ⌘/Ctrl+B is the sidebar's).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return
      const k = e.key.toLowerCase()
      if (mod(e) && k === "n" && !e.shiftKey) c.newSession()
      else if (mod(e) && k === "o") void c.openFolder()
      else if (mod(e) && k === ",") c.setView(c.view === "settings" ? "chat" : "settings")
      else if (mod(e) && k === "t" && s && !s.picker) c.submit("/model")
      else if (mod(e) && k === "i") c.setPref("inspector", c.prefs.inspector === false)
      else if (e.key === "Escape") {
        if (document.querySelector("[role=dialog][data-state=open]")) return // the dialog closes itself
        if (s?.question) c.send({ op: "answer", answer: "" })
        else if (s?.busy) c.send({ op: "interrupt" })
        else if (c.view === "settings") c.setView("chat")
        else return
      } else return
      e.preventDefault()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [c, s])

  const ready = c.status.kind === "ready" && s
  return (
    <TooltipProvider delayDuration={400}>
      <div className="flex h-svh flex-col [--header-height:44px]">
        <SidebarProvider className="flex min-h-0 flex-1 flex-col" style={{ "--sidebar-width": "17rem" } as CSSProperties}>
          <TitleBar />
          <div className="flex min-h-0 flex-1">
            <AppSidebar />
            <SidebarInset className="min-h-0 min-w-0">
              {c.status.kind === "error" || c.status.kind === "exited" ? (
                <Stopped />
              ) : !ready ? (
                <div className="grid flex-1 place-items-center text-muted-foreground">
                  <Loader2 className="size-6 animate-spin text-primary" aria-label="Starting oah" />
                </div>
              ) : c.view === "settings" ? (
                <Settings />
              ) : (
                <Chat />
              )}
            </SidebarInset>
          </div>
        </SidebarProvider>
        {ready && s.picker ? <Picker /> : null}
        <Toaster position="bottom-center" offset={{ bottom: 150 }} />
      </div>
    </TooltipProvider>
  )
}
