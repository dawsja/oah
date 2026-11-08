import { useEffect } from "react"
import { ImageLightbox } from "@/components/ImageView"
import { TouchContext } from "@/components/chat/touch"
import { Toaster } from "@/components/ui/sonner"
import { ChatScreen, NewChatScreen } from "./ChatScreen"
import { ChatsScreen } from "./Chats"
import { DesktopsScreen } from "./Desktops"
import { PairScreen } from "./Pair"
import { useRoute } from "./route"
import { useMobile } from "./store"

export function MobileApp() {
  const route = useRoute()
  const loaded = useMobile((s) => s.loaded)
  const paired = useMobile((s) => s.desktops.length > 0)
  const desktopId = useMobile((s) => s.desktopId)

  useEffect(() => {
    void useMobile.getState().init()
  }, [])

  let screen
  if (!loaded) screen = <div className="h-full bg-background" />
  else if (route.screen === "pair" || !paired) screen = <PairScreen />
  else if (route.screen === "desktops" || !desktopId) screen = <DesktopsScreen />
  else if (route.screen === "chat") screen = <ChatScreen key={route.chatId} chatId={route.chatId} />
  else if (route.screen === "new") screen = <NewChatScreen key={route.projectId} projectId={route.projectId} />
  else screen = <ChatsScreen />

  return (
    <TouchContext.Provider value={true}>
      {screen}
      <ImageLightbox />
      <Toaster position="top-center" />
    </TouchContext.Provider>
  )
}
