// Zooming and panning an image in a box: where it sits (`x`, `y`: its top-left corner, in the
// box's pixels) and how big (`s`: its scale). It never zooms out past fitting the box, and never
// pans so far that empty space shows beside a side that's bigger than the box; a side smaller
// than the box stays centred.

export type View = { s: number; x: number; y: number }
export type Size = { w: number; h: number }

/** The most it zooms in: ten times its actual size. */
export const MAX_SCALE = 10

/** The scale that fits `img` in `box` with `pad` around it, never bigger than actual size. */
export function fitScale(img: Size, box: Size, pad = 16): number {
  if (!img.w || !img.h) return 1
  return Math.min(1, Math.max(0.01, (box.w - pad * 2) / img.w), Math.max(0.01, (box.h - pad * 2) / img.h))
}

/** The image fitted and centred. */
export function fitView(img: Size, box: Size, pad = 16): View {
  return clampView({ s: fitScale(img, box, pad), x: 0, y: 0 }, img, box)
}

/** Keeps the image covering the box on each side it's bigger than, centred on the others. */
export function clampView(v: View, img: Size, box: Size): View {
  const axis = (pos: number, size: number, room: number) => (size <= room ? (room - size) / 2 : Math.min(0, Math.max(room - size, pos)))
  return { s: v.s, x: axis(v.x, img.w * v.s, box.w), y: axis(v.y, img.h * v.s, box.h) }
}

/** Zooms by `factor` keeping the image's point under (`px`, `py`) where it is. */
export function zoomAt(v: View, factor: number, px: number, py: number, img: Size, box: Size, pad = 16): View {
  const s = Math.min(MAX_SCALE, Math.max(fitScale(img, box, pad), v.s * factor))
  const k = s / v.s
  return clampView({ s, x: px - (px - v.x) * k, y: py - (py - v.y) * k }, img, box)
}

/** Moves the image by (`dx`, `dy`). */
export function panBy(v: View, dx: number, dy: number, img: Size, box: Size): View {
  return clampView({ s: v.s, x: v.x + dx, y: v.y + dy }, img, box)
}

/** Fitted: zoom in on (`px`, `py`) (to actual size, or twice the fit if that's barely bigger).
 *  Zoomed in: back to fit. */
export function toggleAt(v: View, px: number, py: number, img: Size, box: Size, pad = 16): View {
  const fit = fitScale(img, box, pad)
  if (v.s > fit * 1.01) return fitView(img, box, pad)
  const target = Math.min(MAX_SCALE, Math.max(1, fit * 2))
  return zoomAt(v, target / v.s, px, py, img, box, pad)
}

/** How much a wheel turn zooms: a trackpad pinch (Ctrl + wheel) is fine-grained, a mouse wheel
 *  notch about 20%. */
export function wheelFactor(deltaY: number, deltaMode: number, pinch: boolean): number {
  const px = deltaMode === 1 ? deltaY * 16 : deltaMode === 2 ? deltaY * 400 : deltaY
  return Math.exp(-px * (pinch ? 0.01 : 0.0015))
}
