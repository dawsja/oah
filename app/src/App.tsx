import { cn } from "@/lib/utils"
import { useEffect } from "react"
import { PhonePairingHost } from "@/components/settings/PhoneSection"
import { PromptHost } from "@/components/Prompt"
import { ImageLightbox } from "@/components/ImageView"
import { RemoteFolderHost } from "@/components/settings/RemoteFolder"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { Panes } from "@/components/shell/Panes"
import { focusPaneInput } from "@/components/shell/pane"
import { RightPanel } from "@/components/right/RightPanel"
import { SettingsPage } from "@/components/settings/SettingsPage"
import { Resizer } from "@/components/shell/Resizer"
import { Sidebar } from "@/components/shell/Sidebar"
import { TitleBar } from "@/components/shell/TitleBar"
import { TerminalPanel } from "@/components/terminal/TerminalPanel"
import { ErrorBoundary } from "@/components/ErrorBoundary"
import { Button } from "@/components/ui/button"
import { defaultLayout, paneOf, useApp } from "@/store/app"
import { useChats } from "@/store/chats"
import { enterCli, leaveCli, useCli } from "@/store/cli"
import { initVoice } from "@/lib/voice"
import { initRemote } from "@/lib/remote"
import { initAttention } from "@/lib/attention"
import { ChatSearch, openChatSearch } from "@/components/shell/ChatSearch"
import { inTerminal } from "@/lib/terminal"
import { useTerminals } from "@/store/terminals"

/** The chords that work on panes wherever the cursor is (see `isAppChord`): Ctrl+1 and Ctrl+2
 *  go to a pane, Ctrl+Shift+\ closes the active one, Ctrl+Shift+M takes the active pane's chat
 *  into its agent's CLI and back. True when `e` was one of them. */
function paneChord(e: KeyboardEvent): boolean {
  if (!e.ctrlKey || e.altKey) return false
  const s = useApp.getState()
  if (!e.shiftKey && (e.code === "Digit1" || e.code === "Digit2")) {
    const pane = s.panes[e.code === "Digit1" ? 0 : 1]
    if (!pane) return false
    s.focusPane(pane.id)
    focusPaneInput(pane.id)
    return true
  }
  if (e.shiftKey && e.code === "Backslash") {
    if (s.panes.length < 2) return false
    s.closePane()
    focusPaneInput(useApp.getState().activePane)
    return true
  }
  if (e.shiftKey && e.code === "KeyM") {
    const chatId = paneOf(s).chatId
    if (!chatId || paneOf(s).viewer) return false
    void (useCli.getState().sessions[chatId] ? leaveCli(chatId) : enterCli(chatId)).then(() => focusPaneInput(useApp.getState().activePane))
    return true
  }
  return false
}

export function App() {
  const load = useApp((s) => s.load)
  const ready = useApp((s) => !!s.settings)
  const error = useApp((s) => s.error)
  const view = useApp((s) => s.view)
  const layout = useApp((s) => s.layout)
  const setLayout = useApp((s) => s.setLayout)

  useEffect(() => {
    load().then(() => {
      void useChats.getState().init().then(initAttention)
      initVoice()
      initRemote()
    })
  }, [load])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const s = useApp.getState()
      if (e.ctrlKey && !e.altKey && e.code === "Backquote") {
        e.preventDefault()
        useTerminals.getState().toggle()
      } else if (s.view === "chat" && paneChord(e)) {
        // Between panes, and a chat's way in and out of its CLI: from inside a terminal too.
        e.preventDefault()
      } else if (inTerminal(e.target) && e.key !== ",") {
        // The shell's keys (Ctrl+N is its own); only those, the terminal toggle and Settings get out.
        return
      } else if (e.ctrlKey && !e.shiftKey && !e.altKey && e.code === "Backslash" && s.view === "chat") {
        // A second pane, or over to the other one (in a terminal Ctrl+\ is the program's).
        e.preventDefault()
        const other = s.panes.find((p) => p.id !== s.activePane)
        if (other) s.focusPane(other.id)
        else s.splitPane(undefined, "right")
        focusPaneInput(useApp.getState().activePane)
      } else if (e.ctrlKey && e.altKey && !e.shiftKey && !e.getModifierState("AltGraph") && e.code === "Backslash" && s.view === "chat") {
        // A second pane below, or two turned the other way (side by side ⇄ one above the other).
        e.preventDefault()
        if (s.panes.length > 1) s.setLayout({ splitDir: s.layout.splitDir === "down" ? "right" : "down" })
        else {
          s.splitPane(undefined, "down")
          focusPaneInput(useApp.getState().activePane)
        }
      } else if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && (e.key === "k" || e.key === "K")) {
        e.preventDefault()
        openChatSearch()
      } else if (e.ctrlKey && (e.key === "n" || e.key === "N")) {
        e.preventDefault()
        s.setPane({ chatId: null, draftCwd: null, viewer: null })
        useApp.setState({ view: "chat" })
      } else if (e.ctrlKey && e.key === ",") {
        e.preventDefault()
        s.view === "settings" ? s.closeSettings() : s.openSettings()
      } else if (e.key === "Escape" && s.view === "settings" && !e.defaultPrevented) {
        // A menu, select or dialog inside Settings closes first (Radix marks its Esc handled).
        s.closeSettings()
      }
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [])

  return (
    <TooltipProvider delayDuration={400}>
      <div className="flex h-full min-h-0 flex-col overflow-clip bg-sidebar">
        <TitleBar />
        {ready ? (
          <div className={cn("flex min-h-0 flex-1 pr-2 pb-2", layout.sidebarWidth > 0 ? "" : "pl-2")}>
            {layout.sidebarWidth > 0 ? (
              <>
                <div style={{ width: layout.sidebarWidth }} className="shrink-0">
                  <ErrorBoundary label="the sidebar">
                    <Sidebar />
                  </ErrorBoundary>
                </div>
                <Resizer side="left" defaultWidth={defaultLayout.sidebarWidth} width={layout.sidebarWidth} min={200} max={460} onWidth={(w) => setLayout({ sidebarWidth: w })} />
              </>
            ) : null}
            <main className="flex min-w-0 flex-1 flex-col">
              {view === "settings" ? (
                <div className="min-h-0 flex-1 overflow-clip rounded-lg border bg-background">
                  <ErrorBoundary label="Settings">
                    <SettingsPage />
                  </ErrorBoundary>
                </div>
              ) : (
                <Panes />
              )}
              <TerminalPanel />
            </main>
            {view === "chat" && layout.rightOpen ? (
              <>
                <Resizer side="right" defaultWidth={defaultLayout.rightWidth} width={layout.rightWidth} min={260} max={640} onWidth={(w) => setLayout({ rightWidth: w })} />
                <div style={{ width: layout.rightWidth }} className="shrink-0 overflow-clip rounded-lg border bg-background">
                  <ErrorBoundary label="the side panel">
                    <RightPanel />
                  </ErrorBoundary>
                </div>
              </>
            ) : null}
          </div>
        ) : error ? (
          // Settings couldn't be loaded at all: say so, instead of an empty window.
          <div role="alert" className="flex flex-1 flex-col items-center justify-center gap-3 bg-background p-6 text-center">
            <p className="text-[13px] font-medium">oah couldn't start.</p>
            <p className="max-w-md font-mono text-[11.5px] break-all text-muted-foreground">{error}</p>
            <Button size="sm" variant="outline" onClick={() => void load()}>
              Try again
            </Button>
          </div>
        ) : (
          <div className="flex-1 bg-background" />
        )}
      </div>
      <Toaster position="bottom-right" />
      <PromptHost />
      <PhonePairingHost />
      <ImageLightbox inWindow />
      <ChatSearch />
      <RemoteFolderHost />
    </TooltipProvider>
  )
}
