export const t = {
  bg: "#0e0e0f",
  panel: "#161617",
  panelHi: "#1f1f21",
  userBg: "#1c1c1f",
  border: "#2c2c2f",
  borderHi: "#424246",
  text: "#ececec",
  dim: "#9a9a9f",
  faint: "#5c5c62",
  accent: "#ffd43b",
  accentDeep: "#e0a800",
  cyan: "#6cc7ff",
  green: "#63d297",
  red: "#ff6b7d",
  yellow: "#ffa94d",
  orange: "#ff8a4c",
  code: "#ffe08a",
  addFg: "#63d297",
  addBg: "#14261d",
  delFg: "#ff6b7d",
  delBg: "#2b161b",
  plan: "#6cc7ff",
  build: "#ffd43b",
  you: "#ffd43b",
} as const

export const SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]

export function fmtTokens(n: number): string {
  if (n >= 999_500) {
    const m = n / 1e6
    return `${m >= 10 ? Math.round(m) : +m.toFixed(1)}M` // 1048576 → 1M, 1.5M, 2M
  }
  if (n >= 10_000) return `${Math.round(n / 1000)}k`
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`
  return String(n)
}

export function fmtMs(ms: number): string {
  const s = Math.round(ms / 1000)
  return s >= 60 ? `${Math.floor(s / 60)}m${String(s % 60).padStart(2, "0")}s` : `${s}s`
}

/** Linear blend of two #rrggbb colors. */
export function mix(a: string, b: string, k: number): string {
  const pa = [1, 3, 5].map((i) => parseInt(a.slice(i, i + 2), 16))
  const pb = [1, 3, 5].map((i) => parseInt(b.slice(i, i + 2), 16))
  return "#" + pa.map((x, i) => Math.round(x + (pb[i]! - x) * k).toString(16).padStart(2, "0")).join("")
}
