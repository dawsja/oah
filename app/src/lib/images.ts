import { defaultUrlTransform } from "react-markdown"

export type ImageSource = { kind: "url"; src: string } | { kind: "file"; path: string }

/** Markdown file images live on the agent's machine, not the webview's origin. */
export function imageSource(src: string): ImageSource | null {
  if (/^(https?:\/\/|\/\/|blob:)/i.test(src)) return { kind: "url", src }
  if (/^data:image\/(png|jpe?g|gif|webp|avif|bmp|x-icon);base64,[a-z\d+/=\s]+$/i.test(src)) return { kind: "url", src }
  let path = src
  try {
    if (/^file:/i.test(path)) {
      const url = new URL(path)
      path = url.hostname && url.hostname !== "localhost" ? `//${url.hostname}${url.pathname}` : url.pathname
      // file:///C:/... is a Windows path, including when rendered on another OS.
      path = path.replace(/^\/([a-z]:\/)/i, "$1")
    } else if (!/^[a-z]:[\\/]/i.test(path) && /^[a-z][a-z\d+.-]*:/i.test(path)) {
      return null
    }
    path = decodeURIComponent(path)
  } catch {
    return null
  }
  return path && !/[\x00-\x1f]/.test(path) ? { kind: "file", path } : null
}

/** Preserve file image paths without relaxing the URL rules for ordinary links. */
export function markdownUrlTransform(url: string, key: string, node: { tagName: string }): string {
  return key === "src" && node.tagName === "img" ? (imageSource(url) ? url : "") : defaultUrlTransform(url)
}
