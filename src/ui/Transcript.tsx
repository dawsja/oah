// The conversation. Answer-first: your prompt as a heading, oah's thinking and
// tool calls folded into one steps line, edits keep their diffs, then the answer.
import type { SyntaxStyle } from "@opentui/core"
import type { Entry } from "../session"
import { Shimmer, useTick } from "./Thinking"
import { fmtMs, SPINNER, t } from "./theme"

type ToolEntry = Extract<Entry, { kind: "tool" }>

function DiffLines({ lines }: { lines: string[] }) {
  if (!lines.length) return null
  return (
    <box flexDirection="column">
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

const verbOf = (title: string) => title.split(" ", 1)[0]!

// internalBlockMode="top-level": by default OpenTUI coalesces all prose into one block
// whose instant streaming preview is inline-only (shows "## ", list markers, …), then
// tree-sitter swaps in the concealed version — so every delta flips the whole answer
// between two layouts. Per-block mode gives the preview real tokens, so both agree.
function Md({ text, style, streaming, dim }: { text: string; style: SyntaxStyle; streaming: boolean; dim?: boolean }) {
  return <markdown content={text} syntaxStyle={style} streaming={streaming} internalBlockMode="top-level" fg={dim ? t.dim : t.text} conceal />
}

interface Turn {
  kind: "turn"
  user?: Extract<Entry, { kind: "user" }>
  steps: { e: Entry; i: number }[]
}

type Group = Turn | { kind: "note"; entry: Entry }

/** Split the transcript into exchanges (your message + oah's work) and standalone notes. */
export function groupTurns(entries: Entry[]): Group[] {
  const out: Group[] = []
  let cur: Turn | null = null
  entries.forEach((e, i) => {
    if (e.kind === "user") {
      cur = { kind: "turn", user: e, steps: [] }
      out.push(cur)
    } else if (e.kind === "info" || e.kind === "error") {
      // Errors during a turn stay inside it; notes stand alone.
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

function Note({ e, style }: { e: Entry; style: SyntaxStyle }) {
  if (e.kind === "error")
    return (
      <box marginTop={1}>
        <text fg={t.red}>✗ {e.text}</text>
      </box>
    )
  if (e.kind === "info")
    return (
      <box marginTop={1} flexDirection="row">
        <text fg={t.cyan} width={2}>
          ※
        </text>
        <box flexGrow={1}>
          <Md text={e.text} style={style} streaming={false} dim />
        </box>
      </box>
    )
  return null
}

/** One folded line for a run of thinking + tool calls; edits keep their diffs. */
function StepsLine({ steps, tick, running }: { steps: Entry[]; tick: number; running: boolean }) {
  const tools = steps.filter((e): e is ToolEntry => e.kind === "tool")
  const verbs = [...new Set(tools.map((x) => verbOf(x.title)))]
  const first = steps[0]
  const last = steps.at(-1)!
  const start = first?.kind === "tool" || first?.kind === "reasoning" ? first.started : first?.at
  const end = last.kind === "tool" && last.ms !== undefined ? last.started + last.ms : last.kind === "reasoning" && last.ms !== undefined ? last.started + last.ms : undefined
  const secs = start && end ? Math.round((end - start) / 1000) : 0
  const failures = tools.filter((x) => x.status === "failed" || x.status === "denied")
  const current = tools.findLast((x) => x.status === "running")
  const edits = tools.filter((x) => /^(Edit|Write|Patch)$/.test(verbOf(x.title)) && x.detail.length)
  const lastFail = failures.at(-1)
  // Pure thinking in progress: the activity line under the transcript already says so.
  if (running && !tools.length) return null
  return (
    <box flexDirection="column" marginTop={1}>
      <text>
        {running ? <span fg={t.accent}>{SPINNER[tick % SPINNER.length]} </span> : <span fg={failures.length ? t.orange : t.faint}>▸ </span>}
        {tools.length ? (
          <span fg={t.dim}>{`${tools.length} step${tools.length === 1 ? "" : "s"}`}</span>
        ) : running ? (
          <Shimmer text="thinking" tick={tick} base={t.dim} glow={t.accent} />
        ) : (
          <span fg={t.dim}>thought</span>
        )}
        {verbs.length ? <span fg={t.faint}> · {verbs.join(", ")}</span> : null}
        {failures.length ? <span fg={t.orange}> · {failures.length} failed</span> : null}
        {secs ? <span fg={t.faint}> · {fmtMs(secs * 1000)}</span> : null}
        {current ? (
          <>
            <span fg={t.faint}> · </span>
            <Shimmer text={current.title} tick={tick} base={t.dim} glow={t.text} />
          </>
        ) : null}
      </text>
      {!running && lastFail ? (
        <box paddingLeft={2}>
          <text fg={t.red}>
            ✗ {lastFail.title}
            {lastFail.summary ? ` — ${lastFail.summary}` : ""}
          </text>
        </box>
      ) : null}
      {edits.map((x, k) => (
        <box key={k} flexDirection="column" paddingLeft={2}>
          <text fg={t.faint}>{x.title}</text>
          <DiffLines lines={x.detail} />
        </box>
      ))}
    </box>
  )
}

function FocusTurn({ turn, style, tick, live, liveText }: { turn: Turn; style: SyntaxStyle; tick: number; live: boolean; liveText: number | null }) {
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
        <box backgroundColor={t.userBg} paddingLeft={1} paddingRight={1} paddingTop={1} paddingBottom={1}>
          <text>
            <b fg={t.you}>› </b>
            {turn.user.text.split(/(\[Image #\d+\])/).map((part, k) =>
              /^\[Image #\d+\]$/.test(part) ? (
                <b key={k} fg={t.accent}>
                  {part}
                </b>
              ) : (
                <b key={k} fg={t.text}>
                  {part}
                </b>
              ),
            )}
          </text>
        </box>
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
    </box>
  )
}

export interface TranscriptProps {
  entries: Entry[]
  style: SyntaxStyle
  liveText: number | null
  busy: boolean
}

export function Transcript({ entries, style, liveText, busy }: TranscriptProps) {
  const tick = useTick(90, busy)
  const groups = groupTurns(entries)
  return (
    <box flexDirection="column">
      {groups.map((g, k) =>
        g.kind === "note" ? <Note key={k} e={g.entry} style={style} /> : <FocusTurn key={k} turn={g} style={style} tick={tick} live={busy && k === groups.length - 1} liveText={liveText} />,
      )}
    </box>
  )
}
