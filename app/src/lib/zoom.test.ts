import { describe, expect, it } from "vitest"
import { MAX_SCALE, clampView, fitScale, fitView, panBy, toggleAt, wheelFactor, zoomAt } from "./zoom"

const img = { w: 2000, h: 1000 }
const box = { w: 1032, h: 832 } // 1000 × 800 inside the 16px padding

describe("zooming an image", () => {
  it("fits, centred, never bigger than actual size", () => {
    expect(fitScale(img, box)).toBe(0.5)
    expect(fitView(img, box)).toEqual({ s: 0.5, x: 16, y: (832 - 500) / 2 })
    // A small image sits at its actual size.
    expect(fitScale({ w: 100, h: 50 }, box)).toBe(1)
  })

  it("zooms around the point under the cursor", () => {
    const v = fitView(img, box)
    // The image pixel under (516, 416) is its centre, (1000, 500).
    const z = zoomAt(v, 2, 516, 416, img, box)
    expect(z.s).toBe(1)
    expect((516 - z.x) / z.s).toBeCloseTo(1000)
    expect((416 - z.y) / z.s).toBeCloseTo(500)
  })

  it("never zooms out past fitting, nor in past the maximum", () => {
    const v = fitView(img, box)
    expect(zoomAt(v, 0.1, 100, 100, img, box).s).toBe(0.5)
    expect(zoomAt(v, 1000, 100, 100, img, box).s).toBe(MAX_SCALE)
  })

  it("pans only as far as the image reaches", () => {
    const z = zoomAt(fitView(img, box), 4, 516, 416, img, box) // 4000 × 2000 shown
    const left = panBy(z, 10_000, 10_000, img, box)
    expect([left.x, left.y]).toEqual([0, 0])
    const right = panBy(z, -10_000, -10_000, img, box)
    expect([right.x, right.y]).toEqual([1032 - 4000, 832 - 2000])
    // A side smaller than the box stays centred, however it's dragged.
    const wide = clampView({ s: 0.6, x: 0, y: 500 }, img, box)
    expect(wide.y).toBe((832 - 600) / 2)
  })

  it("a click zooms in on the spot, and back out", () => {
    const v = fitView(img, box)
    const inside = toggleAt(v, 300, 400, img, box)
    expect(inside.s).toBe(1)
    expect((300 - inside.x) / inside.s).toBeCloseTo((300 - v.x) / v.s)
    expect(toggleAt(inside, 300, 400, img, box)).toEqual(v)
    // A small image (already actual size) zooms to twice that.
    const small = { w: 200, h: 100 }
    expect(toggleAt(fitView(small, box), 500, 400, small, box).s).toBe(2)
  })

  it("wheel notches and trackpad pinches zoom by sensible steps", () => {
    expect(wheelFactor(-100, 0, false)).toBeCloseTo(Math.exp(0.15))
    expect(wheelFactor(100, 0, false)).toBeLessThan(1)
    // Lines (Firefox) count as 16px each; a pinch is finer-grained per pixel but bigger per unit.
    expect(wheelFactor(-3, 1, false)).toBeCloseTo(wheelFactor(-48, 0, false))
    expect(wheelFactor(-10, 0, true)).toBeCloseTo(Math.exp(0.1))
  })
})
