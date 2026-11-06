import { Check, Copy } from "lucide-react"
import { memo, useRef, useState, type ReactNode } from "react"
import ReactMarkdown, { type Components } from "react-markdown"
import rehypeHighlight from "rehype-highlight"
import remarkGfm from "remark-gfm"
import { Button } from "@/components/ui/button"
import { client } from "@/lib/client"
import { cn } from "@/lib/utils"

function Pre({ children }: { children?: ReactNode }) {
  const ref = useRef<HTMLPreElement>(null)
  const [copied, setCopied] = useState(false)
  return (
    <pre ref={ref} className="group/code">
      <Button
        variant="ghost"
        size="icon"
        className="absolute top-1.5 right-1.5 size-7 text-muted-foreground opacity-0 group-hover/code:opacity-100"
        aria-label="Copy code"
        onClick={() => {
          void client.t.copy(ref.current?.querySelector("code")?.innerText ?? "")
          setCopied(true)
          setTimeout(() => setCopied(false), 1200)
        }}
      >
        {copied ? <Check className="text-green" /> : <Copy />}
      </Button>
      {children}
    </pre>
  )
}

const components: Components = {
  pre: Pre,
  // Links open in the browser, never inside the app window.
  a: ({ href, children }) => (
    <a
      href={href}
      onClick={(e) => {
        e.preventDefault()
        if (href && /^https?:\/\//.test(href)) void client.t.openUrl(href)
      }}
    >
      {children}
    </a>
  ),
  img: ({ alt }) => <span className="text-muted-foreground">[{alt || "image"}]</span>,
}

/** The model's markdown. Raw HTML is not rendered (react-markdown's default). */
export const Markdown = memo(function Markdown({ text, className }: { text: string; className?: string }) {
  return (
    <div className={cn("md", className)}>
      <ReactMarkdown remarkPlugins={[remarkGfm]} rehypePlugins={[[rehypeHighlight, { detect: false, ignoreMissing: true }]]} components={components}>
        {text}
      </ReactMarkdown>
    </div>
  )
})
