// A phone's camera makes photos larger than the desktop takes (MAX_IMAGE). Those are scaled
// down to a JPEG that fits, rather than refused.

import { attachmentMime, MAX_IMAGE } from "@/lib/attachments"

/** Longest edges to try, largest first. */
const EDGES = [3072, 2048, 1440]

/** `width`×`height` scaled so its longer edge is at most `edge` (never up). */
export function fitSize(width: number, height: number, edge: number): { width: number; height: number } {
  const scale = Math.min(1, edge / Math.max(width, height))
  return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) }
}

/** `photo.heic` → `photo.jpg`. */
export function jpegName(name: string): string {
  return `${name.replace(/\.[^./\\]*$/, "") || "photo"}.jpg`
}

/** The image itself when it fits (or can't be redrawn: a GIF would lose its frames), else a
 *  smaller JPEG of it. */
export async function fitImage(file: File, max = MAX_IMAGE): Promise<File> {
  const mime = attachmentMime(file)
  if (file.size <= max || !mime || mime === "image/gif") return file
  try {
    const bitmap = await createImageBitmap(file)
    try {
      for (const edge of EDGES) {
        const { width, height } = fitSize(bitmap.width, bitmap.height, edge)
        const canvas = document.createElement("canvas")
        canvas.width = width
        canvas.height = height
        const ctx = canvas.getContext("2d")
        if (!ctx) return file
        // JPEG has no transparency: a see-through PNG goes on white.
        ctx.fillStyle = "#fff"
        ctx.fillRect(0, 0, width, height)
        ctx.drawImage(bitmap, 0, 0, width, height)
        const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/jpeg", 0.85))
        if (blob && blob.size <= max) return new File([blob], jpegName(file.name), { type: "image/jpeg" })
      }
    } finally {
      bitmap.close()
    }
  } catch {
    /* Not something this WebView can decode: it's refused as too large below. */
  }
  return file
}
