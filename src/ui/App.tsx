// Layout (top → bottom): header · transcript (scrolls) · activity line ·
// composer (or permission prompt) · status bar. Overlays: command palette
// above the composer, centered pickers.
import { RGBA, SyntaxStyle, TextAttributes, type ScrollBoxRenderable, type TextareaRenderable } from "@opentui/core"
import { useKeyboard, useRenderer, useSelectionHandler, useTerminalDimensions } from "@opentui/react"
import { useEffect, useMemo, useRef, useSyncExternalStore } from "react"
import * as auth from "../auth"
import { displayName } from "../catalog"
import { permissionInfo, providerInfo, shortPath, VERSION } from "../config"
import { Shimmer, Thinking, useTick } from "./Thinking"
import { fmtTokens, t } from "./theme"
import { Transcript } from "./Transcript"
import { runningShells } from "../tools/shells"
import type { Store } from "./store"

function makeStyle(renderer: any): SyntaxStyle {
  const c = (hex: string) => RGBA.fromHex(hex)
  return SyntaxStyle.fromStyles(
    {
      default: { fg: c(t.text) },
      "markup.heading": { fg: c(t.accent), bold: true },
      "markup.heading.1": { fg: c(t.accent), bold: true },
      "markup.heading.2": { fg: c(t.accent), bold: true },
      "markup.strong": { fg: c("#ffffff"), bold: true },
      "markup.italic": { italic: true },
      "markup.strikethrough": { dim: true },
      "markup.raw": { fg: c(t.code) },
      "markup.raw.block": { fg: c("#c9d4f6") },
      "markup.link": { fg: c(t.cyan), underline: true },
      "markup.link.label": { fg: c(t.cyan), underline: true },
      "markup.link.url": { fg: c(t.faint) },
      "markup.quote": { fg: c(t.dim), italic: true },
      "markup.list": { fg: c(t.accent) },
      keyword: { fg: c("#c792ea") },
      string: { fg: c("#c3e88d") },
      comment: { fg: c(t.faint), italic: true },
      number: { fg: c("#f78c6c") },
      function: { fg: c("#82aaff") },
      type: { fg: c("#ffcb6b") },
      constant: { fg: c("#f78c6c") },
      variable: { fg: c(t.text) },
      property: { fg: c("#89ddff") },
      operator: { fg: c("#89ddff") },
      punctuation: { fg: c(t.dim) },
    },
    renderer.nativeScene,
  )
}

export function App({ store, onExit }: { store: Store; onExit: () => void }) {
  useSyncExternalStore(store.subscribe, store.getVersion)
  const renderer = useRenderer()
  const { width, height } = useTerminalDimensions()
  const style = useMemo(() => makeStyle(renderer), [renderer])
  const scroll = useRef<ScrollBoxRenderable>(null)
  const input = useRef<TextareaRenderable>(null)
  const s = store.settings

  // Copy selections when the mouse is released.
  useSelectionHandler((sel) => {
    const text = sel.getSelectedText()
    if (text.trim()) store.copy(text) ? store.notify(`Copied ${text.length} characters`) : store.notify("Clipboard unavailable", true)
  })

  // Let the store replace the composer text synchronously (history, clear, …).
  useEffect(() => {
    store.applyInput = (text: string) => {
      const ta = input.current
      if (!ta) return
      ta.setText(text)
      ta.cursorOffset = text.length
    }
    return () => {
      store.applyInput = null
    }
  }, [store])

  useEffect(() => {
    if (store.quit) onExit()
  }, [store.quit])

  const palette = store.palette()
  if (store.paletteIdx >= palette.length) store.paletteIdx = 0

  useKeyboard((key) => {
    const stop = () => key.preventDefault()
    if (key.ctrl && key.name === "c") {
      stop()
      if (store.inputMode !== "prompt") {
        store.inputMode = "prompt"
        store.setInput("")
        return
      }
      return onExit()
    }
    // Permission prompt captures input.
    if (store.permission) {
      stop()
      if (key.name === "y" || key.name === "return") store.answerPermission("once")
      else if (key.name === "a") store.answerPermission("always")
      else if (key.name === "n" || key.name === "escape") store.answerPermission("deny")
      return
    }
    // A question from the model: pick an option, or type an answer and press enter.
    const q = store.question
    if (q) {
      const typing = (input.current?.plainText ?? store.input).length > 0
      const n = q.options.length
      if (key.name === "escape") {
        stop()
        store.setInput("")
        return store.answerQuestion("")
      }
      if (!typing && n) {
        if (key.name === "up" || key.name === "down") {
          stop()
          q.selected = (q.selected + (key.name === "up" ? n - 1 : 1)) % n
          return store.emit(true)
        }
        if (key.name === "space" && q.multiSelect) {
          stop()
          q.checked.has(q.selected) ? q.checked.delete(q.selected) : q.checked.add(q.selected)
          return store.emit(true)
        }
        const digit = Number(key.sequence)
        if (key.sequence && /^[1-9]$/.test(key.sequence) && digit <= n) {
          stop()
          q.selected = digit - 1
          if (q.multiSelect) {
            q.checked.has(q.selected) ? q.checked.delete(q.selected) : q.checked.add(q.selected)
            return store.emit(true)
          }
          return store.answerQuestion(store.questionChoice())
        }
      }
      if ((key.name === "return" || key.name === "kpenter") && !key.shift && !key.meta) {
        stop()
        return store.submit(input.current?.plainText ?? store.input)
      }
      return
    }
    // Picker.
    const p = store.picker
    if (p) {
      stop()
      const n = store.pickerVisible().length
      if (key.name === "escape") store.picker = null
      else if (key.name === "up") p.selected = n ? (p.selected + n - 1) % n : 0
      else if (key.name === "down" || key.name === "tab") p.selected = n ? (p.selected + 1) % n : 0
      else if (key.name === "pageup") p.selected = Math.max(0, p.selected - 10)
      else if (key.name === "pagedown") p.selected = Math.min(n - 1, p.selected + 10)
      else if (key.name === "return") return store.pick()
      else if (key.name === "backspace") (p.filter = p.filter.slice(0, -1)), (p.selected = 0)
      else if (key.sequence && key.sequence.length === 1 && key.sequence >= " " && !key.ctrl) (p.filter += key.sequence), (p.selected = 0)
      return store.emit(true)
    }
    // Command palette.
    if (palette.length) {
      if (key.name === "up" || key.name === "down") {
        stop()
        store.paletteIdx = (store.paletteIdx + (key.name === "up" ? palette.length - 1 : 1)) % palette.length
        return store.emit(true)
      }
      if (key.name === "tab" && !key.shift) {
        stop()
        store.setInput(`/${palette[store.paletteIdx]!.name} `)
        return
      }
      if (key.name === "escape") {
        stop()
        return store.setInput("")
      }
      if (key.name === "return" && !key.shift && !key.meta) {
        stop()
        const cmd = palette[store.paletteIdx]!.name
        store.paletteIdx = 0
        store.setInput("")
        return store.command(`/${cmd}`)
      }
    }
    switch (key.name) {
      case "tab":
        stop()
        return key.shift ? store.cyclePermission() : store.toggleMode()
      case "escape":
        stop()
        if (store.interrupt()) return
        if (store.inputMode !== "prompt") {
          store.inputMode = "prompt"
          store.setInput("")
        }
        return
      case "return":
      case "kpenter":
        if (key.shift || key.meta || key.ctrl) return // textarea inserts the newline
        stop()
        return store.submit(input.current?.plainText ?? store.input)
      case "pageup":
        stop()
        return scroll.current?.scrollBy(-Math.max(1, height - 12))
      case "pagedown":
        stop()
        return scroll.current?.scrollBy(Math.max(1, height - 12))
      case "up": {
        const ta = input.current
        if (ta && ta.logicalCursor.row === 0 && store.historyPrev()) stop()
        return
      }
      case "down": {
        const ta = input.current
        if (ta && ta.logicalCursor.row >= ta.lineCount - 1 && store.historyNext()) stop()
        return
      }
    }
  })

  const busy = store.busy
  const entries = store.session.transcript
  const masked = store.inputMode !== "prompt"
  const modeColor = s.mode === "plan" ? t.plan : t.accent

  return (
    <box width={width} height={height} flexDirection="column" backgroundColor={t.bg}>
      <Header store={store} width={width} />
      <scrollbox ref={scroll} flexGrow={1} stickyScroll stickyStart="bottom" paddingLeft={1} paddingRight={1} scrollbarOptions={{ trackOptions: { foregroundColor: t.borderHi, backgroundColor: t.bg } }}>
        {entries.length === 0 ? <Welcome store={store} /> : null}
        <Transcript entries={entries} design={s.design} style={style} liveText={store.liveText} liveReasoning={store.liveReasoning} busy={!!busy} model={s.model} />
        <TodoPanel store={store} />
        <box height={1} />
      </scrollbox>
      <Activity store={store} />
      {palette.length ? <Palette store={store} items={palette} /> : null}
      {store.question ? <QuestionBox store={store} /> : null}
      {store.permission ? (
        <PermissionBox store={store} />
      ) : (
        <box flexShrink={0} border borderStyle="rounded" borderColor={masked ? t.yellow : modeColor} title={masked ? (store.inputMode === "openai-key" ? " OpenAI API key " : " Anthropic API key ") : undefined} flexDirection="row" paddingLeft={1} paddingRight={1} marginLeft={1} marginRight={1} minHeight={3}>
          <text fg={modeColor}>
            <b>{"› "}</b>
          </text>
          <textarea
            ref={input}
            focused={!store.picker && !store.permission}
            flexGrow={1}
            maxHeight={10}
            minHeight={1}
            textColor={masked ? t.bg : t.text}
            focusedTextColor={masked ? t.bg : t.text}
            placeholder={
              masked
                ? "paste the key and press enter · esc to cancel"
                : store.question
                  ? "or type your own answer and press enter"
                  : busy
                  ? "type to queue a message · esc to interrupt"
                  : s.mode === "plan"
                    ? "describe what to plan… (tab for build mode)"
                    : "ask anything · / for commands · shift+enter for newline"
            }
            placeholderColor={t.faint}
            keyBindings={[
              { name: "return", shift: true, action: "newline" },
              { name: "return", meta: true, action: "newline" },
              { name: "j", ctrl: true, action: "newline" },
            ]}
            onContentChange={() => {
              const v = input.current?.plainText ?? ""
              if (v !== store.input) {
                store.input = v
                store.histIdx = null
                store.emit()
              }
            }}
          />
        </box>
      )}
      <StatusBar store={store} width={width} />
      {store.picker ? <PickerModal store={store} width={width} height={height} /> : null}
    </box>
  )
}

function Header({ store, width }: { store: Store; width: number }) {
  const g = store.session.goal
  const right = `${g && !g.done ? `◎ goal · turn ${g.turns} · ` : ""}session ${store.session.id.slice(0, 8)}`
  const room = Math.max(8, width - 9 - (width > 60 ? right.length + 2 : 0))
  const path = shortPath(store.cwd)
  const shown = path.length > room ? "…" + path.slice(path.length - room + 1) : path
  return (
    <box height={1} flexShrink={0} flexDirection="row" backgroundColor={t.panel} paddingRight={1}>
      <text bg={t.accent} fg={t.bg}>
        <b>{" ◆ nur "}</b>
      </text>
      <text fg={t.dim} flexGrow={1} wrapMode="none">
        {" "}
        {shown}
      </text>
      {width > 60 ? <text fg={g && !g.done ? t.yellow : t.faint}>{right}</text> : null}
    </box>
  )
}

function Welcome({ store }: { store: Store }) {
  const signedIn = auth.available(store.authStatus).length > 0
  const tips: [string, string][] = signedIn
    ? [
        ["/model", "pick a model (ChatGPT plan · OpenAI · Anthropic)"],
        ["tab", "plan ↔ build"],
        ["shift+tab", "auto → always ask → always allow"],
        ["/goal", "work until it's done"],
        ["/design", "panels · bubbles · cards · gutter · focus"],
        ["/help", "keys & commands"],
      ]
    : [
        ["/auth", "continue with ChatGPT, or add an Anthropic / OpenAI key"],
        ["/help", "keys & commands"],
      ]
  return (
    <box flexDirection="column" paddingTop={1} paddingLeft={1}>
      <ascii-font text="nur" font="block" color={[t.accent, t.cyan]} />
      <text>
        <span fg={t.dim}>an AI coding agent for your terminal</span>
        <span fg={t.faint}>  v{VERSION}</span>
      </text>
      <text fg={t.faint}>{shortPath(store.cwd)}</text>
      <box height={1} />
      {tips.map(([k, d]) => (
        <text key={k}>
          <span fg={t.accent}>{k.padEnd(11)}</span>
          <span fg={t.dim}>{d}</span>
        </text>
      ))}
    </box>
  )
}

function TodoPanel({ store }: { store: Store }) {
  const todos = store.session.todos
  if (!todos.length || todos.every((x) => x.status === "completed")) return null
  const done = todos.filter((x) => x.status === "completed").length
  return (
    <box marginTop={1} flexDirection="column" border borderStyle="rounded" borderColor={t.border} title={` Tasks ${done}/${todos.length} `} paddingLeft={1} paddingRight={1}>
      {todos.map((x, i) => (
        <text key={i} fg={x.status === "completed" ? t.faint : x.status === "in_progress" ? t.yellow : t.dim}>
          {x.status === "completed" ? "☒ " : x.status === "in_progress" ? "◐ " : "☐ "}
          {x.status === "completed" ? <span attributes={TextAttributes.STRIKETHROUGH}>{x.text}</span> : x.status === "in_progress" ? <b>{x.text}</b> : x.text}
        </text>
      ))}
    </box>
  )
}

function Activity({ store }: { store: Store }) {
  const b = store.busy
  const toast = store.toast
  return (
    <box height={1} flexShrink={0} flexDirection="row" paddingLeft={2} paddingRight={2}>
      {b ? (
        <box flexGrow={1}>
          <Thinking label={store.activity || "Working"} started={b.started} color={b.kind === "compact" ? t.cyan : b.kind === "login" ? t.yellow : t.accent} />
        </box>
      ) : (
        <box flexGrow={1} />
      )}
      {toast ? (
        <text fg={toast.error ? t.red : t.green}>
          {toast.error ? "✗ " : "✓ "}
          {toast.text}
        </text>
      ) : store.queue.length ? (
        <text fg={t.dim}>{store.queue.length} queued</text>
      ) : null}
    </box>
  )
}

function PermissionBox({ store }: { store: Store }) {
  const p = store.permission!
  return (
    <box flexShrink={0} border borderStyle="rounded" borderColor={t.yellow} title={` Permission · ${p.kind} `} flexDirection="column" paddingLeft={1} paddingRight={1} marginLeft={1} marginRight={1} backgroundColor={t.panel}>
      <text fg={t.text}>{p.title}</text>
      {p.preview.length ? (
        <box flexDirection="column" marginTop={1}>
          {p.preview.slice(0, 12).map((l, i) => {
            const add = l.startsWith("+")
            const del = l.startsWith("-")
            return (
              <text key={i} fg={add ? t.addFg : del ? t.delFg : t.dim} bg={add ? t.addBg : del ? t.delBg : undefined} wrapMode="none">
                {l}
              </text>
            )
          })}
        </box>
      ) : null}
      <text marginTop={1}>
        <span bg={t.yellow} fg={t.bg}>
          <b> y </b>
        </span>
        <span fg={t.text}> allow once   </span>
        <span bg={t.yellow} fg={t.bg}>
          <b> a </b>
        </span>
        <span fg={t.text}> always allow   </span>
        <span bg={t.yellow} fg={t.bg}>
          <b> n </b>
        </span>
        <span fg={t.text}> deny   </span>
        <span fg={t.faint}>esc deny</span>
      </text>
    </box>
  )
}

function QuestionBox({ store }: { store: Store }) {
  const q = store.question!
  const typing = store.input.length > 0
  return (
    <box flexShrink={0} border borderStyle="rounded" borderColor={t.cyan} title=" nur has a question " flexDirection="column" paddingLeft={1} paddingRight={1} marginLeft={1} marginRight={1} backgroundColor={t.panel}>
      <text fg={t.text}>
        <b>{q.question}</b>
      </text>
      {q.options.length ? (
        <box flexDirection="column" marginTop={1}>
          {q.options.map((o, i) => {
            const on = i === q.selected && !typing
            const mark = q.multiSelect ? (q.checked.has(i) ? "☒" : "☐") : on ? "›" : " "
            return (
              <box key={i} flexDirection="row" backgroundColor={on ? t.panelHi : t.panel}>
                <text fg={on ? t.cyan : t.faint} wrapMode="none">
                  {` ${mark} ${i + 1}. `}
                </text>
                <text fg={on ? t.text : t.dim} wrapMode="none">
                  {on ? <b>{o.label}</b> : o.label}
                </text>
                {o.description ? (
                  <text fg={t.faint} flexGrow={1} wrapMode="none">
                    {"  "}
                    {o.description}
                  </text>
                ) : null}
              </box>
            )
          })}
        </box>
      ) : null}
      <text fg={t.faint} marginTop={1}>
        {q.options.length ? (q.multiSelect ? "↑↓ move · space or 1-9 tick · enter submit · " : "↑↓ choose · 1-9 pick · enter submit · ") : ""}
        type below to answer in your own words · esc dismiss
      </text>
    </box>
  )
}

function Palette({ store, items }: { store: Store; items: { name: string; args: string; desc: string }[] }) {
  const n = Math.min(items.length, 10)
  const sel = store.paletteIdx
  const start = sel >= n ? sel - n + 1 : 0
  const nameW = Math.min(44, Math.max(...items.map((c) => c.name.length + (c.args ? c.args.length + 1 : 0))) + 3)
  return (
    <box flexShrink={0} border borderStyle="rounded" borderColor={t.borderHi} backgroundColor={t.panel} marginLeft={1} marginRight={1} flexDirection="column" paddingLeft={1} paddingRight={1}>
      {items.slice(start, start + n).map((c, k) => {
        const i = start + k
        const on = i === sel
        return (
          <box key={c.name} flexDirection="row" backgroundColor={on ? t.panelHi : t.panel}>
            <text width={nameW} wrapMode="none">
              <span fg={on ? t.accent : t.text}>{on ? <b>/{c.name}</b> : `/${c.name}`}</span>
              <span fg={t.faint}> {c.args}</span>
            </text>
            <text fg={t.dim} flexGrow={1} wrapMode="none">
              {c.desc}
            </text>
          </box>
        )
      })}
    </box>
  )
}

function PickerModal({ store, width, height }: { store: Store; width: number; height: number }) {
  const p = store.picker!
  const vis = store.pickerVisible()
  const bw = Math.max(36, Math.min(92, width - 8))
  const rows = Math.max(1, Math.min(vis.length || 1, height - 12))
  const sel = Math.min(p.selected, Math.max(0, vis.length - 1))
  const start = sel >= rows ? sel - rows + 1 : 0
  const labelW = Math.min(Math.floor(bw / 2), Math.max(12, ...vis.map((i) => i.label.length)) + 2)
  let lastGroup = ""
  return (
    <box position="absolute" left={Math.floor((width - bw) / 2)} top={Math.max(1, Math.floor((height - rows - 6) / 2))} width={bw} zIndex={10} border borderStyle="rounded" borderColor={t.accent} backgroundColor={t.panel} title={` ${p.title} `} flexDirection="column" paddingLeft={1} paddingRight={1}>
      <text>
        <span fg={t.dim}>⌕ </span>
        {p.filter ? <span fg={t.text}>{p.filter}▏</span> : <span fg={t.faint}>type to filter</span>}
      </text>
      {vis.length === 0 ? <text fg={t.faint}>no matches</text> : null}
      {vis.slice(start, start + rows).map((item, k) => {
        const i = start + k
        const on = i === sel
        const header = item.group && item.group !== lastGroup ? item.group : null
        lastGroup = item.group ?? lastGroup
        return (
          <box key={`${item.value}-${i}`} flexDirection="column">
            {header ? <text fg={t.faint}>{header}</text> : null}
            <box flexDirection="row" backgroundColor={on ? t.panelHi : t.panel}>
              <text fg={on ? t.accent : t.panel}>▌</text>
              <text fg={t.green}>{item.current ? "● " : "  "}</text>
              <text width={labelW} wrapMode="none" fg={on ? t.text : t.dim}>
                {on ? <b>{item.label}</b> : item.label}
              </text>
              <text fg={t.faint} flexGrow={1} wrapMode="none">
                {item.detail}
              </text>
            </box>
          </box>
        )
      })}
      <text fg={t.faint}>↑↓ select · enter confirm · esc close</text>
    </box>
  )
}

function StatusBar({ store, width }: { store: Store; width: number }) {
  const s = store.settings
  const window = store.contextWindow()
  const used = store.session.contextTokens
  const pct = window ? Math.min(999, Math.round((used / window) * 100)) : 0
  const filled = Math.min(10, Math.ceil(pct / 10))
  const meter = pct >= 85 ? t.red : pct >= 60 ? t.yellow : t.green
  const permColor = s.permission === "auto" ? t.green : s.permission === "ask" ? t.yellow : t.red
  const effortColor = s.effort === "high" ? t.accent : s.effort === "xhigh" || s.effort === "max" ? t.orange : s.effort === "medium" ? t.cyan : t.dim
  const authLabel = auth.label(store.authStatus, store.provider)
  const wide = width >= 110
  return (
    <box height={1} flexShrink={0} flexDirection="row" paddingLeft={2} paddingRight={2}>
      <text flexGrow={1} wrapMode="none">
        <b fg={t.text}>{displayName(s.model)}</b>
        <span fg={t.faint}> · </span>
        <span fg={effortColor}>{s.effort}</span>
        <span fg={t.faint}> · </span>
        <span fg={permColor}>{permissionInfo[s.permission].label}</span>
        <span fg={t.faint}> · </span>
        <span fg={s.mode === "plan" ? t.plan : t.build}>
          <b>{s.mode === "plan" ? "◇ plan" : "◆ build"}</b>
        </span>
        {runningShells().length ? <span fg={t.yellow}>{`  ⧗ ${runningShells().length} bg`}</span> : null}
        {wide ? <span fg={t.faint}>{"  tab mode · shift+tab perms"}</span> : null}
      </text>
      <text wrapMode="none">
        {width >= 90 ? (
          <>
            <span fg={auth.has(store.authStatus, store.provider) ? t.dim : t.yellow}>{authLabel}</span>
            <span fg={t.faint}> · </span>
          </>
        ) : null}
        <span fg={t.dim}>
          {fmtTokens(used)} / {fmtTokens(window)}{" "}
        </span>
        <span fg={meter}>{"▰".repeat(filled)}</span>
        <span fg={t.faint}>{"▱".repeat(10 - filled)}</span>
        <span fg={meter}>
          <b> {pct}%</b>
        </span>
      </text>
    </box>
  )
}

export { providerInfo, Shimmer, useTick }
