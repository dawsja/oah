// Screens are hash routes, so Android's back button (the WebView's history) goes back a screen.
//   #/            the open desktop's chats (or pairing, with no desktop yet)
//   #/pair        pair a desktop
//   #/desktops    paired desktops
//   #/settings    this phone's settings
//   #/chat/<id>   a chat
//   #/new/<projectId>   a new chat in a project

import { useEffect, useState } from "react"

export type Route =
  | { screen: "chats" }
  | { screen: "pair" }
  | { screen: "desktops" }
  | { screen: "settings" }
  | { screen: "chat"; chatId: string }
  | { screen: "new"; projectId: string }

export function parse(hash: string): Route {
  const parts = hash.replace(/^#\/?/, "").split("/").filter(Boolean).map(decodeURIComponent)
  if (parts[0] === "pair") return { screen: "pair" }
  if (parts[0] === "desktops") return { screen: "desktops" }
  if (parts[0] === "settings") return { screen: "settings" }
  if (parts[0] === "chat" && parts[1]) return { screen: "chat", chatId: parts[1] }
  if (parts[0] === "new" && parts[1]) return { screen: "new", projectId: parts[1] }
  return { screen: "chats" }
}

export function useRoute(): Route {
  const [route, setRoute] = useState(() => parse(location.hash))
  useEffect(() => {
    const on = () => setRoute(parse(location.hash))
    window.addEventListener("hashchange", on)
    return () => window.removeEventListener("hashchange", on)
  }, [])
  return route
}

/** Opens a screen. `replace`: instead of the current one (back skips it). */
export function go(path: string, replace = false) {
  const hash = `#${path}`
  if (replace) {
    history.replaceState(null, "", hash)
    window.dispatchEvent(new HashChangeEvent("hashchange"))
  } else if (location.hash !== hash) {
    location.hash = hash
  }
}

/** Back a screen, or to the chats when there's nothing to go back to. */
export function back() {
  if (history.length > 1) history.back()
  else go("/", true)
}
