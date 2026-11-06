// Images in prompts: ctrl+v reads the clipboard (an image is attached, text is
// pasted), and pasting or dragging an image file's path attaches that file. Each attachment shows up in
// the input as "[Image #N]" and is sent as an image part where the marker sits.
import { createHostClipboard } from "@opentui/core"
import { existsSync, readFileSync, rmSync, statSync } from "node:fs"
import { homedir, tmpdir } from "node:os"
import { basename, isAbsolute, join, resolve } from "node:path"
import type { ModelMessage } from "ai"

export interface PastedImage {
  n: number
  /** base64 */
  data: string
  mediaType: string
  name: string
  bytes: number
}

/** Providers cap images around 5 MB. */
export const MAX_IMAGE_BYTES = 5 * 1024 * 1024
export const MARKER = /\[Image #(\d+)\]/g

const EXT: Record<string, string> = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp" }

/** Image type from the file's first bytes. */
export function sniff(b: Uint8Array): string | null {
  if (b[0] === 0x89 && b[1] === 0x50 && b[2] === 0x4e && b[3] === 0x47) return "image/png"
  if (b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff) return "image/jpeg"
  if (b[0] === 0x47 && b[1] === 0x49 && b[2] === 0x46) return "image/gif"
  if (b[0] === 0x52 && b[1] === 0x49 && b[2] === 0x46 && b[3] === 0x46 && b[8] === 0x57 && b[9] === 0x45 && b[10] === 0x42 && b[11] === 0x50) return "image/webp"
  return null
}

/** Paths in a paste: terminals quote or backslash-escape dragged files. */
export function pastedPaths(text: string): string[] {
  const t = text.trim()
  if (!t || t.length > 4096) return []
  const out: string[] = []
  const re = /'([^']+)'|"([^"]+)"|((?:\\.|[^\s'"])+)/g
  for (const m of t.matchAll(re)) {
    let p = m[1] ?? m[2] ?? m[3]!.replace(/\\(.)/g, "$1")
    if (p.startsWith("file://")) p = decodeURIComponent(p.slice(7))
    if (p.startsWith("~/")) p = homedir() + p.slice(1)
    out.push(p)
  }
  return out
}

/** An image file at `path`, or an error message, or null if it isn't an image path. */
export function imageFromFile(path: string, cwd: string): Omit<PastedImage, "n"> | string | null {
  const full = isAbsolute(path) ? path : resolve(cwd, path)
  const ext = full.split(".").pop()?.toLowerCase() ?? ""
  if (!EXT[ext] || !existsSync(full)) return null
  try {
    if (!statSync(full).isFile()) return null
    const bytes = readFileSync(full)
    if (bytes.length > MAX_IMAGE_BYTES) return `${basename(full)} is ${(bytes.length / 1048576).toFixed(1)} MB — images must be under 5 MB`
    return { data: bytes.toString("base64"), mediaType: sniff(bytes) ?? EXT[ext]!, name: basename(full), bytes: bytes.length }
  } catch (e: any) {
    return `Couldn't read ${basename(full)}: ${e?.message ?? e}`
  }
}

let host: ReturnType<typeof createHostClipboard> | null = null

/** Read an image from the system clipboard, or null when there isn't one. */
export async function clipboardImage(): Promise<Omit<PastedImage, "n"> | string | null> {
  let bytes: Uint8Array | null = null
  let type = ""
  try {
    host ??= createHostClipboard({ timeoutMs: 3000, maxReadBytes: MAX_IMAGE_BYTES * 4 })
    const r = await host.read({ preferredTypes: ["image/png", "image/jpeg", "image/webp", "image/gif"] })
    if (r.status === "read" && r.representation.mimeType.startsWith("image/")) {
      bytes = r.representation.bytes
      type = r.representation.mimeType
    } else if (r.status === "limit-exceeded") return "The image on the clipboard is too large (5 MB max)"
  } catch {}
  if (!bytes) bytes = await clipboardViaCli()
  if (!bytes?.length) return null
  if (bytes.length > MAX_IMAGE_BYTES) return `The image on the clipboard is ${(bytes.length / 1048576).toFixed(1)} MB — images must be under 5 MB`
  const mediaType = sniff(bytes) ?? type
  if (!mediaType) return null
  return { data: Buffer.from(bytes).toString("base64"), mediaType, name: "clipboard", bytes: bytes.length }
}

/** Read text from the system clipboard, or null when it holds none. */
export async function clipboardText(): Promise<string | null> {
  try {
    host ??= createHostClipboard({ timeoutMs: 3000, maxReadBytes: MAX_IMAGE_BYTES * 4 })
    const r = await host.read({ preferredTypes: ["text/plain"] })
    if (r.status === "read") {
      const text = new TextDecoder().decode(r.representation.bytes)
      if (text) return text
    }
  } catch {}
  const cmds =
    process.platform === "darwin"
      ? [["pbpaste"]]
      : [...(process.env.WAYLAND_DISPLAY ? [["wl-paste", "--no-newline"]] : []), ...(process.env.DISPLAY ? [["xclip", "-selection", "clipboard", "-o"]] : [])]
  for (const cmd of cmds) {
    try {
      const p = Bun.spawn(cmd, { stdout: "pipe", stderr: "ignore", stdin: "ignore" })
      const out = await new Response(p.stdout).text()
      if ((await p.exited) === 0 && out) return out
    } catch {}
  }
  return null
}

/** Fallback readers for when the native clipboard can't provide an image. */
async function clipboardViaCli(): Promise<Uint8Array | null> {
  const tmp = join(tmpdir(), `oah-clip-${process.pid}.png`)
  const cmds: { cmd: string[]; file?: string }[] =
    process.platform === "darwin"
      ? [{ cmd: ["osascript", "-e", "set png to (the clipboard as «class PNGf»)", "-e", `set f to (open for access POSIX file "${tmp}" with write permission)`, "-e", "set eof f to 0", "-e", "write png to f", "-e", "close access f"], file: tmp }]
      : [
          ...(process.env.WAYLAND_DISPLAY ? [{ cmd: ["wl-paste", "--no-newline", "--type", "image/png"] }] : []),
          ...(process.env.DISPLAY ? [{ cmd: ["xclip", "-selection", "clipboard", "-t", "image/png", "-o"] }] : []),
        ]
  for (const { cmd, file } of cmds) {
    try {
      const p = Bun.spawn(cmd, { stdout: "pipe", stderr: "ignore", stdin: "ignore" })
      let out = new Uint8Array(await new Response(p.stdout).arrayBuffer())
      if ((await p.exited) !== 0) continue
      if (file) {
        out = new Uint8Array(readFileSync(file))
        rmSync(file, { force: true })
      }
      if (sniff(out)) return out
    } catch {}
  }
  return null
}

/** User message content: text with each [Image #N] marker replaced by that image. */
export function userContent(text: string, images: Map<number, PastedImage>): ModelMessage["content"] & any {
  const used = [...text.matchAll(MARKER)].filter((m) => images.has(Number(m[1])))
  if (!used.length) return text
  const parts: any[] = []
  let last = 0
  for (const m of used) {
    const before = text.slice(last, m.index! + m[0].length)
    if (before.trim()) parts.push({ type: "text", text: before })
    const img = images.get(Number(m[1]))!
    parts.push({ type: "image", image: img.data, mediaType: img.mediaType })
    last = m.index! + m[0].length
  }
  const rest = text.slice(last)
  if (rest.trim()) parts.push({ type: "text", text: rest })
  return parts
}

/** Image numbers referenced in `text` that exist. */
export const referenced = (text: string, images: Map<number, PastedImage>) => [...new Set([...text.matchAll(MARKER)].map((m) => Number(m[1])))].filter((n) => images.has(n))
