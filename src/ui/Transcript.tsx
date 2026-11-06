// The conversation, in one of five designs:
//   A · Panels   — you: tinted panel with a cyan bar · nur: labelled column
//   B · Bubbles  — you: bubble on the right · nur: bubble on the left
//   Cards        — each exchange in one card: your prompt on top, nur's work and answer below
//   Gutter       — chat log with a speaker column and a colored bar per message
//   Focus        — answer-first: your prompt as a heading, steps folded into one line
import type { SyntaxStyle } from "@opentui/core"
import type { ReactNode } from "react"
import type { Design } from "../config"
import { displayName } from "../catalog"
import type { Entry } from "../session"
import { Shimmer, useTick } from "./Thinking"
import { fmtMs, SPINNER, t } from "./theme"

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
  const tick = useTick(90, liveReasoning !== null || anyRunning || (busy && (design === "focus" || design === "cards")))
  if (design === "cards" || design === "focus") {
    const groups = groupTurns(entries)
    return (
      <box flexDirection="column">
        {groups.map((g, k) => {
          const live = busy && k === groups.length - 1 && g.kind === "turn"
          if (g.kind === "note") return <NoteEntry key={k} e={g.entry} style={style} />
          return design === "cards" ? (
            <Card key={k} turn={g} style={style} tick={tick} live={live} liveText={liveText} liveReasoning={liveReasoning} model={model} />
          ) : (
            <FocusTurn key={k} turn={g} style={style} tick={tick} live={live} liveText={liveText} liveReasoning={liveReasoning} />
          )
        })}
      </box>
    )
  }
  const out: ReactNode[] = []
  entries.forEach((e, i) => {
    const prev = entries[i - 1]
    const firstOfTurn = isAssistantSide(e) && !isAssistantSide(prev)
    const props = { e, i, firstOfTurn, style, tick, model: e.kind === "assistant" ? e.model : model, live: { text: liveText === i, reasoning: liveReasoning === i } }
    out.push(
      design === "bubbles" ? (
        <BubbleEntry key={i} {...props} />
      ) : design === "gutter" ? (
        <GutterEntry key={i} {...props} prev={prev} />
      ) : (
        <PanelEntry key={i} {...props} />
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

// ---- shared: exchanges ---------------------------------------------------------------

interface Turn {
  kind: "turn"
  user?: Extract<Entry, { kind: "user" }>
  steps: { e: Entry; i: number }[]
}

type Group = Turn | { kind: "note"; entry: Entry }

/** Split the transcript into exchanges (your message + nur's work) and standalone notes. */
function groupTurns(entries: Entry[]): Group[] {
  const out: Group[] = []
  let cur: Turn | null = null
  entries.forEach((e, i) => {
    if (e.kind === "user") {
      cur = { kind: "turn", user: e, steps: [] }
      out.push(cur)
    } else if (e.kind === "info" || e.kind === "error") {
      // Errors and notes during a turn stay inside it; others stand alone.
      if (cur && e.kind === "error") cur.steps.push({ e, i })
      else {
        out.push({ kind: "note", entry: e })
        cur = null
      }
    } else {
      if (!cur) {
        cur = { kind: "turn", steps: [] }
        out.push(cur)
      }
      cur.steps.push({ e, i })
    }
  })
  return out
}

function NoteEntry({ e, style }: { e: Entry; style: SyntaxStyle }) {
  if (e.kind === "error")
    return (
      <box marginTop={1}>
        <text fg={t.red}>✗ {e.text}</text>
      </box>
    )
  if (e.kind === "info")
    return (
      <box marginTop={1}>
        <InfoBlock text={e.text} style={style} />
      </box>
    )
  return null
}

function turnStats(turn: Turn) {
  const tools = turn.steps.filter((s) => s.e.kind === "tool").length
  const start = turn.user?.at ?? turn.steps[0]?.e.at
  const lastStep = turn.steps.at(-1)?.e
  const end = lastStep ? (lastStep.kind === "tool" && lastStep.ms !== undefined ? lastStep.started + lastStep.ms : lastStep.at) : undefined
  const secs = start && end ? Math.max(0, Math.round((end - start) / 1000)) : 0
  return { tools, secs }
}

// ---- Cards ----------------------------------------------------------------------------

function Card({ turn, style, tick, live, liveText, liveReasoning, model }: { turn: Turn; style: SyntaxStyle; tick: number; live: boolean; liveText: number | null; liveReasoning: number | null; model: string }) {
  const time = turn.user?.at ? new Date(turn.user.at).toTimeString().slice(0, 5) : ""
  const { tools, secs } = turnStats(turn)
  const answerModel = turn.steps.map((s) => s.e).find((e): e is Extract<Entry, { kind: "assistant" }> => e.kind === "assistant")?.model ?? model
  const footer = [displayName(answerModel), tools ? `${tools} tool${tools === 1 ? "" : "s"}` : "", secs ? fmtMs(secs * 1000) : ""].filter(Boolean).join(" · ")
  return (
    <box marginTop={1} border borderStyle="rounded" borderColor={live ? t.accent : t.border} title={` you${time ? ` · ${time}` : ""} `} titleAlignment="left" flexDirection="column" paddingLeft={1} paddingRight={1}>
      {turn.user ? <text fg={t.you}>{turn.user.text}</text> : null}
      {turn.user && turn.steps.length ? (
        <text fg={t.border} wrapMode="none">
          {"─".repeat(400)}
        </text>
      ) : null}
      {turn.steps.map(({ e, i }) => {
        if (e.kind === "assistant")
          return (
            <box key={i} marginTop={0}>
              <Md text={e.text} style={style} streaming={liveText === i} />
            </box>
          )
        if (e.kind === "reasoning") return <ReasoningBlock key={i} e={e} live={liveReasoning === i} tick={tick} oneLine />
        if (e.kind === "tool") return <ToolBlock key={i} e={e} tick={tick} compact />
        if (e.kind === "error")
          return (
            <text key={i} fg={t.red}>
              ✗ {e.text}
            </text>
          )
        return null
      })}
      {live ? (
        <text fg={t.faint}>
          <Shimmer text="working…" tick={tick} base={t.faint} glow={t.accent} />
        </text>
      ) : (
        <box flexDirection="row" justifyContent="flex-end">
          <text fg={t.faint}>{footer}</text>
        </box>
      )}
    </box>
  )
}

// ---- Gutter ---------------------------------------------------------------------------

const GUTTER = 5

function GutterRow({ label, color, bar, top, children }: { label: string; color: string; bar: string; top: number; children: ReactNode }) {
  return (
    <box flexDirection="row" marginTop={top}>
      <text width={GUTTER} fg={color} wrapMode="none">
        <b>{label.padStart(GUTTER - 1)}</b>
      </text>
      <box width={2} border={["left"]} borderColor={bar} />
      <box flexGrow={1} flexDirection="column">
        {children}
      </box>
    </box>
  )
}

function GutterEntry({ e, style, tick, live, prev, model }: EntryProps & { prev?: Entry }) {
  const speaker = (x?: Entry) => (!x ? "" : x.kind === "user" ? "you" : isAssistantSide(x) ? "nur" : x.kind)
  const newSpeaker = speaker(e) !== speaker(prev)
  const top = newSpeaker ? 1 : 0
  switch (e.kind) {
    case "user":
      return (
        <GutterRow label="you" color={t.you} bar={t.you} top={1}>
          <text fg={t.text}>{e.text}</text>
        </GutterRow>
      )
    case "assistant":
      return (
        <GutterRow label={newSpeaker ? "nur" : ""} color={t.accent} bar={t.accent} top={top}>
          {newSpeaker ? <text fg={t.faint}>{displayName(e.model || model)}</text> : null}
          <Md text={e.text} style={style} streaming={live.text} />
        </GutterRow>
      )
    case "reasoning":
      return (
        <GutterRow label={newSpeaker ? "nur" : "∴"} color={newSpeaker ? t.accent : t.faint} bar={t.accent} top={top}>
          <ReasoningBlock e={e} live={live.reasoning} tick={tick} oneLine />
        </GutterRow>
      )
    case "tool": {
      const { g, c } = statusGlyph(e.status, tick)
      return (
        <GutterRow label={newSpeaker ? "nur" : g} color={newSpeaker ? t.accent : c} bar={t.accent} top={top}>
          {newSpeaker ? (
            <ToolBlock e={e} tick={tick} compact />
          ) : (
            <>
              <text>
                <b fg={t.text}>{splitTitle(e.title)[0]}</b>
                <span fg={t.dim}> {splitTitle(e.title)[1]}</span>
                {e.summary ? <span fg={e.status === "failed" ? t.red : t.faint}> — {e.summary}</span> : null}
                {e.ms && e.ms > 900 ? <span fg={t.faint}> · {fmtMs(e.ms)}</span> : null}
              </text>
              <DiffLines lines={e.detail} indent={0} />
            </>
          )}
        </GutterRow>
      )
    }
    case "info":
      return (
        <GutterRow label="※" color={t.cyan} bar={t.border} top={1}>
          <Md text={e.text} style={style} streaming={false} dim />
        </GutterRow>
      )
    case "error":
      return (
        <GutterRow label="✗" color={t.red} bar={t.red} top={1}>
          <text fg={t.red}>{e.text}</text>
        </GutterRow>
      )
  }
}

// ---- Focus ----------------------------------------------------------------------------

/** One folded line for a run of thinking + tool calls; edits keep their diffs. */
function StepsLine({ steps, tick, running }: { steps: Entry[]; tick: number; running: boolean }) {
  const tools = steps.filter((e): e is ToolEntry => e.kind === "tool")
  const verbs = [...new Set(tools.map((x) => splitTitle(x.title)[0]))]
  const first = steps[0]
  const last = steps.at(-1)!
  const start = first?.kind === "tool" || first?.kind === "reasoning" ? first.started : first?.at
  const end = last.kind === "tool" && last.ms !== undefined ? last.started + last.ms : last.kind === "reasoning" && last.ms !== undefined ? last.started + last.ms : undefined
  const secs = start && end ? Math.round((end - start) / 1000) : 0
  const failed = tools.filter((x) => x.status === "failed" || x.status === "denied").length
  const current = tools.findLast((x) => x.status === "running")
  const edits = tools.filter((x) => /^(Edit|Write|Patch)$/.test(splitTitle(x.title)[0]) && x.detail.length)
  return (
    <box flexDirection="column" marginTop={1}>
      <text>
        {running ? <span fg={t.accent}>{SPINNER[tick % SPINNER.length]} </span> : <span fg={failed ? t.orange : t.faint}>▸ </span>}
        <span fg={t.dim}>{tools.length ? `${tools.length} step${tools.length === 1 ? "" : "s"}` : "thinking"}</span>
        {verbs.length ? <span fg={t.faint}> · {verbs.join(", ")}</span> : null}
        {failed ? <span fg={t.orange}> · {failed} failed</span> : null}
        {secs ? <span fg={t.faint}> · {fmtMs(secs * 1000)}</span> : null}
        {current ? (
          <>
            <span fg={t.faint}> · </span>
            <Shimmer text={current.title} tick={tick} base={t.dim} glow={t.text} />
          </>
        ) : null}
      </text>
      {edits.map((x, k) => (
        <box key={k} flexDirection="column" paddingLeft={2}>
          <text fg={t.faint}>{x.title}</text>
          <DiffLines lines={x.detail} indent={0} />
        </box>
      ))}
    </box>
  )
}

function FocusTurn({ turn, style, tick, live, liveText }: { turn: Turn; style: SyntaxStyle; tick: number; live: boolean; liveText: number | null; liveReasoning: number | null }) {
  // Fold consecutive thinking/tool entries into one steps line each.
  const blocks: ({ kind: "steps"; items: Entry[]; key: number } | { kind: "entry"; e: Entry; i: number })[] = []
  for (const { e, i } of turn.steps) {
    if (e.kind === "tool" || e.kind === "reasoning") {
      const lastBlock = blocks.at(-1)
      if (lastBlock?.kind === "steps") lastBlock.items.push(e)
      else blocks.push({ kind: "steps", items: [e], key: i })
    } else blocks.push({ kind: "entry", e, i })
  }
  return (
    <box flexDirection="column" marginTop={1}>
      {turn.user ? (
        <>
          <text fg={t.border} wrapMode="none">
            {"─".repeat(400)}
          </text>
          <text>
            <b fg={t.you}>› </b>
            <b fg={t.text}>{turn.user.text}</b>
          </text>
        </>
      ) : null}
      {blocks.map((b, k) =>
        b.kind === "steps" ? (
          <StepsLine key={`s${b.key}`} steps={b.items} tick={tick} running={live && k === blocks.length - 1} />
        ) : b.e.kind === "assistant" ? (
          <box key={b.i} marginTop={1}>
            <Md text={b.e.text} style={style} streaming={liveText === b.i} />
          </box>
        ) : b.e.kind === "error" ? (
          <text key={b.i} fg={t.red}>
            ✗ {b.e.text}
          </text>
        ) : null,
      )}
      {live && !blocks.length ? (
        <text marginTop={1}>
          <span fg={t.accent}>{SPINNER[tick % SPINNER.length]} </span>
          <Shimmer text="thinking" tick={tick} base={t.dim} glow={t.accent} />
        </text>
      ) : null}
    </box>
  )
}
