// The oah mark (assets/svg/oah-mark-*.svg) and the window controls. Everything else is lucide.

import { useId } from "react"
import type { AgentKind } from "@/lib/types"

export function Mark({ size = 20, label, className }: { size?: number; label?: string; className?: string }) {
  return (
    <svg viewBox="10 10 100 100" width={size} height={size} className={className} role={label ? "img" : undefined} aria-label={label} aria-hidden={label ? undefined : true}>
      <path d="M97.95 58.14 A38 38 0 1 1 61.86 22.05" fill="none" stroke="var(--oah-brand)" strokeWidth="14" strokeLinecap="round" />
      <circle cx="86.87" cy="33.13" r="9" fill="var(--primary)" />
    </svg>
  )
}

/** The WSL penguin (assets/svg/wsl-logo.svg). Its 193x256 box is padded to a square so it
 *  scales like a lucide icon; the mark belongs to Microsoft, see THIRD_PARTY_NOTICES.md. */
export function WslLogo({ className }: { className?: string }) {
  return (
    <svg viewBox="-31.5 0 256 256" className={className} aria-hidden="true">
      <defs>
        <linearGradient id="wsl-logo-body" x1="20.89856" y1="28.70122" x2="160.51789" y2="270.52899" gradientUnits="userSpaceOnUse">
          <stop offset="0" stopColor="#0078d3" />
          <stop offset="1" stopColor="#0c5aa5" />
        </linearGradient>
        <radialGradient id="wsl-logo-face" cx="96.61321" cy="162.92136" fx="96.61321" fy="162.92136" r="40.58323" gradientTransform="translate(-28.32862 259.53456) rotate(-90) scale(1 .76688)" gradientUnits="userSpaceOnUse">
          <stop offset="0" stopColor="#c86508" />
          <stop offset="1" stopColor="#bf6009" />
        </radialGradient>
        <radialGradient id="wsl-logo-beak" cx="96.5" cy="154.76035" fx="96.5" fy="154.76035" r="40.36821" gradientTransform="translate(-2.6625 251.26035) rotate(-90) scale(1 .64075)" gradientUnits="userSpaceOnUse">
          <stop offset="0" stopColor="#ef8506" />
          <stop offset="1" stopColor="#ed7e03" />
        </radialGradient>
        <radialGradient id="wsl-logo-belly" cx="96.95112" cy="129.9307" fx="96.95112" fy="129.9307" r="103.07409" gradientTransform="translate(-11.25307 226.88182) rotate(-90) scale(1 .83279)" gradientUnits="userSpaceOnUse">
          <stop offset=".0784" stopColor="#ffc700" />
          <stop offset="1" stopColor="#ffc700" />
        </radialGradient>
        <filter id="wsl-logo-shadow" x="40.5" y="102" width="112" height="113" filterUnits="userSpaceOnUse">
          <feOffset dx="0" dy="7" />
          <feGaussianBlur result="blur" stdDeviation="8" />
          <feFlood floodColor="#000" floodOpacity=".4" />
          <feComposite in2="blur" operator="in" />
          <feComposite in="SourceGraphic" />
        </filter>
      </defs>
      <path fill="url(#wsl-logo-body)" d="M96.49981,0h.00019C149.7598,0,193,43.2402,193,96.5v149.5c0,5.51915-4.48085,10-10,10H10C4.48085,256,0,251.51915,0,246V96.49981C0,43.24011,43.24011,0,96.49981,0Z" />
      <circle fill="#fff" cx="56.5" cy="87.11" r="32" />
      <circle fill="#fff" cx="136.5" cy="87.11" r="32" />
      <path fill="#fff" d="M88.5,87.11H24.5v80.77687c0,35.23677,28.56505,63.80182,63.80182,63.80182h.19818V87.11Z" />
      <path fill="#fff" d="M104.5,87.11h64v80.77687c0,35.23677-28.56505,63.80182-63.80182,63.80182h-.19818V87.11Z" />
      <rect fill="#fff" x="75.96226" y="178.00943" width="40.75472" height="48.45283" />
      <circle cx="56.5" cy="96.10999" r="16.3019" transform="translate(-47.41926 136.48533) rotate(-80.78253)" />
      <circle cx="136.5" cy="96.11" r="16.30189" />
      <circle fill="url(#wsl-logo-face)" filter="url(#wsl-logo-shadow)" cx="96.5" cy="151.91" r="32" />
      <path fill="url(#wsl-logo-beak)" d="M64.5,149.48114c0-14.94339,13.16492-29.21021,32-29.37115,19.08282-.16306,32,16.88058,32,29.37115,0,0-11.54718,7.62885-32,7.62885s-32-7.62885-32-7.62885Z" />
      <path fill="url(#wsl-logo-belly)" d="M24.5,160.25473c0,19.28659-2.59474,71.85527,71.95556,71.85527s72.04444-50.83755,72.04444-72.35057c0,0-14.9004,53.93208-72.04444,53.93208s-71.95556-53.43678-71.95556-53.43678Z" />
    </svg>
  )
}

export const WinMin = () => (
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0 5h10" stroke="currentColor" strokeWidth="1" /></svg>
)
function AntigravityGlyph({ className }: { className?: string }) {
  // A gradient id of its own per glyph, usable in `url(#…)`.
  const id = `agy${useId().replace(/[^a-zA-Z0-9]/g, "")}`
  return (
    <svg viewBox="10 11 92 92" className={className} aria-hidden="true">
      <defs>
        <linearGradient id={id} x1="14" y1="0" x2="98" y2="0" gradientUnits="userSpaceOnUse">
          <stop offset="0" stopColor="#00B95C" />
          <stop offset="0.38" stopColor="#FBBC04" />
          <stop offset="0.6" stopColor="#FC413D" />
          <stop offset="1" stopColor="#3186FF" />
        </linearGradient>
      </defs>
      <path
        fill={`url(#${id})`}
        d="M89.6992 93.695C94.3659 97.195 101.366 94.8617 94.9492 88.445C75.6992 69.7783 79.7825 18.445 55.8659 18.445C31.9492 18.445 36.0325 69.7783 16.7825 88.445C9.78251 95.445 17.3658 97.195 22.0325 93.695C40.1159 81.445 38.9492 59.8617 55.8659 59.8617C72.7825 59.8617 71.6159 81.445 89.6992 93.695Z"
      />
    </svg>
  )
}

export const WinMax = () => (
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" strokeWidth="1" /></svg>
)
export const WinClose = () => (
  <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0 0l10 10M10 0L0 10" stroke="currentColor" strokeWidth="1" /></svg>
)

/** Agent marks, simplified to flat glyphs. */
/** Each agent's own mark: Claude's starburst, OpenAI's for Codex, OpenCode's (from
 *  opencode.ai's favicon), Antigravity's arch (from antigravity.google's favicon, its colours as
 *  one gradient). The marks belong to their owners; see THIRD_PARTY_NOTICES.md. */
export function AgentGlyph({ agent, className }: { agent: AgentKind; className?: string }) {
  if (agent === "antigravity") return <AntigravityGlyph className={className} />
  if (agent === "claude")
    return (
      <svg viewBox="0 0 24 24" className={className} aria-hidden="true">
        <path fill="#D97757" fillRule="nonzero" d="M4.709 15.955l4.72-2.647.08-.23-.08-.128H9.2l-.79-.048-2.698-.073-2.339-.097-2.266-.122-.571-.121L0 11.784l.055-.352.48-.321.686.06 1.52.103 2.278.158 1.652.097 2.449.255h.389l.055-.157-.134-.098-.103-.097-2.358-1.596-2.552-1.688-1.336-.972-.724-.491-.364-.462-.158-1.008.656-.722.881.06.225.061.893.686 1.908 1.476 2.491 1.833.365.304.145-.103.019-.073-.164-.274-1.355-2.446-1.446-2.49-.644-1.032-.17-.619a2.97 2.97 0 01-.104-.729L6.283.134 6.696 0l.996.134.42.364.62 1.414 1.002 2.229 1.555 3.03.456.898.243.832.091.255h.158V9.01l.128-1.706.237-2.095.23-2.695.08-.76.376-.91.747-.492.584.28.48.685-.067.444-.286 1.851-.559 2.903-.364 1.942h.212l.243-.242.985-1.306 1.652-2.064.73-.82.85-.904.547-.431h1.033l.76 1.129-.34 1.166-1.064 1.347-.881 1.142-1.264 1.7-.79 1.36.073.11.188-.02 2.856-.606 1.543-.28 1.841-.315.833.388.091.395-.328.807-1.969.486-2.309.462-3.439.813-.042.03.049.061 1.549.146.662.036h1.622l3.02.225.79.522.474.638-.079.485-1.215.62-1.64-.389-3.829-.91-1.312-.329h-.182v.11l1.093 1.068 2.006 1.81 2.509 2.33.127.578-.322.455-.34-.049-2.205-1.657-.851-.747-1.926-1.62h-.128v.17l.444.649 2.345 3.521.122 1.08-.17.353-.608.213-.668-.122-1.374-1.925-1.415-2.167-1.143-1.943-.14.08-.674 7.254-.316.37-.729.28-.607-.461-.322-.747.322-1.476.389-1.924.315-1.53.286-1.9.17-.632-.012-.042-.14.018-1.434 1.967-2.18 2.945-1.726 1.845-.414.164-.717-.37.067-.662.401-.589 2.388-3.036 1.44-1.882.93-1.086-.006-.158h-.055L4.132 18.56l-1.13.146-.487-.456.061-.746.231-.243 1.908-1.312-.006.006z" />
      </svg>
    )
  if (agent === "codex")
    return (
      <svg viewBox="0 0 24 24" className={className} aria-hidden="true">
        <path fill="currentColor" fillRule="evenodd" d="M9.205 8.658v-2.26c0-.19.072-.333.238-.428l4.543-2.616c.619-.357 1.356-.523 2.117-.523 2.854 0 4.662 2.212 4.662 4.566 0 .167 0 .357-.024.547l-4.71-2.759a.797.797 0 00-.856 0l-5.97 3.473zm10.609 8.8V12.06c0-.333-.143-.57-.429-.737l-5.97-3.473 1.95-1.118a.433.433 0 01.476 0l4.543 2.617c1.309.76 2.189 2.378 2.189 3.948 0 1.808-1.07 3.473-2.76 4.163zM7.802 12.703l-1.95-1.142c-.167-.095-.239-.238-.239-.428V5.899c0-2.545 1.95-4.472 4.591-4.472 1 0 1.927.333 2.712.928L8.23 5.067c-.285.166-.428.404-.428.737v6.898zM12 15.128l-2.795-1.57v-3.33L12 8.658l2.795 1.57v3.33L12 15.128zm1.796 7.23c-1 0-1.927-.332-2.712-.927l4.686-2.712c.285-.166.428-.404.428-.737v-6.898l1.974 1.142c.167.095.238.238.238.428v5.233c0 2.545-1.974 4.472-4.614 4.472zm-5.637-5.303l-4.544-2.617c-1.308-.761-2.188-2.378-2.188-3.948A4.482 4.482 0 014.21 6.327v5.423c0 .333.143.571.428.738l5.947 3.449-1.95 1.118a.432.432 0 01-.476 0zm-.262 3.9c-2.688 0-4.662-2.021-4.662-4.519 0-.19.024-.38.047-.57l4.686 2.71c.286.167.571.167.856 0l5.97-3.448v2.26c0 .19-.07.333-.237.428l-4.543 2.616c-.619.357-1.356.523-2.117.523zm5.899 2.83a5.947 5.947 0 005.827-4.756C22.287 18.339 24 15.84 24 13.296c0-1.665-.713-3.282-1.998-4.448.119-.5.19-.999.19-1.498 0-3.401-2.759-5.947-5.946-5.947-.642 0-1.26.095-1.88.31A5.962 5.962 0 0010.205 0a5.947 5.947 0 00-5.827 4.757C1.713 5.447 0 7.945 0 10.49c0 1.666.713 3.283 1.998 4.448-.119.5-.19 1-.19 1.499 0 3.401 2.759 5.946 5.946 5.946.642 0 1.26-.095 1.88-.309a5.96 5.96 0 004.162 1.713z" />
      </svg>
    )
  return (
    <svg viewBox="88 88 336 336" className={className} aria-hidden="true">
      <path d="M320 224V352H192V224H320Z" fill="currentColor" fillOpacity="0.28" />
      <path fillRule="evenodd" clipRule="evenodd" d="M384 416H128V96H384V416ZM320 160H192V352H320V160Z" fill="currentColor" />
    </svg>
  )
}
