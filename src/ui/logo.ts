// The oah mark — an open ring (the "o") with the agent node in its gap — drawn
// from its geometry (assets/svg/oah-mark-*.svg, viewBox 10 10 100 100) with
// half blocks: each cell is two near-square pixels ("▀": fg = top, bg = bottom).
// Pixels are solid (≥ half covered) or empty — no blending, so edges stay crisp.

export interface Cell {
  ch: string
  fg?: string
  bg?: string
}

const C = 60 // ring centre
const R = 38 // ring radius
const HALF = 7 // half the stroke width
const NODE = { x: 86.87, y: 33.13, r: 9 }
// The arc runs clockwise (y down) from its right end to its top end; the gap is the top-right quadrant.
const A0 = Math.atan2(58.14 - C, 97.95 - C)
const A1 = Math.atan2(22.05 - C, 61.86 - C)
const ENDS = [
  [97.95, 58.14],
  [61.86, 22.05],
] as const

function inRing(x: number, y: number): boolean {
  const a = Math.atan2(y - C, x - C)
  if (!(a > A1 && a < A0)) return Math.abs(Math.hypot(x - C, y - C) - R) <= HALF
  return ENDS.some(([ex, ey]) => Math.hypot(x - ex, y - ey) <= HALF) // round caps
}
const inNode = (x: number, y: number) => Math.hypot(x - NODE.x, y - NODE.y) <= NODE.r

type Px = "ring" | "node" | null

/** The mark as `rows` lines of `2 * rows` cells (pixels are square: two per cell, stacked). */
export function markCells(rows: number, colors: { ring: string; node: string }): Cell[][] {
  const size = rows * 2
  const S = 6
  const pixel = (px: number, py: number): Px => {
    let ring = 0
    let node = 0
    for (let sy = 0; sy < S; sy++)
      for (let sx = 0; sx < S; sx++) {
        const x = 10 + ((px + (sx + 0.5) / S) / size) * 100
        const y = 10 + ((py + (sy + 0.5) / S) / size) * 100
        if (inNode(x, y)) node++
        else if (inRing(x, y)) ring++
      }
    const half = (S * S) / 2
    return node >= half ? "node" : ring >= half ? "ring" : node + ring >= half ? (node > ring ? "node" : "ring") : null
  }
  const color = (p: Px) => (p ? colors[p] : undefined)
  const out: Cell[][] = []
  for (let row = 0; row < rows; row++) {
    const line: Cell[] = []
    for (let col = 0; col < size; col++) {
      const top = pixel(col, row * 2)
      const bottom = pixel(col, row * 2 + 1)
      if (!top && !bottom) line.push({ ch: " " })
      else if (top === bottom) line.push({ ch: "█", fg: color(top) })
      else if (!bottom) line.push({ ch: "▀", fg: color(top) })
      else if (!top) line.push({ ch: "▄", fg: color(bottom) })
      else line.push({ ch: "▀", fg: color(top), bg: color(bottom) })
    }
    out.push(line)
  }
  return out
}
