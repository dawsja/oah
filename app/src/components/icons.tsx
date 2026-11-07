// The oah mark (assets/svg/oah-mark-*.svg) and the window controls. Everything else is lucide.

export function Mark({ size = 20, label, className }: { size?: number; label?: string; className?: string }) {
  return (
    <svg viewBox="10 10 100 100" width={size} height={size} className={className} role={label ? "img" : undefined} aria-label={label} aria-hidden={label ? undefined : true}>
      <path d="M97.95 58.14 A38 38 0 1 1 61.86 22.05" fill="none" stroke="var(--oah-brand)" strokeWidth="14" strokeLinecap="round" />
      <circle cx="86.87" cy="33.13" r="9" fill="var(--primary)" />
    </svg>
  )
}

export const WinMin = () => (
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0 5h10" stroke="currentColor" strokeWidth="1" /></svg>
)
export const WinMax = () => (
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" strokeWidth="1" /></svg>
)
export const WinClose = () => (
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0 0l10 10M10 0L0 10" stroke="currentColor" strokeWidth="1" /></svg>
)

/** Claude Code and Codex marks, simplified to flat glyphs. */
export function AgentGlyph({ agent, className }: { agent: "claude" | "codex"; className?: string }) {
  if (agent === "claude")
    return (
      <svg viewBox="0 0 16 16" className={className} aria-hidden="true">
        <path fill="#d97757" d="M8 1l1.3 4.2L13.5 3 10.8 6.7 15 8l-4.2 1.3 2.7 3.7-4.2-2.2L8 15l-1.3-4.2L2.5 13l2.7-3.7L1 8l4.2-1.3L2.5 3l4.2 2.2z" />
      </svg>
    )
  return (
    <svg viewBox="0 0 16 16" className={className} aria-hidden="true">
      <rect x="1.5" y="1.5" width="13" height="13" fill="none" stroke="currentColor" strokeWidth="1.5" />
      <path d="M4.5 6l2 2-2 2M8 10.5h3.5" fill="none" stroke="currentColor" strokeWidth="1.5" />
    </svg>
  )
}
