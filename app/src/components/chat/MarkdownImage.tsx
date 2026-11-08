import { useContext, useEffect, useState } from "react"
import { ImageOff } from "lucide-react"
import { openImages } from "@/components/ImageView"
import { git } from "@/lib/git"
import { imageSource } from "@/lib/images"
import { useScope } from "@/store/git"
import { ImageSourceContext } from "./ChatImages"
import { CwdContext } from "./cwd"

/** Load file images where the chat runs, including absolute paths outside the checkout. */
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
  if (current?.error) {
    return <span className="my-1 inline-flex items-center gap-1.5 text-sm text-muted-foreground" title={current.error}><ImageOff className="size-4" />{name}: image unavailable</span>
  }
  if (!resolved) return <span className="my-1 inline-block text-sm text-muted-foreground" role="status">Loading {name}…</span>
  return (
    <button className="my-1 inline-block" onClick={() => openImages([{ src: resolved, name }])} aria-label={`Open ${name}`}>
      <img src={resolved} alt={alt ?? ""} loading="lazy" className="rounded-md max-h-96 max-w-full border object-contain" onError={() => setLoaded({ key, error: "The image could not be displayed" })} />
    </button>
  )
}
