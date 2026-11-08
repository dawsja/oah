// The oah phone app (android/): pairs with the desktop and drives its chats.

import "@fontsource-variable/inter"
import "@fontsource-variable/jetbrains-mono"
import { StrictMode } from "react"
import { createRoot } from "react-dom/client"
import { MobileApp } from "./App"
import { initPhoneTheme } from "./theme"
import { useMobile } from "./store"
import "../styles.css"
import "./mobile.css"

const stopTheme = initPhoneTheme()
if (import.meta.hot) import.meta.hot.dispose(stopTheme)

// Dev builds: tests drive the app through the store.
if (import.meta.env.DEV) Object.assign(window, { __mobile: useMobile })

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <MobileApp />
  </StrictMode>,
)
