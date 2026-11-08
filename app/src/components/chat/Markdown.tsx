import { memo, useState, type ReactNode } from "react"
import { Check, Copy } from "lucide-react"
import ReactMarkdown from "react-markdown"
import rehypeHighlight from "rehype-highlight"
import remarkGfm from "remark-gfm"
import { copyText } from "@/lib/clipboard"
import { cn } from "@/lib/utils"
import { markdownUrlTransform } from "@/lib/images"
import { MarkdownImage } from "./MarkdownImage"
import { useTouch } from "./touch"

function text(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node)
  if (Array.isArray(node)) return node.map(text).join("")
  if (node && typeof node === "object" && "props" in node) return text((node as { props: { children?: ReactNode } }).props.children)
  return ""
}

/** A fenced code block: language label and a copy button over the code. */
function Pre({ children }: { children?: ReactNode }) {
  const [copied, setCopied] = useState(false)
  const touch = useTouch()
  const code = children as { props?: { className?: string } } | undefined
  const lang = /language-([\w+-]+)/.exec(code?.props?.className ?? "")?.[1]
  return (
    <div className="overflow-hidden rounded-lg group/code relative mb-3 border bg-card">
      <div className={cn("flex items-center border-b px-3 font-mono text-faint", touch ? "h-11 text-[12px]" : "h-7 text-[11px]")}>
        {lang ?? "text"}
        <button
          className="ml-auto -mr-2 flex h-full items-center gap-1 px-2 text-muted-foreground opacity-0 group-hover/code:opacity-100 hover:text-foreground [@media(hover:none)]:opacity-100"
          onClick={() => {
            copyText(text(children).replace(/\n$/, ""))
            setCopied(true)
            setTimeout(() => setCopied(false), 1200)
          }}
        >
          {copied ? <Check className="size-3" /> : <Copy className="size-3" />}
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      <pre className="!m-0 !border-0 !bg-transparent">{children}</pre>
    </div>
  )
}

export const Markdown = memo(function Markdown({ text, className }: { text: string; className?: string }) {
  return (
    <div className={cn("md", className)}>
      <ReactMarkdown
        urlTransform={markdownUrlTransform}
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[[rehypeHighlight, { detect: false, ignoreMissing: true }]]}
        components={{
          pre: Pre,
          img: ({ src, alt }) => <MarkdownImage src={typeof src === "string" ? src : undefined} alt={alt} />,
          a: ({ href, children }) => (
            <a href={href} target="_blank" rel="noreferrer">
              {children}
            </a>
          ),
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  )
})
