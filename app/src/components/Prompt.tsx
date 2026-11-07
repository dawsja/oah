import { useEffect, useState } from "react"
import { create } from "zustand"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"

type Ask = { title: string; description?: string; value?: string; placeholder?: string; confirm?: string; danger?: boolean; input?: boolean; mono?: boolean; resolve: (v: string | null) => void }

const useAsk = create<{ ask: Ask | null }>(() => ({ ask: null }))

/** Asks for one line of text; null when cancelled. */
export function promptText(o: Omit<Ask, "resolve" | "input">): Promise<string | null> {
  return new Promise((resolve) => useAsk.setState({ ask: { ...o, input: true, resolve } }))
}

/** Asks yes or no. */
export function confirmAction(o: Omit<Ask, "resolve" | "input" | "value" | "placeholder">): Promise<boolean> {
  return new Promise((resolve) => useAsk.setState({ ask: { ...o, input: false, resolve: (v) => resolve(v !== null) } }))
}

export function PromptHost() {
  const ask = useAsk((s) => s.ask)
  const [value, setValue] = useState("")
  useEffect(() => setValue(ask?.value ?? ""), [ask])
  const done = (v: string | null) => {
    ask?.resolve(v)
    useAsk.setState({ ask: null })
  }
  return (
    <Dialog open={!!ask} onOpenChange={(o) => !o && done(null)}>
      <DialogContent className="gap-3 sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="text-[15px]">{ask?.title}</DialogTitle>
          {ask?.description ? <DialogDescription className="text-[12.5px]">{ask.description}</DialogDescription> : null}
        </DialogHeader>
        {ask?.input ? (
          <input
            autoFocus
            value={value}
            placeholder={ask.placeholder}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && value.trim() && done(value.trim())}
            className={cn("rounded-md h-8 w-full border border-input bg-background px-2.5 text-[12.5px] outline-none focus:border-ring/70", ask.mono && "font-mono")}
          />
        ) : null}
        <DialogFooter>
          <Button variant="ghost" size="sm" onClick={() => done(null)}>
            Cancel
          </Button>
          <Button size="sm" variant={ask?.danger ? "destructive" : "default"} disabled={ask?.input && !value.trim()} onClick={() => done(ask?.input ? value.trim() : "")}>
            {ask?.confirm ?? "OK"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
