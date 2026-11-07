import type { ReactNode } from "react"
import { cn } from "@/lib/utils"

/** A settings section: title, one-line description, then its groups. */
export function Section({ title, description, actions, children }: { title: string; description?: ReactNode; actions?: ReactNode; children: ReactNode }) {
  return (
    <div className="mx-auto w-full max-w-[52rem] px-10 pt-9 pb-16">
      <div className="mb-6 flex items-end gap-4">
        <div className="min-w-0 flex-1">
          <h1 className="text-[20px] font-semibold tracking-tight">{title}</h1>
          {description ? <p className="mt-1 text-[12.5px] leading-relaxed text-muted-foreground">{description}</p> : null}
        </div>
        {actions}
      </div>
      <div className="flex flex-col gap-6">{children}</div>
    </div>
  )
}

/** A bordered block of rows, with an optional heading above it. */
export function Group({ title, aside, children, className }: { title?: ReactNode; aside?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <section>
      {title || aside ? (
        <div className="mb-2 flex items-center gap-2">
          {title ? <h2 className="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">{title}</h2> : null}
          <div className="ml-auto">{aside}</div>
        </div>
      ) : null}
      <div className={cn("overflow-hidden rounded-lg divide-y border bg-card", className)}>{children}</div>
    </section>
  )
}

/** Label and description on the left, the control on the right. */
export function Row({ label, description, children, className }: { label: ReactNode; description?: ReactNode; children?: ReactNode; className?: string }) {
  return (
    <div className={cn("flex min-h-12 items-center gap-6 px-4 py-2.5", className)}>
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-medium">{label}</div>
        {description ? <div className="mt-0.5 text-[12px] leading-relaxed text-muted-foreground">{description}</div> : null}
      </div>
      {children ? <div className="flex shrink-0 items-center gap-2">{children}</div> : null}
    </div>
  )
}

/** A row of joined buttons, one of them pressed. */
export function Segmented<T extends string>({ value, options, onChange }: { value: T; options: { value: T; label: ReactNode }[]; onChange: (v: T) => void }) {
  return (
    <div className="overflow-hidden rounded-md flex border border-input">
      {options.map((o, i) => (
        <button
          key={o.value}
          className={cn(
            "h-7 px-3 text-[12px] text-muted-foreground hover:bg-accent hover:text-foreground",
            i > 0 && "border-l border-input",
            o.value === value && "bg-primary text-primary-foreground hover:bg-primary hover:text-primary-foreground",
          )}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  )
}

/** A square status dot. */
export function Dot({ tone }: { tone: "ok" | "warn" | "off" | "err" }) {
  const c = { ok: "bg-green", warn: "bg-warn", off: "bg-faint", err: "bg-red" }[tone]
  return <span className={cn("inline-block size-1.5 shrink-0 rounded-full", c)} />
}
