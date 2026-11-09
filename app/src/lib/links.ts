/** Where a link in the agent's answer goes. Agent output is untrusted: only web and mail links
 *  leave the app (in the browser, never in the app's own window); a relative path is a file in
 *  the project, shown in the file viewer; anything else does nothing. */
export type LinkTarget = { kind: "url"; url: string } | { kind: "file"; path: string; line?: number } | { kind: "none" }

export function linkTarget(href: string | undefined): LinkTarget {
  const h = (href ?? "").trim()
  if (!h || h.startsWith("#")) return { kind: "none" }
  if (/^(https?:|mailto:)/i.test(h)) return { kind: "url", url: h }
  // Any other scheme (javascript:, file:, data:…), or a protocol-relative URL.
  if (/^[a-z][a-z\d+.-]*:/i.test(h) || h.startsWith("//")) return { kind: "none" }
  let path = h
  let line: number | undefined
  const anchor = /#L?(\d+)(?:-L?\d+)?$/i.exec(path)
  if (anchor) {
    line = Number(anchor[1])
    path = path.slice(0, anchor.index)
  } else {
    path = path.replace(/[#?].*$/, "")
  }
  // `file.ts:12`, as agents often write.
  const colon = /:(\d+)(?::\d+)?$/.exec(path)
  if (colon && line === undefined) {
    line = Number(colon[1])
    path = path.slice(0, colon.index)
  }
  try {
    path = decodeURIComponent(path)
  } catch {
    return { kind: "none" }
  }
  path = path.replace(/^\.\//, "")
  if (!path || path.startsWith("/") || path.split("/").includes("..")) return { kind: "none" }
  return line ? { kind: "file", path, line } : { kind: "file", path }
}
