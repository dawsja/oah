import { createContext, useContext, useEffect, useState } from "react"
import { Image as ImageIcon, ImageOff } from "lucide-react"
import { openImages, type ShownImage } from "@/components/ImageView"
import { git } from "@/lib/git"
import { ipc } from "@/lib/ipc"
import { useApp } from "@/store/app"
import { useScope } from "@/store/git"
import { cn } from "@/lib/utils"

/** Where a chat's images come from when it isn't this window's own open chat (the phone app
 *  reads them from the desktop). `key` keeps one source's images apart from another's. */
export type ImageSource = { key: string; chatImage: (file: string) => Promise<string>; readImage: (path: string) => Promise<string> }
export const ImageSourceContext = createContext<ImageSource | null>(null)

/** Saved chat images and files the agent read, as `data:` URLs, kept for the session. */
const cache = new Map<string, Promise<string>>()
function load(key: string, get: () => Promise<string>): Promise<string> {
  if (!cache.has(key)) cache.set(key, get().catch((e) => (cache.delete(key), Promise.reject(e))))
  return cache.get(key)!
}

function useSources(keys: string[], get: (i: number) => Promise<string>): (string | null | "gone")[] {
  const [srcs, setSrcs] = useState<(string | null | "gone")[]>(() => keys.map(() => null))
  const sig = keys.join("|")
  useEffect(() => {
    setSrcs(keys.map(() => null))
    keys.forEach((k, i) =>
      load(k, () => get(i)).then(
        (src) => setSrcs((s) => s.map((x, j) => (j === i ? src : x))),
        () => setSrcs((s) => s.map((x, j) => (j === i ? "gone" : x))),
      ),
    )
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sig])
  return srcs
}

/** A row of square thumbnails; a click opens them full-window. */
function Thumbs({ srcs, names, size = "size-20" }: { srcs: (string | null | "gone")[]; names: string[]; size?: string }) {
  const shown: ShownImage[] = srcs.flatMap((s, i) => (s && s !== "gone" ? [{ src: s, name: names[i] }] : []))
  return (
    <div className="flex flex-wrap gap-1.5">
      {srcs.map((s, i) =>
        s === "gone" ? (
          <div key={i} className={cn("rounded-md grid place-items-center border border-dashed text-faint", size)} title="This image is no longer saved">
            <ImageOff className="size-4" />
          </div>
        ) : s === null ? (
          <div key={i} className={cn("rounded-md animate-pulse border bg-muted", size)} />
        ) : (
          <button
            key={i}
            className={cn("rounded-md overflow-hidden border bg-background hover:border-ring/60", size)}
            onClick={() => openImages(shown, shown.findIndex((x) => x.src === s))}
            aria-label={`Open ${names[i]}`}
          >
            <img src={s} alt={names[i]} loading="lazy" className="size-full object-cover" />
          </button>
        ),
      )}
    </div>
  )
}

/** The images a message was sent with. Messages from before images were saved say how many. */
export function MessageImages({ files, count }: { files: string[]; count: number }) {
  const chatId = useApp((s) => s.chatId)
  const source = useContext(ImageSourceContext)
  const srcs = useSources(
    files.map((f) => `${source?.key ?? chatId}/${f}`),
    (i) => (source ? source.chatImage(files[i]) : ipc.chatImage(chatId!, files[i])),
  )
  if (!files.length) {
    return count ? (
      <div className="mt-1.5 flex items-center gap-1 text-[11.5px] text-muted-foreground">
        <ImageIcon className="size-3" /> {count} image{count > 1 ? "s" : ""}
      </div>
    ) : null
  }
  return (
    <div className="mt-2">
      <Thumbs srcs={srcs} names={files.map((_, i) => `Image ${i + 1}`)} />
    </div>
  )
}

/** An image file the agent looked at (a Read of a .png…), shown without opening the step. */
export function ReadImage({ path }: { path: string }) {
  const scope = useScope()
  const source = useContext(ImageSourceContext)
  const srcs = useSources([source ? `${source.key}|${path}` : `${scope?.projectId}|${scope?.cwd}|${path}`], () =>
    source ? source.readImage(path) : scope ? git.readImage(scope, path) : Promise.reject(new Error("no project")),
  )
  if (srcs[0] === "gone") return null
  return (
    <div className="mt-1 mb-1.5">
      <Thumbs srcs={srcs} names={[path.split(/[\\/]/).pop() ?? path]} size="size-24" />
    </div>
  )
}
