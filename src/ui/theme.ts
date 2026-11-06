export const t = {
  bg: "#0f1117",
  panel: "#151823",
  panelHi: "#1d2131",
  userBg: "#171b2a",
  border: "#2a2f42",
  borderHi: "#3a4160",
  text: "#e6e8ef",
  dim: "#8a90a6",
  faint: "#545a72",
  accent: "#9d8cff",
  cyan: "#6cc7ff",
  green: "#63d297",
  red: "#ff6b7d",
  yellow: "#f2c46d",
  orange: "#ff9e64",
  code: "#f0a6ca",
  addFg: "#63d297",
  addBg: "#14261d",
  delFg: "#ff6b7d",
  delBg: "#2b161b",
  plan: "#f2c46d",
  build: "#6cc7ff",
  you: "#6cc7ff",
} as const

export const SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]

export function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1e6).toFixed(n >= 10e6 ? 0 : 1)}M`
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
