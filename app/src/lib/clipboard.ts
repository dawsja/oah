// Copying goes through Tauri's clipboard plugin: the webview's own clipboard API can be refused
// (WebKitGTK on Linux), the plugin always works.

import { toast } from "sonner"

export function copyText(text: string) {
  import("@tauri-apps/plugin-clipboard-manager")
    .then((c) => c.writeText(text))
    .catch((e) => toast.error(`Couldn't copy: ${e}`))
}
