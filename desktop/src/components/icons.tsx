// The oah mark (assets/svg/oah-mark-on-dark.svg). Other icons come from lucide-react.

export function Mark({ size = 20, ring = "#E6EDF3", node = "#FFD43B", label, className }: { size?: number; ring?: string; node?: string; label?: string; className?: string }) {
  return (
    <svg viewBox="10 10 100 100" width={size} height={size} className={className} role={label ? "img" : undefined} aria-label={label} aria-hidden={label ? undefined : true}>
      <path d="M97.95 58.14 A38 38 0 1 1 61.86 22.05" fill="none" stroke={ring} strokeWidth="14" strokeLinecap="round" />
      <circle cx="86.87" cy="33.13" r="9" fill={node} />
    </svg>
  )
}

/** Braille spinner, as in the TUI. */
export const SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]

/** Window controls (Windows / Linux). */
export const WinMin = () => (
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0 5h10" stroke="currentColor" strokeWidth="1" /></svg>
)
export const WinMax = () => (
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" strokeWidth="1" /></svg>
)
export const WinClose = () => (
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0 0l10 10M10 0L0 10" stroke="currentColor" strokeWidth="1" /></svg>
)
