// The conversation, in one of three designs:
//   A · Panels   — you: tinted panel with a cyan bar · nur: labelled column
//   B · Bubbles  — you: bubble on the right · nur: bubble on the left
//   C · Timeline — a single rail with a node per event (variants c, c2, c3, c4)
import type { SyntaxStyle } from "@opentui/core"
import type { ReactNode } from "react"
import type { Design } from "../config"
import { displayName } from "../catalog"
import type { Entry } from "../session"
import { Shimmer, useTick } from "./Thinking"
import { fmtMs, mix, SPINNER, t } from "./theme"

type ToolEntry = Extract<Entry, { kind: "tool" }>
type ReasoningEntry = Extract<Entry, { kind: "reasoning" }>

const statusGlyph = (s: ToolEntry["status"], tick: number) =>
  s === "running" ? { g: SPINNER[tick % SPINNER.length]!, c: t.yellow } : s === "ok" ? { g: "✓", c: t.green } : s === "denied" ? { g: "⊘", c: t.orange } : { g: "✗", c: t.red }

function DiffLines({ lines, indent }: { lines: string[]; indent: number }) {
  if (!lines.length) return null
  return (
    <box flexDirection="column" paddingLeft={indent}>
      {lines.map((l, i) => {
        const add = l.startsWith("+")
        const del = l.startsWith("-")
        return (
          <text key={i} fg={add ? t.addFg : del ? t.delFg : l.startsWith("…") ? t.faint : t.dim} bg={add ? t.addBg : del ? t.delBg : undefined} wrapMode="none">
            {l.replace(/\t/g, "    ")}
          </text>
        )
      })}
    </box>
  )
}

function splitTitle(title: string): [string, string] {
  const i = title.indexOf(" ")
  return i < 0 ? [title, ""] : [title.slice(0, i), title.slice(i + 1)]
}

/** Tool call: ✓ Bash cargo test · 1.2s  /  ⎿ exit 0 · 3 lines  /  preview */
function ToolBlock({ e, tick, compact }: { e: ToolEntry; tick: number; compact?: boolean }) {
  const { g, c } = statusGlyph(e.status, tick)
  const [verb, rest] = splitTitle(e.title)
  const dur = e.ms !== undefined && e.ms > 900 ? ` · ${fmtMs(e.ms)}` : ""
  if (compact) {
    return (
      <box flexDirection="column">
        <text>
          <span fg={t.faint}>▸ </span>
          <b fg={t.text}>{verb}</b>
          <span fg={t.dim}> {rest}</span>
          <span fg={c}>
            {"  "}
            {g}
          </span>
          {e.summary ? <span fg={e.status === "failed" ? t.red : t.faint}> {e.summary}</span> : null}
          <span fg={t.faint}>{dur}</span>
        </text>
        <DiffLines lines={e.detail} indent={2} />
      </box>
    )
  }
  return (
    <box flexDirection="column">
      <text>
        <span fg={c}>{g} </span>
        <b fg={t.text}>{verb}</b>
        <span fg={t.dim}> {rest}</span>
        <span fg={t.faint}>{dur}</span>
      </text>
      {e.summary ? (
        <text>
          <span fg={t.faint}>  ⎿ </span>
          <span fg={e.status === "failed" ? t.red : e.status === "denied" ? t.orange : t.dim}>{e.summary}</span>
        </text>
      ) : null}
      <DiffLines lines={e.detail} indent={4} />
    </box>
  )
}

/** Reasoning summary: animated while streaming, then "Thought for 4s". */
function ReasoningBlock({ e, live, tick, oneLine, bare }: { e: ReasoningEntry; live: boolean; tick: number; oneLine?: boolean; bare?: boolean }) {
  const text = e.text.replace(/\*\*/g, "").trim()
  const lines = text.split("\n").filter((l) => l.trim())
  const label = live ? "Thinking" : `Thought for ${fmtMs(e.ms ?? Date.now() - e.started)}`
  if (oneLine) {
    return (
      <text>
        <span fg={t.faint}>∴ </span>
        {live ? <Shimmer text={label} tick={tick} base={t.faint} glow={t.accent} /> : <span fg={t.faint}>{label}</span>}
        <span fg={t.faint}> — {lines[0]?.slice(0, 120) ?? ""}</span>
      </text>
    )
  }
  const shown = live ? lines.slice(-4) : lines.slice(0, 6)
  return (
    <box flexDirection="column">
      <text>
        {bare ? null : <span fg={live ? t.accent : t.faint}>∴ </span>}
        {live ? <Shimmer text={label} tick={tick} base={t.dim} glow={t.accent} /> : <i fg={t.faint}>{label}</i>}
      </text>
      <box paddingLeft={bare ? 0 : 2} flexDirection="column">
        {shown.map((l, i) => (
          <text key={i} fg={t.faint}>
            <i>{l}</i>
          </text>
        ))}
        {!live && lines.length > 6 ? <text fg={t.faint}>… {lines.length - 6} more lines</text> : null}
      </box>
    </box>
  )
}

function Md({ text, style, streaming, dim }: { text: string; style: SyntaxStyle; streaming: boolean; dim?: boolean }) {
  return <markdown content={text} syntaxStyle={style} streaming={streaming} fg={dim ? t.dim : t.text} conceal />
}

const isAssistantSide = (e: Entry | undefined) => !!e && (e.kind === "assistant" || e.kind === "tool" || e.kind === "reasoning")

export interface TranscriptProps {
  entries: Entry[]
  design: Design
  style: SyntaxStyle
  liveText: number | null
  liveReasoning: number | null
  busy: boolean
  /** Model shown on labels for entries that don't record one. */
  model: string
}

export function Transcript({ entries, design, style, liveText, liveReasoning, busy, model }: TranscriptProps) {
  const anyRunning = entries.some((e) => e.kind === "tool" && e.status === "running")
  const tick = useTick(90, liveReasoning !== null || anyRunning)
  const out: ReactNode[] = []
  entries.forEach((e, i) => {
    const prev = entries[i - 1]
    const firstOfTurn = isAssistantSide(e) && !isAssistantSide(prev)
    const props = { e, i, firstOfTurn, style, tick, model: e.kind === "assistant" ? e.model : model, live: { text: liveText === i, reasoning: liveReasoning === i } }
    const lastOfTurn = isAssistantSide(e) && !isAssistantSide(entries[i + 1])
    const last = i === entries.length - 1 && !busy
    out.push(
      design === "b" ? (
        <BubbleEntry key={i} {...props} />
      ) : design === "a" ? (
        <PanelEntry key={i} {...props} />
      ) : (
        <RailEntry key={i} {...props} last={last} lastOfTurn={lastOfTurn} variant={design} index={i} />
      ),
    )
  })
  return <box flexDirection="column">{out}</box>
}

interface EntryProps {
  e: Entry
  i: number
  firstOfTurn: boolean
  style: SyntaxStyle
  tick: number
  model: string
  live: { text: boolean; reasoning: boolean }
}

function NurLabel({ model, glyph = "◆" }: { model: string; glyph?: string }) {
  return (
    <text>
      <span fg={t.accent}>{glyph} </span>
      <b fg={t.accent}>nur</b>
      {model ? <span fg={t.faint}> · {displayName(model)}</span> : null}
    </text>
  )
}

function InfoBlock({ text, style }: { text: string; style: SyntaxStyle }) {
  return (
    <box flexDirection="row">
      <text fg={t.cyan} width={2}>※</text>
      <box flexGrow={1}>
        <Md text={text} style={style} streaming={false} dim />
      </box>
    </box>
  )
}

// ---- A · Panels -----------------------------------------------------------------

function PanelEntry({ e, firstOfTurn, style, tick, live, model }: EntryProps) {
  const top = firstOfTurn || e.kind === "user" || e.kind === "info" || e.kind === "error" ? 1 : 0
  switch (e.kind) {
    case "user":
      return (
        <box marginTop={1} backgroundColor={t.userBg} border={["left"]} borderColor={t.you} paddingLeft={1} paddingRight={1} flexDirection="column">
          <text>
            <b fg={t.you}>you</b>
          </text>
          <text fg={t.text}>{e.text}</text>
        </box>
      )
    case "assistant":
      return (
        <box marginTop={top} flexDirection="column">
          {firstOfTurn ? <NurLabel model={model} /> : null}
          <box paddingLeft={2} marginTop={firstOfTurn ? 0 : 0}>
            <Md text={e.text} style={style} streaming={live.text} />
          </box>
        </box>
      )
    case "reasoning":
      return (
        <box marginTop={top} flexDirection="column">
          {firstOfTurn ? <NurLabel model={model} /> : null}
          <box paddingLeft={2}>
            <ReasoningBlock e={e} live={live.reasoning} tick={tick} />
          </box>
        </box>
      )
    case "tool":
      return (
        <box marginTop={top} flexDirection="column">
          {firstOfTurn ? <NurLabel model={model} /> : null}
          <box paddingLeft={2}>
            <ToolBlock e={e} tick={tick} />
          </box>
        </box>
      )
    case "info":
      return (
        <box marginTop={1}>
          <InfoBlock text={e.text} style={style} />
        </box>
      )
    case "error":
      return (
        <box marginTop={1}>
          <text fg={t.red}>✗ {e.text}</text>
        </box>
      )
  }
}

// ---- B · Bubbles ----------------------------------------------------------------

function BubbleEntry({ e, style, tick, live }: EntryProps) {
  switch (e.kind) {
    case "user":
      return (
        <box marginTop={1} flexDirection="row" justifyContent="flex-end">
          <box border borderStyle="rounded" borderColor={t.you} title=" you " titleAlignment="right" paddingLeft={1} paddingRight={1} maxWidth="78%" backgroundColor={t.userBg}>
            <text fg={t.text}>{e.text}</text>
          </box>
        </box>
      )
    case "assistant":
      return (
        <box marginTop={1} flexDirection="row">
          <box border borderStyle="rounded" borderColor={live.text ? t.accent : t.borderHi} title={` nur · ${displayName(e.model)} `} paddingLeft={1} paddingRight={1} maxWidth="92%">
            <Md text={e.text} style={style} streaming={live.text} />
          </box>
        </box>
      )
    case "reasoning":
      return (
        <box marginTop={1} paddingLeft={2}>
          <ReasoningBlock e={e} live={live.reasoning} tick={tick} oneLine />
        </box>
      )
    case "tool":
      return (
        <box paddingLeft={2}>
          <ToolBlock e={e} tick={tick} compact />
        </box>
      )
    case "info":
      return (
        <box marginTop={1} paddingLeft={2}>
          <InfoBlock text={e.text} style={style} />
        </box>
      )
    case "error":
      return (
        <box marginTop={1} paddingLeft={2}>
          <text fg={t.red}>✗ {e.text}</text>
        </box>
      )
  }
}

// ---- C · Timeline variants ---------------------------------------------------------
//   c  (C1) Rail     — a node per event, blank rail row between events
//   c2 Compact rail  — no gaps, one line per tool / thought, times on the right
//   c3 Lanes         — rail colored by speaker, time dividers before each of your messages
//   c4 Tree          — one nur node per turn; thinking and tools branch off it

type RailVariant = "c" | "c2" | "c3" | "c4"

const hhmm = (ms?: number) => (ms ? new Date(ms).toTimeString().slice(0, 5) : "")
const nurLane = mix(t.accent, t.bg, 0.55)
const youLane = mix(t.you, t.bg, 0.45)

/** One rail row group: a node glyph (or a bare continuation) plus content. */
function Rail({ glyph, color, lane = t.border, children, last, gap = true, gutter }: { glyph: string | null; color?: string; lane?: string; children: ReactNode; last?: boolean; gap?: boolean; gutter?: string | null }) {
  return (
    <box flexDirection="row">
      {gutter !== undefined ? (
        <text width={6} fg={t.faint} wrapMode="none">
          {gutter ?? ""}
        </text>
      ) : null}
      <box width={3} flexDirection="column">
        {glyph !== null ? <text fg={color}>{glyph}</text> : null}
        {last ? null : <box flexGrow={1} border={["left"]} borderColor={lane} />}
      </box>
      <box flexGrow={1} flexDirection="column" paddingBottom={gap ? 1 : 0}>
        {children}
      </box>
    </box>
  )
}

/** Tree branch inside a turn: ├─ for middle items, └─ for the last. */
function Branch({ last, children }: { last: boolean; children: ReactNode }) {
  return (
    <box flexDirection="row">
      <box width={3} flexDirection="column">
        <text fg={nurLane}>{last ? "└─" : "├─"}</text>
        {last ? null : <box flexGrow={1} border={["left"]} borderColor={nurLane} />}
      </box>
      <box flexGrow={1} flexDirection="column">
        {children}
      </box>
    </box>
  )
}

function Header({ children, time }: { children: ReactNode; time?: string }) {
  return (
    <box flexDirection="row">
      <text flexGrow={1} wrapMode="none">
        {children}
      </text>
      {time ? <text fg={t.faint}>{time}</text> : null}
    </box>
  )
}

/** Spans for a tool's title and result; wrap in <text>. */
function ToolLine({ e }: { e: ToolEntry; tick?: number }) {
  const [verb, rest] = splitTitle(e.title)
  return (
    <>
      <b fg={t.text}>{verb}</b>
      <span fg={t.dim}> {rest}</span>
      {e.summary ? <span fg={e.status === "failed" ? t.red : e.status === "denied" ? t.orange : t.faint}> — {e.summary}</span> : null}
      {e.ms && e.ms > 900 ? <span fg={t.faint}> · {fmtMs(e.ms)}</span> : null}
    </>
  )
}

function RailEntry({ e, style, tick, live, last, firstOfTurn, lastOfTurn, model, variant, index }: EntryProps & { last: boolean; lastOfTurn: boolean; variant: RailVariant; index: number }) {
  const time = hhmm(e.at)
  const compact = variant === "c2"
  const gap = !compact
  const lane = variant === "c3" ? (e.kind === "user" ? youLane : nurLane) : t.border
  const tool = e.kind === "tool" ? statusGlyph(e.status, tick) : null

  // ---- C4 · Tree: a node per turn, entries hang off it ----
  if (variant === "c4") {
    if (e.kind === "user" || e.kind === "info" || e.kind === "error") {
      const glyph = e.kind === "user" ? "●" : e.kind === "info" ? "※" : "✗"
      const color = e.kind === "user" ? t.you : e.kind === "info" ? t.cyan : t.red
      return (
        <Rail glyph={glyph} color={color} last={last} gutter={time}>
          {e.kind === "user" ? (
            <>
              <text>
                <b fg={t.you}>you</b>
              </text>
              <text fg={t.text}>{e.text}</text>
            </>
          ) : e.kind === "info" ? (
            <Md text={e.text} style={style} streaming={false} dim />
          ) : (
            <text fg={t.red}>{e.text}</text>
          )}
        </Rail>
      )
    }
    const body =
      e.kind === "assistant" ? (
        <Md text={e.text} style={style} streaming={live.text} />
      ) : e.kind === "reasoning" ? (
        <ReasoningBlock e={e} live={live.reasoning} tick={tick} oneLine />
      ) : (
        <>
          <text>
            <span fg={tool!.c}>{tool!.g} </span>
            <ToolLine e={e} />
          </text>
          <DiffLines lines={e.detail} indent={2} />
        </>
      )
    return (
      <>
        {firstOfTurn ? (
          <Rail glyph="◆" color={t.accent} gap={false} gutter={time} last={false} lane={nurLane}>
            <text>
              <b fg={t.accent}>nur</b>
              {model ? <span fg={t.faint}> · {displayName(model)}</span> : null}
            </text>
          </Rail>
        ) : null}
        <Rail glyph={null} gutter={null} lane={nurLane} last={last} gap={lastOfTurn}>
          <Branch last={lastOfTurn}>{body}</Branch>
        </Rail>
      </>
    )
  }

  // ---- C1 / C2 / C3 ----
  const divider =
    variant === "c3" && e.kind === "user" && index > 0 ? (
      <box flexDirection="row">
        <text fg={youLane} wrapMode="none">
          {`── ${time || "·····"} ${"─".repeat(200)}`}
        </text>
      </box>
    ) : null
  switch (e.kind) {
    case "user":
      return (
        <>
          {divider}
          <Rail glyph="●" color={t.you} lane={lane} last={last} gap={gap}>
            {compact ? (
              <Header time={time}>
                <b fg={t.you}>you </b>
                <span fg={t.text}>{e.text.split("\n")[0]}</span>
              </Header>
            ) : (
              <Header time={variant === "c3" ? undefined : undefined}>
                <b fg={t.you}>you</b>
              </Header>
            )}
            {compact ? (e.text.includes("\n") ? <text fg={t.text}>{e.text.split("\n").slice(1).join("\n")}</text> : null) : (
              <box backgroundColor={variant === "c3" ? t.userBg : undefined} paddingRight={variant === "c3" ? 1 : 0}>
                <text fg={t.text}>{e.text}</text>
              </box>
            )}
          </Rail>
        </>
      )
    case "assistant":
      return (
        <Rail glyph="◆" color={t.accent} lane={lane} last={last} gap={gap}>
          {firstOfTurn || !compact ? (
            <Header time={compact ? time : undefined}>
              <b fg={t.accent}>nur</b>
              <span fg={t.faint}> · {displayName(e.model)}</span>
            </Header>
          ) : null}
          <Md text={e.text} style={style} streaming={live.text} />
        </Rail>
      )
    case "reasoning":
      return (
        <Rail glyph="∴" color={live.reasoning ? t.accent : t.faint} lane={lane} last={last} gap={gap}>
          {compact ? <ReasoningBlock e={e} live={live.reasoning} tick={tick} oneLine /> : <ReasoningBlock e={e} live={live.reasoning} tick={tick} bare />}
        </Rail>
      )
    case "tool":
      return (
        <Rail glyph={compact || e.status === "running" ? tool!.g : "○"} color={tool!.c} lane={lane} last={last} gap={gap}>
          <text>
            <ToolLine e={e} />
          </text>
          <DiffLines lines={compact ? e.detail.slice(0, 4) : e.detail} indent={0} />
        </Rail>
      )
    case "info":
      return (
        <Rail glyph="※" color={t.cyan} lane={lane} last={last} gap={gap}>
          <Md text={e.text} style={style} streaming={false} dim />
        </Rail>
      )
    case "error":
      return (
        <Rail glyph="✗" color={t.red} lane={lane} last={last} gap={gap}>
          <text fg={t.red}>{e.text}</text>
        </Rail>
      )
  }
}
