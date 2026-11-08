export type Attachment = { mime: string; data: string; name: string }
export const MAX_IMAGE = 5 * 1024 * 1024

/** Some file managers provide no MIME type for PNGs (or use octet-stream). */
export function attachmentMime(file: Pick<File, "type" | "name">): string | null {
  const types: Record<string, string> = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp" }
  const mime = file.type.toLowerCase()
  if (Object.values(types).includes(mime)) return mime
  if (mime && mime !== "application/octet-stream") return null
  return types[file.name.split(".").pop()?.toLowerCase() ?? ""] ?? null
}

export function readAttachment(file: File): Promise<Attachment> {
  const mime = attachmentMime(file)
  if (!mime || file.size > MAX_IMAGE) return Promise.reject(new Error("Unsupported or oversized image"))
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve({ mime, data: String(reader.result).split(",")[1] ?? "", name: file.name || "pasted image" })
    reader.onerror = reader.onabort = () => reject(new Error(`Could not read ${file.name || "image"}`))
    reader.readAsDataURL(file)
  })
}
