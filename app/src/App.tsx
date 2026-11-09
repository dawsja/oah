import { useEffect } from "react"
import { PhonePairingHost } from "@/components/settings/PhoneSection"
import { PromptHost } from "@/components/Prompt"
import { ImageLightbox } from "@/components/ImageView"
import { RemoteFolderHost } from "@/components/settings/RemoteFolder"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { ChatView } from "@/components/chat/ChatView"
import { RightPanel } from "@/components/right/RightPanel"
import { SettingsPage } from "@/components/settings/SettingsPage"
import { Resizer } from "@/components/shell/Resizer"
import { Sidebar } from "@/components/shell/Sidebar"
import { TitleBar } from "@/components/shell/TitleBar"
import { TerminalPanel } from "@/components/terminal/TerminalPanel"
import { defaultLayout, useApp } from "@/store/app"
import { useChats } from "@/store/chats"
import { initVoice } from "@/lib/voice"
import { initRemote } from "@/lib/remote"
import { ChatSearch, openChatSearch } from "@/components/shell/ChatSearch"
import { inTerminal } from "@/lib/terminal"
import { useTerminals } from "@/store/terminals"

export function App() {
  const load = useApp((s) => s.load)
  const ready = useApp((s) => !!s.settings)
  const view = useApp((s) => s.view)
  const layout = useApp((s) => s.layout)
  const setLayout = useApp((s) => s.setLayout)

  useEffect(() => {
    load().then(() => {
      useChats.getState().init()
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
      } else if (inTerminal(e.target) && e.key !== ",") {
        // The shell's keys (Ctrl+N is its own); only the terminal toggle and Settings get out.
        return
      } else if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && (e.key === "k" || e.key === "K")) {
        e.preventDefault()
        openChatSearch()
      } else if (e.ctrlKey && (e.key === "n" || e.key === "N")) {
        e.preventDefault()
        useApp.setState({ chatId: null, draftCwd: null, view: "chat" })
      } else if (e.ctrlKey && e.key === ",") {
        e.preventDefault()
        s.view === "settings" ? s.closeSettings() : s.openSettings()
      } else if (e.key === "Escape" && s.view === "settings") {
        s.closeSettings()
      }
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [])

  return (
    <TooltipProvider delayDuration={400}>
      <div className="flex h-full min-h-0 flex-col overflow-clip">
        <TitleBar />
        {ready ? (
          <div className="flex min-h-0 flex-1">
            {layout.sidebarWidth > 0 ? (
              <>
                <div style={{ width: layout.sidebarWidth }} className="shrink-0">
                  <Sidebar />
                </div>
                <Resizer side="left" defaultWidth={defaultLayout.sidebarWidth} width={layout.sidebarWidth} min={200} max={460} onWidth={(w) => setLayout({ sidebarWidth: w })} />
              </>
            ) : null}
            <main className="flex min-w-0 flex-1 flex-col">
              <div className="min-h-0 flex-1">{view === "settings" ? <SettingsPage /> : <ChatView />}</div>
              <TerminalPanel />
            </main>
            {view === "chat" && layout.rightOpen ? (
              <>
                <Resizer side="right" defaultWidth={defaultLayout.rightWidth} width={layout.rightWidth} min={260} max={640} onWidth={(w) => setLayout({ rightWidth: w })} />
                <div style={{ width: layout.rightWidth }} className="shrink-0">
                  <RightPanel />
                </div>
              </>
            ) : null}
          </div>
        ) : (
          <div className="flex-1 bg-background" />
        )}
      </div>
      <Toaster position="bottom-right" />
      <PromptHost />
      <PhonePairingHost />
      <ImageLightbox />
      <ChatSearch />
      <RemoteFolderHost />
    </TooltipProvider>
  )
}
