import { invoke } from "@tauri-apps/api/core"
import { filename, imageBlob, toPng } from "@/lib/image-download"

async function action(action: "copy" | "save", image: { src: string; name: string }): Promise<boolean> {
  const original = await imageBlob(image.src)
  const blob = action === "copy" && original.type !== "image/png" ? await toPng(original) : original
  const data = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(String(reader.result).split(",")[1])
    reader.onerror = () => reject(reader.error)
    reader.readAsDataURL(blob)
  })
  const result = await invoke<{ saved: boolean }>("image_action", { action, name: filename(image), mime: blob.type, data })
  return result.saved
}

export const phoneImageActions = {
  save: (image: { src: string; name: string }) => action("save", image),
  copy: async (src: string) => { await action("copy", { src, name: "Image" }) },
}
