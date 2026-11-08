type Image = { src: string; name: string }

/** File images are already loaded from the session's host. Decode them without a webview fetch. */
export async function imageBlob(src: string): Promise<Blob> {
  if (/^data:/i.test(src)) {
    const match = /^data:(image\/[a-z0-9.+-]+);base64,([\s\S]*)$/i.exec(src)
    if (!match) throw new Error("Unsupported image data")
    const binary = atob(match[2].replace(/\s/g, ""))
    return new Blob([Uint8Array.from(binary, (c) => c.charCodeAt(0))], { type: match[1] })
  }
  const response = await fetch(src)
  if (!response.ok) throw new Error(`Could not load image (${response.status})`)
  return response.blob()
}

function filename(image: Image): string {
  const name = image.name.split(/[\\/]/).pop()?.replace(/[<>:"|?*\x00-\x1f]/g, "_").trim().replace(/[. ]+$/, "") || "Image"
  if (/\.(png|jpe?g|gif|webp|avif|bmp|ico|svg)$/i.test(name)) return name
  const mime = /^data:image\/([^;,]+)/i.exec(image.src)?.[1].toLowerCase()
  const ext = mime && ({ jpeg: "jpg", png: "png", gif: "gif", webp: "webp", avif: "avif", bmp: "bmp", "x-icon": "ico", "svg+xml": "svg" } as Record<string, string>)[mime]
  return ext ? `${name}.${ext}` : name
}

/** The dialog and writer belong to this device, never to the image's project or execution host. */
export async function saveImageLocally(image: Image): Promise<boolean> {
  const { save } = await import("@tauri-apps/plugin-dialog")
  const path = await save({ defaultPath: filename(image), title: "Save image" })
  if (!path) return false
  const bytes = new Uint8Array(await (await imageBlob(image.src)).arrayBuffer())
  const { invoke } = await import("@tauri-apps/api/core")
  await invoke("save_bytes", { path, data: Array.from(bytes) })
  return true
}

/** Write to the clipboard on this device using the same decoded bytes as Save. */
export async function copyImageLocally(src: string): Promise<void> {
  const blob = await imageBlob(src)
  const png = blob.type === "image/png" ? blob : await toPng(blob)
  const { writeImage } = await import("@tauri-apps/plugin-clipboard-manager")
  await writeImage(new Uint8Array(await png.arrayBuffer()))
}

async function toPng(blob: Blob): Promise<Blob> {
  const bmp = await createImageBitmap(blob)
  const canvas = document.createElement("canvas")
  canvas.width = bmp.width
  canvas.height = bmp.height
  canvas.getContext("2d")!.drawImage(bmp, 0, 0)
  bmp.close()
  return new Promise((resolve, reject) => canvas.toBlob((b) => (b ? resolve(b) : reject(new Error("no PNG"))), "image/png"))
}

