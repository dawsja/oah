import "@fontsource-variable/inter"
import "@fontsource-variable/jetbrains-mono"
import { StrictMode } from "react"
import { createRoot } from "react-dom/client"
import { App } from "./App"
import "./styles.css"
import { useApp } from "./store/app"

// Dev builds: scripts/dev-cdp.ts drives the app through the store.
if (import.meta.env.DEV) (window as unknown as { __oah: typeof useApp }).__oah = useApp

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
