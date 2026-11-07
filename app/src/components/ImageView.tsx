import { useEffect, useState } from "react"
import { ChevronLeft, ChevronRight, Copy, Download, Maximize2, Minimize2, X } from "lucide-react"
import { create } from "zustand"
import { toast } from "sonner"
import { Tip } from "@/components/Tip"
import { cn } from "@/lib/utils"

/** An image to show: a `data:` URL (or any URL the webview can load) and a name. */
export type ShownImage = { src: string; name: string }

/** The image on a checkerboard, fitted to the space or at its actual size (click to switch),
 *  with its size and Copy / Save. */
export function ImageStage({ image, className, toolbar = true }: { image: ShownImage; className?: string; toolbar?: boolean }) {
  const [actual, setActual] = useState(false)
  const [size, setSize] = useState<{ w: number; h: number } | null>(null)
  useEffect(() => {
    setActual(false)
    setSize(null)
  }, [image.src])
  return (
    <div className={cn("flex min-h-0 flex-col", className)}>
      {toolbar ? (
        <div className="flex h-9 shrink-0 items-center gap-1 px-3 text-[11.5px] text-muted-foreground">
          <span className="truncate font-mono">{image.name}</span>
          {size ? <span className="shrink-0 font-mono text-faint">· {size.w}×{size.h}</span> : null}
          <span className="ml-auto flex items-center gap-0.5">
            <IconButton label={actual ? "Fit to window" : "Actual size"} onClick={() => setActual(!actual)}>
              {actual ? <Minimize2 /> : <Maximize2 />}
            </IconButton>
            <IconButton label="Copy image" onClick={() => void copyImage(image)}>
              <Copy />
            </IconButton>
            <IconButton label="Save as…" onClick={() => void saveImage(image)}>
              <Download />
            </IconButton>
          </span>
        </div>
      ) : null}
      <div className={cn("checkerboard min-h-0 flex-1 overflow-auto", actual ? "" : "grid place-items-center p-4")}>
        <img
          src={image.src}
          alt={image.name}
          draggable={false}
          onLoad={(e) => setSize({ w: e.currentTarget.naturalWidth, h: e.currentTarget.naturalHeight })}
          onClick={() => setActual(!actual)}
          className={cn(actual ? "max-w-none cursor-zoom-out" : "max-h-full max-w-full cursor-zoom-in object-contain")}
        />
      </div>
    </div>
  )
}

function IconButton({ label, onClick, children }: { label: string; onClick: () => void; children: React.ReactNode }) {
  return (
    <Tip content={label}>
      <button className="rounded-md grid size-7 place-items-center hover:bg-accent hover:text-foreground [&_svg]:size-3.5" aria-label={label} onClick={onClick}>
        {children}
      </button>
    </Tip>
  )
}

async function blobOf(image: ShownImage): Promise<Blob> {
  return (await fetch(image.src)).blob()
}

/** Copies the picture itself, as PNG, through the clipboard plugin (the webview's own clipboard
 *  can refuse images). */
async function copyImage(image: ShownImage) {
  try {
    const blob = await blobOf(image)
    const png = blob.type === "image/png" ? blob : await toPng(blob)
    const { writeImage } = await import("@tauri-apps/plugin-clipboard-manager")
    await writeImage(new Uint8Array(await png.arrayBuffer()))
    toast.success("Image copied")
  } catch (e) {
    toast.error(`Couldn't copy the image: ${e}`)
  }
}

async function toPng(blob: Blob): Promise<Blob> {
  const bmp = await createImageBitmap(blob)
  const canvas = document.createElement("canvas")
  canvas.width = bmp.width
  canvas.height = bmp.height
  canvas.getContext("2d")!.drawImage(bmp, 0, 0)
  return new Promise((resolve, reject) => canvas.toBlob((b) => (b ? resolve(b) : reject(new Error("no PNG"))), "image/png"))
}

/** Saves through the system's Save dialog. */
async function saveImage(image: ShownImage) {
  try {
    const { save } = await import("@tauri-apps/plugin-dialog")
    const path = await save({ defaultPath: image.name, title: "Save image" })
    if (!path) return
    const bytes = new Uint8Array(await (await blobOf(image)).arrayBuffer())
    const { invoke } = await import("@tauri-apps/api/core")
    await invoke("save_bytes", { path, data: Array.from(bytes) })
    toast.success("Image saved")
  } catch (e) {
    toast.error(`Couldn't save the image: ${e}`)
  }
}

// ---- The full-window viewer ---------------------------------------------------------------------

const useLightbox = create<{ images: ShownImage[]; index: number }>(() => ({ images: [], index: 0 }))

/** Shows `images` full-window, starting at `index`: ← → between them, Esc closes. */
export function openImages(images: ShownImage[], index = 0) {
  if (images.length) useLightbox.setState({ images, index })
}

export function ImageLightbox() {
  const { images, index } = useLightbox()
  const close = () => useLightbox.setState({ images: [], index: 0 })
  const go = (d: number) => useLightbox.setState({ index: (index + d + images.length) % images.length })
  useEffect(() => {
    if (!images.length) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close()
      else if (e.key === "ArrowLeft" && images.length > 1) go(-1)
      else if (e.key === "ArrowRight" && images.length > 1) go(1)
      else return
      e.preventDefault()
      e.stopPropagation()
    }
    window.addEventListener("keydown", onKey, true)
    return () => window.removeEventListener("keydown", onKey, true)
  })
  if (!images.length) return null
  const image = images[Math.min(index, images.length - 1)]
  return (
    <div className="fixed inset-0 z-50 flex flex-col bg-background/95 backdrop-blur-sm" role="dialog" aria-label={image.name} onClick={(e) => e.target === e.currentTarget && close()}>
      <div className="flex h-10 shrink-0 items-center gap-2 border-b px-3 text-[12px]">
        <span className="text-muted-foreground">{images.length > 1 ? `${index + 1} of ${images.length}` : "Image"}</span>
        <button className="rounded-md ml-auto grid size-7 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Close" onClick={close}>
          <X className="size-4" />
        </button>
      </div>
      <div className="relative flex min-h-0 flex-1">
        <ImageStage image={image} className="flex-1" />
        {images.length > 1 ? (
          <>
            <button className="rounded-md absolute top-1/2 left-3 grid size-9 -translate-y-1/2 place-items-center border bg-card/90 hover:bg-accent" aria-label="Previous image" onClick={() => go(-1)}>
              <ChevronLeft className="size-4" />
            </button>
            <button className="rounded-md absolute top-1/2 right-3 grid size-9 -translate-y-1/2 place-items-center border bg-card/90 hover:bg-accent" aria-label="Next image" onClick={() => go(1)}>
              <ChevronRight className="size-4" />
            </button>
          </>
        ) : null}
      </div>
      {images.length > 1 ? (
        <div className="flex shrink-0 justify-center gap-1.5 border-t p-2">
          {images.map((im, i) => (
            <button key={i} className={cn("rounded-sm size-12 overflow-hidden border", i === index ? "border-primary" : "opacity-60 hover:opacity-100")} onClick={() => useLightbox.setState({ index: i })} aria-label={im.name}>
              <img src={im.src} alt="" className="size-full object-cover" />
            </button>
          ))}
        </div>
      ) : null}
    </div>
  )
}
