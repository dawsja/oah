import "@xterm/xterm/css/xterm.css"
import { useLayoutEffect, useRef } from "react"
import { Button } from "@/components/ui/button"
import type { ChatMeta } from "@/lib/chat"
import { AGENT_LABEL } from "@/lib/types"
import { attachCli, detachCli, fitCli, focusCli, leaveCli, restartCli, useCli } from "@/store/cli"

/** The chat in its agent's own CLI: a terminal on the chat's session, in place of the
 *  transcript and the composer. It keeps running while the chat is off screen. */
export function CliView({ meta }: { meta: ChatMeta }) {
  const ref = useRef<HTMLDivElement>(null)
  const session = useCli((s) => s.sessions[meta.id])
  const over = session?.status === "exited" || session?.status === "failed"

  useLayoutEffect(() => {
    const host = ref.current
    if (!host) return
    attachCli(meta.id, host)
    fitCli(meta.id)
    focusCli(meta.id)
    const ro = new ResizeObserver(() => fitCli(meta.id))
    ro.observe(host)
    return () => {
      ro.disconnect()
      detachCli(meta.id, host)
    }
  }, [meta.id])

  return (
    <div className="relative flex min-h-0 flex-1 flex-col bg-sidebar">
      <div className="relative min-h-0 flex-1">
        <div ref={ref} className="absolute inset-0 py-1 pl-2" />
      </div>
      {over ? (
        <div role="status" className="flex shrink-0 flex-wrap items-center justify-end gap-2 border-t bg-background px-4 py-2 text-[12.5px]">
          <p className="min-w-40 flex-1 text-muted-foreground">
            {session.status === "failed" ? session.error : `${AGENT_LABEL[meta.agent]}'s CLI ended${typeof session.code === "number" ? ` with code ${session.code}` : ""}.`}
          </p>
          <Button size="sm" variant="outline" onClick={() => void restartCli(meta.id)}>
            Open it again
          </Button>
          <Button size="sm" onClick={() => void leaveCli(meta.id)}>
            Back to the chat
          </Button>
        </div>
      ) : null}
    </div>
  )
}
