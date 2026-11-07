import { useEffect } from "react"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { ChatView } from "@/components/chat/ChatView"
import { RightPanel } from "@/components/right/RightPanel"
import { SettingsPage } from "@/components/settings/SettingsPage"
import { Resizer } from "@/components/shell/Resizer"
import { Sidebar } from "@/components/shell/Sidebar"
import { TitleBar } from "@/components/shell/TitleBar"
import { useApp } from "@/store/app"
import { useChats } from "@/store/chats"

export function App() {
  const load = useApp((s) => s.load)
  const ready = useApp((s) => !!s.settings)
  const view = useApp((s) => s.view)
  const layout = useApp((s) => s.layout)
  const setLayout = useApp((s) => s.setLayout)

  useEffect(() => {
    load().then(() => useChats.getState().init())
  }, [load])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const s = useApp.getState()
      if (e.ctrlKey && (e.key === "n" || e.key === "N")) {
        e.preventDefault()
        useApp.setState({ chatId: null, view: "chat" })
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
      <div className="flex h-full flex-col">
        <TitleBar />
        {ready ? (
          <div className="flex min-h-0 flex-1">
            {layout.sidebarWidth > 0 ? (
              <>
                <div style={{ width: layout.sidebarWidth }} className="shrink-0">
                  <Sidebar />
                </div>
                <Resizer side="left" width={layout.sidebarWidth} min={200} max={460} onWidth={(w) => setLayout({ sidebarWidth: w })} />
              </>
            ) : null}
            <main className="min-w-0 flex-1">{view === "settings" ? <SettingsPage /> : <ChatView />}</main>
            {view === "chat" && layout.rightOpen ? (
              <>
                <Resizer side="right" width={layout.rightWidth} min={260} max={640} onWidth={(w) => setLayout({ rightWidth: w })} />
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
    </TooltipProvider>
  )
}
