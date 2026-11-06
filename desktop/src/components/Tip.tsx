// A shadcn tooltip around one element: the app's only kind of tooltip (no native
// `title` tooltips).
import type { ReactElement, ReactNode } from "react"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"

export function Tip({ content, side, children }: { content: ReactNode; side?: "top" | "right" | "bottom" | "left"; children: ReactElement }) {
  if (content === null || content === undefined || content === "") return children
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent side={side} className="max-w-sm break-words">
        {content}
      </TooltipContent>
    </Tooltip>
  )
}
