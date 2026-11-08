// A shadcn tooltip around one element: the app's only kind of tooltip (no native
// `title` tooltips).
import { useRef, useState, type ReactElement, type ReactNode } from "react"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"

export function Tip({ content, side, children }: { content: ReactNode; side?: "top" | "right" | "bottom" | "left"; children: ReactElement }) {
  const ref = useRef<HTMLButtonElement>(null)
  const [open, setOpen] = useState(false)
  if (content === null || content === undefined || content === "") return children
  // Radix also opens on any focus. A button keeps focus after it's clicked, so the window getting
  // focus back (after a title bar drag, say) refocused it and popped the tip with the pointer far
  // away. Open only while hovered, or focused from the keyboard.
  const onOpenChange = (next: boolean) => {
    const el = ref.current
    if (next && el && !el.matches(":hover") && !el.matches(":focus-visible")) return
    setOpen(next)
  }
  return (
    <Tooltip open={open} onOpenChange={onOpenChange}>
      <TooltipTrigger asChild ref={ref}>
        {children}
      </TooltipTrigger>
      <TooltipContent side={side} className="max-w-sm break-words">
        {content}
      </TooltipContent>
    </Tooltip>
  )
}
