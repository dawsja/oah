import { Component, createContext, useCallback, useContext, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react"
import { ChevronLeft, ChevronRight, Copy, Download, Maximize2, Minimize2, X, ZoomIn, ZoomOut } from "lucide-react"
import { create } from "zustand"
import { toast } from "sonner"
import { Tip } from "@/components/Tip"
import { cn } from "@/lib/utils"
import { copyImageLocally, saveImageLocally } from "@/lib/image-download"
import { clampView, fitScale, fitView, panBy, toggleAt, wheelFactor, zoomAt, type Size, type View } from "@/lib/zoom"

export const ImageActionsContext = createContext({ copy: copyImageLocally, save: saveImageLocally })

/** An image to show: a `data:` URL (or any URL the webview can load) and a name. */
export type ShownImage = { src: string; name: string }

/** Room around a fitted image. */
const PAD = 16
/** How far a pointer moves before a press is a drag, not a click. */
const SLOP = 4

/** The image on a checkerboard, fitted to the space, to zoom and look around: the wheel or a
 *  pinch zooms where it points, dragging moves it, a click (a double tap) zooms in on that spot
 *  or back out. `keyboard`: + − 0 1 zoom too (the full-window viewer). Its size, the zoom, and
 *  Copy / Save are above it. */
export function ImageStage({ image, className, toolbar = true, keyboard = false }: { image: ShownImage; className?: string; toolbar?: boolean; keyboard?: boolean }) {
  const actions = useContext(ImageActionsContext)
  const boxRef = useRef<HTMLDivElement>(null)
  const [natural, setNatural] = useState<Size | null>(null)
  const [box, setBox] = useState<Size>({ w: 0, h: 0 })
  const [view, setView] = useState<View | null>(null)
  const [dragging, setDragging] = useState(false)
  // What gestures read between renders.
  const sizes = useRef({ natural, box })
  sizes.current = { natural, box }
  /** Fitted (not zoomed by hand): it fits again when the space changes. */
  const fitted = useRef(true)

  useEffect(() => {
    setNatural(null)
    setView(null)
    fitted.current = true
  }, [image.src])

  useLayoutEffect(() => {
    const el = boxRef.current
    if (!el) return
    const measure = () => setBox({ w: el.clientWidth, h: el.clientHeight })
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  // Fit once the image and the space are known; when the space changes, fit again (or keep the
  // zoom and stay in bounds).
  useEffect(() => {
    if (!natural || !box.w || !box.h) return
    setView((v) => (!v || fitted.current ? fitView(natural, box, PAD) : clampView({ ...v, s: Math.max(v.s, fitScale(natural, box, PAD)) }, natural, box)))
  }, [natural, box])

  const update = useCallback((edit: (v: View, img: Size, box: Size) => View) => {
    const { natural: img, box: b } = sizes.current
    if (!img || !b.w) return
    setView((v) => {
      if (!v) return v
      const next = edit(v, img, b)
      fitted.current = next.s <= fitScale(img, b, PAD) * 1.001
      return next
    })
  }, [])
  const zoomBy = useCallback((factor: number) => update((v, img, b) => zoomAt(v, factor, b.w / 2, b.h / 2, img, b, PAD)), [update])
  const fit = useCallback(() => update((_, img, b) => fitView(img, b, PAD)), [update])
  const actual = useCallback(() => update((v, img, b) => zoomAt(v, 1 / v.s, b.w / 2, b.h / 2, img, b, PAD)), [update])

  // The wheel zooms where it points (not passive: the page mustn't scroll).
  useEffect(() => {
    const el = boxRef.current
    if (!el) return
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      const r = el.getBoundingClientRect()
      const f = wheelFactor(e.deltaY, e.deltaMode, e.ctrlKey)
      update((v, img, b) => zoomAt(v, f, e.clientX - r.left, e.clientY - r.top, img, b, PAD))
    }
    el.addEventListener("wheel", onWheel, { passive: false })
    return () => el.removeEventListener("wheel", onWheel)
  }, [update])

  useEffect(() => {
    if (!keyboard) return
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null
      if (e.ctrlKey || e.metaKey || e.altKey || t?.closest("input, textarea, [contenteditable=true]")) return
      if (e.key === "+" || e.key === "=") zoomBy(1.5)
      else if (e.key === "-" || e.key === "_") zoomBy(1 / 1.5)
      else if (e.key === "0") fit()
      else if (e.key === "1") actual()
      else return
      e.preventDefault()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [keyboard, zoomBy, fit, actual])

  // Pointers: one drags, two pinch; a press that doesn't move is a click (a tap).
  const pointers = useRef(new Map<number, { x: number; y: number }>())
  const press = useRef<{ x: number; y: number; moved: boolean; pinch: { d: number; mx: number; my: number } | null }>({ x: 0, y: 0, moved: false, pinch: null })
  const lastTap = useRef<{ t: number; x: number; y: number } | null>(null)
  const local = (e: React.PointerEvent) => {
    const r = boxRef.current!.getBoundingClientRect()
    return { x: e.clientX - r.left, y: e.clientY - r.top }
  }
  const pinchOf = () => {
    const [a, b] = [...pointers.current.values()]
    return { d: Math.hypot(a.x - b.x, a.y - b.y) || 1, mx: (a.x + b.x) / 2, my: (a.y + b.y) / 2 }
  }
  const onPointerDown = (e: React.PointerEvent) => {
    if (e.pointerType === "mouse" && e.button !== 0) return
    e.currentTarget.setPointerCapture(e.pointerId)
    const p = local(e)
    pointers.current.set(e.pointerId, p)
    if (pointers.current.size === 1) press.current = { x: p.x, y: p.y, moved: false, pinch: null }
    else if (pointers.current.size === 2) press.current = { ...press.current, moved: true, pinch: pinchOf() }
  }
  const onPointerMove = (e: React.PointerEvent) => {
    const before = pointers.current.get(e.pointerId)
    if (!before) return
    const p = local(e)
    pointers.current.set(e.pointerId, p)
    const g = press.current
    if (pointers.current.size === 1) {
      if (!g.moved && Math.hypot(p.x - g.x, p.y - g.y) > SLOP) {
        g.moved = true
        setDragging(true)
      }
      if (g.moved) update((v, img, b) => panBy(v, p.x - before.x, p.y - before.y, img, b))
    } else if (pointers.current.size === 2 && g.pinch) {
      const now = pinchOf()
      const was = g.pinch
      update((v, img, b) => panBy(zoomAt(v, now.d / was.d, now.mx, now.my, img, b, PAD), now.mx - was.mx, now.my - was.my, img, b))
      g.pinch = now
    }
  }
  const onPointerUp = (e: React.PointerEvent) => {
    const p = pointers.current.get(e.pointerId)
    if (!p) return
    pointers.current.delete(e.pointerId)
    const g = press.current
    if (pointers.current.size === 1) {
      // One finger left after a pinch: it drags on from here, and isn't a tap.
      const [rest] = [...pointers.current.values()]
      press.current = { x: rest.x, y: rest.y, moved: true, pinch: null }
      return
    }
    if (pointers.current.size) return
    setDragging(false)
    if (g.moved) return
    if (e.pointerType === "mouse") {
      update((v, img, b) => toggleAt(v, p.x, p.y, img, b, PAD))
      return
    }
    const t = lastTap.current
    if (t && e.timeStamp - t.t < 320 && Math.hypot(p.x - t.x, p.y - t.y) < 30) {
      lastTap.current = null
      update((v, img, b) => toggleAt(v, p.x, p.y, img, b, PAD))
    } else {
      lastTap.current = { t: e.timeStamp, x: p.x, y: p.y }
    }
  }
  const onPointerCancel = (e: React.PointerEvent) => {
    pointers.current.delete(e.pointerId)
    if (!pointers.current.size) setDragging(false)
  }

  const fitS = natural && box.w ? fitScale(natural, box, PAD) : 1
  const zoomed = !!view && view.s > fitS * 1.001
  const percent = view ? Math.round(view.s * 100) : null
  return (
    <div className={cn("flex min-h-0 flex-col", className)}>
      {toolbar ? (
        <div className="flex h-9 [@media(pointer:coarse)]:h-12 shrink-0 items-center gap-1 px-3 text-[11.5px] text-muted-foreground">
          <span className="min-w-0 truncate font-mono">{image.name}</span>
          {/* Phones pinch to zoom: their toolbar keeps the zoom, not the buttons or the size. */}
          {natural ? <span className="shrink-0 font-mono text-faint [@media(pointer:coarse)]:hidden">· {natural.w}×{natural.h}</span> : null}
          <span className="ml-auto flex shrink-0 items-center gap-0.5">
            <IconButton label="Zoom out (−)" onClick={() => zoomBy(1 / 1.5)} disabled={!zoomed} className="[@media(pointer:coarse)]:hidden">
              <ZoomOut />
            </IconButton>
            <Tip content="Fit (0)">
              <button className="rounded-md h-7 [@media(pointer:coarse)]:h-11 min-w-12 px-1 font-mono text-[11px] hover:bg-accent hover:text-foreground" aria-label="Fit to window" onClick={fit}>
                {percent === null ? "…" : `${percent}%`}
              </button>
            </Tip>
            <IconButton label="Zoom in (+)" onClick={() => zoomBy(1.5)} disabled={!!view && view.s >= 10} className="[@media(pointer:coarse)]:hidden">
              <ZoomIn />
            </IconButton>
            {zoomed || fitS < 1 ? (
              <IconButton label={zoomed ? "Fit to window" : "Actual size"} onClick={zoomed ? fit : actual}>
                {zoomed ? <Minimize2 /> : <Maximize2 />}
              </IconButton>
            ) : null}
            <IconButton label="Copy image" onClick={() => void copyImage(image, actions.copy)}>
              <Copy />
            </IconButton>
            <IconButton label="Save as…" onClick={() => void saveImage(image, actions.save)}>
              <Download />
            </IconButton>
          </span>
        </div>
      ) : null}
      <div
        ref={boxRef}
        className="checkerboard relative min-h-0 flex-1 overflow-hidden select-none"
        style={{ touchAction: "none", cursor: dragging ? "grabbing" : zoomed ? "grab" : "zoom-in" }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerCancel}
      >
        <img
          src={image.src}
          alt={image.name}
          draggable={false}
          onLoad={(e) => setNatural({ w: e.currentTarget.naturalWidth, h: e.currentTarget.naturalHeight })}
          className="pointer-events-none absolute top-0 left-0 max-w-none origin-top-left"
          style={{
            width: natural?.w,
            height: natural?.h,
            opacity: view ? 1 : 0,
            transform: view ? `translate3d(${view.x}px, ${view.y}px, 0) scale(${view.s})` : undefined,
            // Pixels stay crisp when they're big (screenshots), smooth otherwise.
            imageRendering: view && view.s >= 2 ? "pixelated" : "auto",
          }}
        />
      </div>
    </div>
  )
}

function IconButton({ label, onClick, disabled, className, children }: { label: string; onClick: () => void; disabled?: boolean; className?: string; children: React.ReactNode }) {
  return (
    <Tip content={label}>
      <button
        className={cn(
          "rounded-md grid size-7 [@media(pointer:coarse)]:size-11 place-items-center hover:bg-accent hover:text-foreground disabled:opacity-40 disabled:hover:bg-transparent [&_svg]:size-3.5",
          className,
        )}
        aria-label={label}
        onClick={onClick}
        disabled={disabled}
      >
        {children}
      </button>
    </Tip>
  )
}

/** Copies the picture itself, as PNG, through the clipboard plugin (the webview's own clipboard
 *  can refuse images). */
async function copyImage(image: ShownImage, copy: typeof copyImageLocally) {
  try {
    await copy(image.src)
    toast.success("Image copied")
  } catch (e) {
    toast.error(`Couldn't copy the image: ${e}`)
  }
}

/** Saves through the system's Save dialog. */
async function saveImage(image: ShownImage, save: typeof saveImageLocally) {
  try {
    if (await save(image)) toast.success("Image saved")
  } catch (e) {
    toast.error(`Couldn't save the image: ${e}`)
  }
}

// ---- The full-window viewer ---------------------------------------------------------------------

const useLightbox = create<{ images: ShownImage[]; index: number }>(() => ({ images: [], index: 0 }))

/** Shows `images` full-window, starting at `index`: ← → between them, + − 0 1 zoom, Esc
 *  closes. */
export function openImages(images: ShownImage[], index = 0) {
  if (images.length) useLightbox.setState({ images, index })
}

/** The viewer, closed with a message if showing an image fails, instead of taking the whole
 *  window down with it. */
/** The image viewer. `inWindow` (the desktop): a panel over the main area, under the title bar,
 *  with the window still showing around it; otherwise (the phone) the whole screen. */
export function ImageLightbox({ inWindow = false }: { inWindow?: boolean }) {
  return (
    <LightboxBoundary>
      <Lightbox inWindow={inWindow} />
    </LightboxBoundary>
  )
}

class LightboxBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false }
  static getDerivedStateFromError() {
    return { failed: true }
  }
  componentDidCatch(error: unknown) {
    useLightbox.setState({ images: [], index: 0 })
    toast.error(`Couldn't show the image: ${error instanceof Error ? error.message : String(error)}`)
    this.setState({ failed: false })
  }
  render() {
    return this.state.failed ? null : this.props.children
  }
}

function Lightbox({ inWindow }: { inWindow: boolean }) {
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
  const viewer = (
    <div
      className={cn(
        "flex flex-col",
        inWindow ? "h-[min(85%,56rem)] w-[min(85%,80rem)] overflow-hidden rounded-lg border bg-background shadow-2xl" : "safe fixed inset-0 z-50 bg-background/95 backdrop-blur-sm",
      )}
      role="dialog"
      aria-label={image.name}
      onClick={(e) => !inWindow && e.target === e.currentTarget && close()}
    >
      <div className="flex h-10 [@media(pointer:coarse)]:h-12 shrink-0 items-center gap-2 border-b px-3 text-[12px]">
        <span className="text-muted-foreground">{images.length > 1 ? `${index + 1} of ${images.length}` : "Image"}</span>
        <button className="rounded-md ml-auto grid size-7 [@media(pointer:coarse)]:size-11 place-items-center text-muted-foreground hover:bg-accent hover:text-foreground" aria-label="Close" onClick={close}>
          <X className="size-4" />
        </button>
      </div>
      <div className="relative flex min-h-0 flex-1">
        <ImageStage image={image} className="flex-1" keyboard />
        {images.length > 1 ? (
          <>
            <button className="rounded-full absolute top-1/2 left-3 grid size-9 -translate-y-1/2 place-items-center border bg-card/90 hover:bg-accent" aria-label="Previous image" onClick={() => go(-1)}>
              <ChevronLeft className="size-4" />
            </button>
            <button className="rounded-full absolute top-1/2 right-3 grid size-9 -translate-y-1/2 place-items-center border bg-card/90 hover:bg-accent" aria-label="Next image" onClick={() => go(1)}>
              <ChevronRight className="size-4" />
            </button>
          </>
        ) : null}
      </div>
      {images.length > 1 ? (
        <div className="flex shrink-0 justify-center gap-1.5 border-t p-2">
          {images.map((im, i) => (
            <button key={i} className={cn("rounded-md size-12 overflow-hidden border", i === index ? "border-primary" : "opacity-60 hover:opacity-100")} onClick={() => useLightbox.setState({ index: i })} aria-label={im.name}>
              <img src={im.src} alt="" className="size-full object-cover" />
            </button>
          ))}
        </div>
      ) : null}
    </div>
  )
  if (!inWindow) return viewer
  // Below the title bar (its window buttons still work), over a dimmed window: a click outside
  // the panel closes it.
  return (
    <div className="fixed inset-x-0 top-9 bottom-0 z-50 grid place-items-center bg-black/50 p-4" onClick={(e) => e.target === e.currentTarget && close()}>
      {viewer}
    </div>
  )
}
