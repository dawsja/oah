import { useContext, useEffect, useState } from "react"
import { ImageDown, ImageOff } from "lucide-react"
import { openImages } from "@/components/ImageView"
import { git } from "@/lib/git"
import { imageSource, remoteHost } from "@/lib/images"
import { useScope } from "@/store/git"
import { ImageSourceContext } from "./ChatImages"
import { CwdContext } from "./cwd"

/** Remote images the user chose to load this session (by URL). */
const allowed = new Set<string>()

/** Load file images where the chat runs, including absolute paths outside the checkout. A
 *  remote image waits for a click: agent output is untrusted, and loading it would tell that
 *  server this computer's address, and whatever the URL carries, with no one asking. */
export function MarkdownImage({ src, alt }: { src?: string; alt?: string }) {
  const scope = useScope()
  const chatCwd = useContext(CwdContext)
  const other = useContext(ImageSourceContext)
  const source = imageSource(src ?? "")
  const path = source?.kind === "file" ? source.path : null
  const projectId = scope?.projectId
  const cwd = chatCwd ?? scope?.cwd ?? null
  const key = JSON.stringify([other?.key ?? projectId, cwd, src])
  const [loaded, setLoaded] = useState<{ key: string; src?: string; error?: string } | null>(null)
  const host = source?.kind === "url" ? remoteHost(source.src) : null
  const [, setAllowedNow] = useState(0)
  useEffect(() => {
    if (!path) return
    let active = true
    setLoaded(null)
    const request = other ? other.readImage(path) : projectId ? git.readImage({ projectId, cwd }, path) : Promise.reject("No project selected")
    request.then(
      (data) => active && setLoaded({ key, src: data }),
      (error) => active && setLoaded({ key, error: String(error) }),
    )
    return () => { active = false }
  }, [key, path, projectId, cwd, other])
  if (!source) return null
  const current = loaded?.key === key ? loaded : null
  const resolved = current?.error ? undefined : source.kind === "url" ? source.src : current?.src
  const name = alt || "Image"
  if (host && source?.kind === "url" && !allowed.has(source.src)) {
    const url = source.src
    return (
      <button
        className="my-1 inline-flex items-center gap-1.5 rounded-md border border-input px-2 py-1 text-sm text-muted-foreground hover:bg-accent hover:text-foreground"
        onClick={() => {
          allowed.add(url)
          setAllowedNow((n) => n + 1)
        }}
        title={url}
      >
        <ImageDown className="size-4" />
        Load {name} from {host}
      </button>
    )
  }
  if (current?.error) {
    return <span className="my-1 inline-flex items-center gap-1.5 text-sm text-muted-foreground" title={current.error}><ImageOff className="size-4" />{name}: image unavailable</span>
  }
  if (!resolved) return <span className="my-1 inline-block text-sm text-muted-foreground" role="status">Loading {name}…</span>
  return (
    <button className="my-1 inline-block" onClick={() => openImages([{ src: resolved, name }])} aria-label={`Open ${name}`}>
      <img src={resolved} alt={alt ?? ""} loading="lazy" className="rounded-lg max-h-96 max-w-full border object-contain" onError={() => setLoaded({ key, error: "The image could not be displayed" })} />
    </button>
  )
}
