import "@fontsource-variable/inter"
import "@fontsource-variable/jetbrains-mono"
import { StrictMode } from "react"
import { createRoot } from "react-dom/client"
import { App } from "./App"
import "./styles.css"

// Browser development only: `?shot=<ms>` holds the page's load event (an image the
// dev bridge serves late) so headless Chrome's --screenshot waits for the UI.
const shot = !("__TAURI_INTERNALS__" in window) && new URLSearchParams(location.search).get("shot")
if (shot) {
  const hold = new Image()
  hold.src = `http://127.0.0.1:1430/hold?ms=${shot}`
  hold.hidden = true
  document.body.append(hold)
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
