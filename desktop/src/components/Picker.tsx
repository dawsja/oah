// The engine's pickers (/model, /effort, /permissions, /session, /auth, /undo,
// /skills, /mcp) as a command dialog. Search matches the TUI's (every word must
// appear); the pick goes back to the engine by value.
import { useEffect, useMemo, useState } from "react"
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from "@/components/ui/command"
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { useClient } from "@/lib/client"
import { filterItems } from "@/lib/format"
import type { PickItem } from "@/lib/protocol"
import { cn } from "@/lib/utils"
import { PERMISSION } from "./icons"
import { Tip } from "./Tip"

export function Picker() {
  const c = useClient()
  const p = c.state!.picker!
  const [filter, setFilter] = useState(p.filter)
  const [value, setValue] = useState("")
  const vis = useMemo(() => filterItems(p.items, filter), [p.items, filter])
  const hidden = filter.trim() ? 0 : p.items.filter((i) => i.searchOnly).length
  const key = (item: PickItem, i: number) => `${i}:${item.value}`

  // Start on the engine's highlighted row (current model, the previous model…).
  useEffect(() => {
    setFilter(p.filter)
    const want = p.items[p.selected]
    const list = filterItems(p.items, p.filter)
    const i = want ? Math.max(0, list.findIndex((x) => x.value === want.value)) : 0
    setValue(list[i] ? key(list[i]!, i) : "")
  }, [p.kind, p.title])

  const groups = useMemo(() => {
    const out: { name: string; items: { item: PickItem; i: number }[] }[] = []
    vis.forEach((item, i) => {
      const name = item.group ?? ""
      const last = out.at(-1)
      if (last && last.name === name) last.items.push({ item, i })
      else out.push({ name, items: [{ item, i }] })
    })
    return out
  }, [vis])

  const close = () => c.send({ op: "closePicker" })
  return (
    <Dialog open onOpenChange={(o) => !o && close()}>
      <DialogHeader className="sr-only">
        <DialogTitle>{p.title}</DialogTitle>
        <DialogDescription>Search and pick an item</DialogDescription>
      </DialogHeader>
      <DialogContent className="top-[16vh] translate-y-0 gap-0 overflow-hidden p-0 sm:max-w-2xl" showCloseButton={false}>
        <div className="border-b px-4 pt-3 pb-2 text-xs text-muted-foreground">{p.title}</div>
        <Command shouldFilter={false} value={value} onValueChange={setValue} loop className="rounded-none">
          <CommandInput
            autoFocus
            placeholder={p.kind === "model" ? "Search models or providers" : "Type to filter"}
            value={filter}
            onValueChange={(v) => {
              setFilter(v)
              const first = filterItems(p.items, v)[0]
              setValue(first ? key(first, 0) : "")
            }}
          />
          <CommandList className="max-h-[50vh]">
            <CommandEmpty>No matches</CommandEmpty>
            {groups.map((g, k) => (
              <CommandGroup key={`${g.name}-${k}`} heading={g.name || undefined}>
                {g.items.map(({ item, i }) => (
                  <CommandItem key={key(item, i)} value={key(item, i)} onSelect={() => c.send({ op: "pick", value: item.value })} className="group gap-3 py-2">
                    {p.kind === "permission" && item.value in PERMISSION ? (
                      (() => {
                        const P = PERMISSION[item.value as keyof typeof PERMISSION]
                        return <P.icon className={cn("size-4 shrink-0", P.className, !item.current && "opacity-70")} />
                      })()
                    ) : (
                      <span className={item.current ? "size-1.5 shrink-0 rounded-full bg-green" : "size-1.5 shrink-0"} />
                    )}
                    <span className="max-w-[50%] shrink-0 truncate group-data-[selected=true]:font-semibold group-data-[selected=true]:text-primary">{item.label}</span>
                    <Tip content={item.detail} side="bottom"><span className="truncate text-xs text-muted-foreground">{item.detail}</span></Tip>
                  </CommandItem>
                ))}
              </CommandGroup>
            ))}
          </CommandList>
        </Command>
        <div className="flex gap-4 border-t px-4 py-2 text-[11px] text-faint">
          <span>↑↓ select · Enter confirm · Esc close</span>
          {hidden ? <span>type to search {hidden} older</span> : null}
          {filter && vis.length ? <span>{vis.length} match{vis.length === 1 ? "" : "es"}</span> : null}
        </div>
      </DialogContent>
    </Dialog>
  )
}
