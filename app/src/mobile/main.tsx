// The oah phone app (android/): pairs with the desktop and drives its chats.

import "@fontsource-variable/inter"
import "@fontsource-variable/jetbrains-mono"
import { StrictMode } from "react"
import { createRoot } from "react-dom/client"
import { MobileApp } from "./App"
import { useMobile } from "./store"
import "../styles.css"
import "./mobile.css"

// Dev builds: tests drive the app through the store.
if (import.meta.env.DEV) Object.assign(window, { __mobile: useMobile })

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <MobileApp />
  </StrictMode>,
)
