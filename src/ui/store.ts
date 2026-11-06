// Application state and behaviour. React components subscribe to `version`.
import type { ModelMessage } from "ai"
import { compact, runTurn, systemPrompt, userMessage, type Question, type Reply } from "../agent"
import * as auth from "../auth"
import { acceptsImages, clampEffort, contextWindow, defaultFor, displayName, effortsFor, modelsFor, pickDefault, refreshCatalog } from "../catalog"
import { HELP, matchCommands, resolveCommand, type Command } from "../commands"
import {
  effortInfo,
  KEY_PROVIDERS,
  nextPermission,
  PERMISSIONS,
  permissionInfo,
  providerInfo,
  saveSettings,
  shortPath,
  inferProvider,
  joinModel,
  splitModel,
  type Effort,
  type Mode,
  type Permission,
  type Provider,
  type Settings,
} from "../config"
import type { Mcp } from "../mcp"
import type { Memory } from "../memory"
import { appendHistory, listSessions, loadHistory, loadSession, newId, newSession, relativeTime, saveSession, type Entry, type SessionData } from "../session"
import type { Skills } from "../skills"
import { oneLine, TOOLS, type Outcome } from "../tools"
import { fmtTokens } from "./theme"
import { Snapshots, type Checkpoint } from "../checkpoints"
import { detectLocal, isRunning, LOCAL_PROVIDERS, localBaseURL, type LocalProvider } from "../local"
import { clipboardImage, imageFromFile, pastedPaths, referenced, userContent, type PastedImage } from "../images"
import { killAllShells, setShellListener } from "../tools/shells"

export type PickerKind = "model" | "effort" | "permission" | "session" | "auth" | "undo"

export interface PickItem {
  label: string
  detail: string
  value: string
  current?: boolean
  group?: string
}

export interface Picker {
  kind: PickerKind
  title: string
  items: PickItem[]
  selected: number
  filter: string
}

export interface PendingPermission {
  title: string
  kind: string
  preview: string[]
  resolve: (r: Reply) => void
}

export interface PendingQuestion extends Question {
  selected: number
  checked: Set<number>
  resolve: (answer: string) => void
}

export type BusyKind = "turn" | "compact" | "login"

export interface Busy {
  kind: BusyKind
  started: number
  abort: AbortController
}

type After = { kind: "none" } | { kind: "submit"; text: string } | { kind: "goal" }

export class Store {
  version = 0
  private listeners = new Set<() => void>()
  private pending: ReturnType<typeof setTimeout> | null = null

  session: SessionData
  history: string[] = loadHistory()
  histIdx: number | null = null
  draft = ""
  input = ""
  /** "prompt", or "key:<provider>" while an API key is being pasted. */
  inputMode: "prompt" | `key:${Provider}` = "prompt"
  /** Set by the UI: replaces the composer's text immediately. */
  applyInput: ((text: string) => void) | null = null
  /** Insert text at the composer's cursor (set by App). */
  insertInput: ((text: string) => void) | null = null
  /** Images attached in this conversation, by [Image #N]. */
  images = new Map<number, PastedImage>()
  private imageCount = 0

  busy: Busy | null = null
  activity = ""
  toast: { text: string; error: boolean; at: number } | null = null
  picker: Picker | null = null
  paletteIdx = 0
  permission: PendingPermission | null = null
  question: PendingQuestion | null = null
  queue: string[] = []
  private after: After = { kind: "none" }
  private streamText: number | null = null
  private streamReasoning: number | null = null
  grants = new Set<string>()
  authStatus = auth.status()
  quit = false
  private login: auth.PendingLogin | null = null
  /** Non-interactive runs (oah -p): answer permission prompts and questions without a UI. */
  headless: { ask(req: { title: string; kind: string }): Reply; question(q: Question): string } | null = null

  constructor(
    public settings: Settings,
    session: SessionData | undefined,
    public cwd: string,
    public skills: Skills,
    public memory: Memory,
    public mcp: Mcp,
    public copy: (text: string) => boolean,
  ) {
    settings.model = pickDefault(auth.available(this.authStatus), settings.model)
    this.session = session ?? newSession(settings.model, settings.effort, settings.mode)
    if (session) {
      if (session.model) settings.model = pickDefault(auth.available(this.authStatus), session.model)
      settings.effort = session.effort ?? settings.effort
      settings.mode = session.mode ?? settings.mode
    }
    refreshCatalog().then((changed) => changed && this.emit())
    this.refreshLocal()
    setShellListener(() => this.emit())
  }

  // ---- reactivity -----------------------------------------------------------

  subscribe = (fn: () => void) => {
    this.listeners.add(fn)
    return () => this.listeners.delete(fn)
  }

  getVersion = () => this.version

  /** Coalesce bursts (streaming deltas) into ~60fps updates. */
  emit = (now = false) => {
    if (now) {
      if (this.pending) clearTimeout(this.pending)
      this.pending = null
      this.version++
      for (const l of this.listeners) l()
      return
    }
    if (this.pending) return
    this.pending = setTimeout(() => {
      this.pending = null
      this.version++
      for (const l of this.listeners) l()
    }, 16)
  }

  // ---- helpers --------------------------------------------------------------

  get provider() {
    return splitModel(this.settings.model).provider
  }

  contextWindow() {
    return contextWindow(this.settings.model, this.settings.contextWindow)
  }

  notify(text: string, error = false) {
    this.toast = { text, error, at: Date.now() }
    this.emit()
    setTimeout(() => {
      if (this.toast && Date.now() - this.toast.at >= 3900) {
        this.toast = null
        this.emit()
      }
    }, 4000)
  }

  push(e: Entry): number {
    this.session.transcript.push({ ...e, at: e.at ?? Date.now() })
    this.emit()
    return this.session.transcript.length - 1
  }

  info(text: string) {
    this.push({ kind: "info", text })
  }

  error(text: string) {
    this.push({ kind: "error", text })
  }

  // ---- images ---------------------------------------------------------------------

  /** Attach an image and put its [Image #N] marker at the cursor. */
  attachImage(img: Omit<PastedImage, "n">) {
    const n = ++this.imageCount
    this.images.set(n, { ...img, n })
    this.insertInput?.(`[Image #${n}] `)
    const kb = img.bytes < 1048576 ? `${Math.max(1, Math.round(img.bytes / 1024))} KB` : `${(img.bytes / 1048576).toFixed(1)} MB`
    this.notify(`Attached ${img.name === "clipboard" ? "image from clipboard" : img.name} as [Image #${n}] · ${kb}${acceptsImages(this.settings.model) ? "" : ` — ${displayName(this.settings.model)} can't see images`}`, !acceptsImages(this.settings.model))
  }

  /** ctrl+v: attach the clipboard's image, if it holds one. */
  async pasteClipboardImage() {
    const r = await clipboardImage()
    if (typeof r === "string") return this.notify(r, true)
    if (!r) return this.notify("No image on the clipboard — paste text with your terminal's paste, or drag in an image file", true)
    this.attachImage(r)
  }

  /** Bracketed paste: image file paths become attachments. Returns true if handled. */
  pasteText(text: string): boolean {
    const paths = pastedPaths(text)
    if (!paths.length) return false
    const found = paths.map((p) => imageFromFile(p, this.cwd))
    if (found.some((f) => f === null)) return false // not purely image paths: paste as text
    for (const f of found) {
      if (typeof f === "string") this.notify(f, true)
      else if (f) this.attachImage(f)
    }
    return true
  }

  setInput(text: string) {
    this.input = text
    this.applyInput?.(text)
    this.emit(true)
  }

  persistSettings() {
    this.session.model = this.settings.model
    this.session.effort = this.settings.effort
    this.session.mode = this.settings.mode
    saveSettings(this.settings)
  }

  // ---- palette ---------------------------------------------------------------

  palette(): Command[] {
    if (this.picker || this.permission || this.inputMode !== "prompt") return []
    const t = this.input
    if (!t.startsWith("/") || /\s/.test(t)) return []
    return matchCommands(t.slice(1), this.skills.list)
  }

  // ---- settings ---------------------------------------------------------------

  setModel(full: string) {
    const prev = this.settings.model
    const { provider, slug } = splitModel(full)
    this.settings.model = joinModel(provider, slug)
    this.settings.effort = clampEffort(this.settings.model, this.settings.effort) ?? this.settings.effort
    const before = this.settings.recentModels ?? []
    this.settings.recentModels = [...new Set([this.settings.model, prev, ...before])].filter(Boolean).slice(0, 8)
    this.persistSettings()
    if (!auth.has(this.authStatus, provider)) this.notify(`${displayName(this.settings.model)} needs ${providerInfo[provider].billing} — run /auth`, true)
    else if (provider === "ollama" && !process.env.OLLAMA_CONTEXT_LENGTH)
      this.notify(`Model → ${displayName(this.settings.model)} · Ollama serves a small context by default — start it with OLLAMA_CONTEXT_LENGTH=32768 (or more) for agent work`, true)
    else this.notify(`Model → ${displayName(this.settings.model)} · ${providerInfo[provider].label}`)
  }

  /** Look for Ollama / LM Studio; refresh credentials and an open model picker. */
  async refreshLocal() {
    const before = auth.available(this.authStatus).join()
    await detectLocal()
    this.authStatus = auth.status()
    // Nothing else connected but a local server is up: use it.
    if (!auth.has(this.authStatus, this.provider) && auth.available(this.authStatus).length) this.settings.model = pickDefault(auth.available(this.authStatus), "")
    if (auth.available(this.authStatus).join() === before && this.picker?.kind !== "model") return
    if (this.picker?.kind === "model") {
      const { filter, selected } = this.picker
      this.openModelPicker(false)
      if (this.picker) Object.assign(this.picker, { filter, selected })
    }
    this.emit()
  }

  /** `/model <query>`: exact id, or the single best match across connected providers. */
  switchModel(query: string) {
    const q = query.trim()
    const all = modelsFor(auth.available(this.authStatus))
    const exact = all.find((m) => m.full === q || m.slug === q)
    if (exact || splitModel(q).provider !== inferProvider(q)) return this.setModel(exact?.full ?? q)
    const words = q.toLowerCase().split(/\s+/)
    const hits = all.filter((m) => words.every((w) => `${m.name} ${m.full}`.toLowerCase().includes(w)))
    if (hits.length === 1) return this.setModel(hits[0]!.full)
    this.openModelPicker()
    if (this.picker) {
      this.picker.filter = q
      this.picker.selected = 0
    }
  }

  setEffort(e: Effort) {
    this.settings.effort = e
    this.persistSettings()
    this.notify(`Reasoning effort → ${e}`)
  }

  setPermission(p: Permission) {
    this.settings.permission = p
    this.persistSettings()
    this.notify(`Permissions → ${permissionInfo[p].label}`)
  }

  cyclePermission() {
    this.setPermission(nextPermission(this.settings.permission))
  }

  setMode(m: Mode) {
    this.settings.mode = m
    this.persistSettings()
    this.notify(m === "plan" ? "Plan mode — read-only research and planning" : "Build mode — edits and commands enabled")
  }

  toggleMode() {
    this.setMode(this.settings.mode === "plan" ? "build" : "plan")
  }

  // ---- history ----------------------------------------------------------------

  historyPrev(): boolean {
    if (!this.history.length || this.inputMode !== "prompt") return false
    if (this.histIdx === null) {
      this.draft = this.input
      this.histIdx = this.history.length - 1
    } else if (this.histIdx > 0) this.histIdx--
    this.setInput(this.history[this.histIdx]!)
    return true
  }

  historyNext(): boolean {
    if (this.histIdx === null) return false
    if (this.histIdx + 1 >= this.history.length) {
      this.histIdx = null
      this.setInput(this.draft)
    } else {
      this.histIdx++
      this.setInput(this.history[this.histIdx]!)
    }
    return true
  }

  // ---- submit -----------------------------------------------------------------

  submit(text: string) {
    if (this.question) {
      const typed = text.trim()
      this.setInput("")
      return this.answerQuestion(typed || this.questionChoice())
    }
    if (this.inputMode !== "prompt") {
      const which = this.inputMode.slice(4) as Provider
      this.inputMode = "prompt"
      this.setInput("")
      if (!text.trim()) return
      auth.setKey(which, text)
      this.authStatus = auth.status()
      if (this.provider !== which) this.settings.model = defaultFor(which)
      this.persistSettings()
      this.info(`Saved your ${providerInfo[which].label} key. Model: **${displayName(this.settings.model)}** — /model to change`)
      return
    }
    text = text.replace(/\s+$/, "")
    if (!text.trim()) return
    if (!text.startsWith("/") && referenced(text, this.images).length && !acceptsImages(this.settings.model))
      return this.notify(`${displayName(this.settings.model)} can't see images — switch models with ctrl+t, or remove the [Image] markers`, true)
    this.setInput("")
    this.histIdx = null
    if (this.history.at(-1) !== text) {
      this.history.push(text)
      appendHistory(text)
    }
    if (text.startsWith("/") && !text.startsWith("//")) return this.command(text)
    if (text.startsWith("#") && !text.startsWith("##") && text.slice(1).trim()) {
      try {
        this.memory.add(text.slice(1))
        this.notify(`Saved to project memory: ${oneLine(text.slice(1), 60)}`)
      } catch (e: any) {
        this.notify(e.message, true)
      }
      return
    }
    this.sendUser(text)
  }

  sendUser(text: string, display?: string) {
    if (this.busy) {
      this.queue.push(text)
      this.notify(`Queued · ${this.queue.length} waiting`)
      return
    }
    if (this.needsCompaction()) {
      this.after = { kind: "submit", text }
      return this.startCompact("", true)
    }
    if (!this.session.title) this.session.title = oneLine(display ?? text, 60)
    this.push({ kind: "user", text: display ?? text })
    this.session.messages.push({ role: "user", content: userContent(this.pendingNote ? `${this.pendingNote}\n\n${text}` : text, this.images) })
    this.pendingNote = ""
    this.startTurn()
  }

  needsCompaction() {
    return this.settings.autoCompact && this.session.contextTokens > this.contextWindow() * this.settings.autoCompactAt
  }

  instructions() {
    const goal = this.session.goal && !this.session.goal.done ? this.session.goal.text : undefined
    return systemPrompt({ cwd: this.cwd, mode: this.settings.mode, goal, skills: this.skills.promptSection(), memory: this.memory.promptSection(), mcp: this.mcp.readyNames() })
  }

  startTurn() {
    if (!auth.has(this.authStatus, this.provider)) {
      this.error(`No credentials for ${providerInfo[this.provider].label}. Run /auth, or pick another model with /model.`)
      return
    }
    const abort = new AbortController()
    this.busy = { kind: "turn", started: Date.now(), abort }
    this.activity = "Thinking"
    this.streamText = null
    this.streamReasoning = null
    this.emit(true)
    const model = this.settings.model
    runTurn({
      model,
      effort: this.settings.effort,
      mode: this.settings.mode,
      permission: this.settings.permission,
      instructions: this.instructions(),
      messages: [...this.session.messages],
      settings: this.settings,
      ctx: { cwd: this.cwd, skills: this.skills, memory: this.memory },
      mcp: this.mcp,
      grants: this.grants,
      goalActive: !!this.session.goal && !this.session.goal.done,
      signal: abort.signal,
      hooks: {
        text: (d) => this.onText(d, model),
        reasoning: (d) => this.onReasoning(d),
        reasoningEnd: () => this.endReasoning(),
        toolStart: (callId, name, title) => {
          this.endReasoning()
          this.streamText = null
          this.activity = `Running ${oneLine(title, 60)}`
          this.push({ kind: "tool", callId, name, title, summary: "", detail: [], status: "running", started: Date.now() })
        },
        toolEnd: (callId, o: Outcome, denied) => {
          const e = this.session.transcript.findLast((x) => x.kind === "tool" && x.callId === callId)
          if (e && e.kind === "tool") {
            e.summary = o.summary
            e.detail = o.detail
            e.status = denied ? "denied" : o.ok ? "ok" : "failed"
            e.ms = Date.now() - e.started
          }
          this.activity = "Thinking"
          this.emit()
        },
        ask: (req) =>
          this.headless ? Promise.resolve(this.headless.ask(req)) :
          new Promise<Reply>((resolve) => {
            this.activity = "Waiting for permission"
            this.permission = { ...req, resolve }
            this.emit(true)
          }),
        todos: (items) => {
          this.session.todos = items
          this.emit()
        },
        goalComplete: (summary) => {
          if (this.session.goal) this.session.goal.done = true
          this.push({ kind: "info", text: `◎ **Goal complete** — ${summary}` })
        },
        toolProgress: (callId, summary, detail) => {
          const e = this.session.transcript.findLast((x) => x.kind === "tool" && x.callId === callId)
          if (e && e.kind === "tool") {
            e.summary = summary
            e.detail = detail
            this.emit()
          }
        },
        question: (q) =>
          this.headless ? Promise.resolve(this.headless.question(q)) :
          new Promise<string>((resolve) => {
            this.activity = "Waiting for your answer"
            this.question = { ...q, selected: 0, checked: new Set(), resolve }
            this.emit(true)
          }),
        beforeChange: () => this.takeCheckpoint(),
        spend: (usd) => this.spend(usd),
        usage: (u) => {
          this.session.contextTokens = u.input + u.output
          this.session.totalTokens += u.input + u.output
          this.emit()
        },
      },
    }).then((r) => this.turnDone(r.messages, r.error, r.aborted))
  }

  private onText(d: string, model: string) {
    this.endReasoning()
    this.activity = "Responding"
    const i = this.streamText
    const last = this.session.transcript[i ?? -1]
    if (i !== null && i === this.session.transcript.length - 1 && last?.kind === "assistant") last.text += d
    else this.streamText = this.push({ kind: "assistant", text: d.replace(/^\n+/, ""), model })
    this.emit()
  }

  private onReasoning(d: string) {
    this.activity = "Thinking"
    const i = this.streamReasoning
    const last = this.session.transcript[i ?? -1]
    if (i !== null && i === this.session.transcript.length - 1 && last?.kind === "reasoning") last.text += d
    else {
      this.streamReasoning = this.push({ kind: "reasoning", text: d, started: Date.now() })
      this.streamText = null
    }
    this.emit()
  }

  private endReasoning() {
    if (this.streamReasoning === null) return
    const e = this.session.transcript[this.streamReasoning]
    if (e?.kind === "reasoning" && e.ms === undefined) e.ms = Date.now() - e.started
    this.streamReasoning = null
  }

  /** Index of the reasoning entry still streaming, if any. */
  get liveReasoning() {
    return this.streamReasoning
  }

  get liveText() {
    return this.streamText
  }

  /** Resolves once nothing is running or queued and checkpoint work has finished. */
  whenIdle(): Promise<void> {
    return new Promise((resolve) => {
      const check = () => {
        if (this.busy || this.queue.length) return
        off()
        this.cpChain.then(() => resolve(), () => resolve())
      }
      const off = this.subscribe(() => setTimeout(check, 0))
      setTimeout(check, 0)
    })
  }

  spend(usd: number) {
    this.session.cost = (this.session.cost ?? 0) + usd
    this.emit()
  }

  // ---- checkpoints ------------------------------------------------------------------

  private snaps: Snapshots | null = null
  private get snapshots(): Snapshots {
    if (!this.snaps || this.snaps.sessionId !== this.session.id) this.snaps = new Snapshots(this.cwd, this.session.id)
    return this.snaps
  }
  /** Serializes snapshot work (one index per session). */
  private cpChain: Promise<unknown> = Promise.resolve()
  /** The checkpoint taken during the running turn, if any. */
  private turnCheckpoint: Promise<Checkpoint | null> | null = null
  /** Told to the model with the next message after an /undo. */
  private pendingNote = ""

  /** Snapshot the workspace once per turn, right before the first file-changing tool. */
  private takeCheckpoint(): Promise<void> {
    if (!this.turnCheckpoint) {
      const label = this.session.transcript.findLast((e) => e.kind === "user")?.text ?? "turn"
      this.turnCheckpoint = (this.cpChain = this.cpChain.then(async () => {
        const tree = await this.snapshots.snapshot()
        if (!tree) return null
        const cp: Checkpoint = { id: newId(), tree, label: oneLine(label, 80), at: Date.now(), files: [] }
        ;(this.session.checkpoints ??= []).push(cp)
        return cp
      })) as Promise<Checkpoint | null>
    }
    return this.turnCheckpoint.then(() => {})
  }

  /** After a turn: record which files it changed, drop no-op checkpoints, keep the newest N. */
  private finishCheckpoint() {
    const pending = this.turnCheckpoint
    this.turnCheckpoint = null
    if (!pending) return
    this.cpChain = this.cpChain.then(async () => {
      const cp = await pending
      if (!cp) return
      const now = await this.snapshots.snapshot()
      cp.files = now ? await this.snapshots.diff(cp.tree, now) : []
      const list = (this.session.checkpoints ??= [])
      if (!cp.files.length) list.splice(list.indexOf(cp), 1)
      const max = Math.max(1, this.settings.maxCheckpoints || 20)
      if (list.length > max) list.splice(0, list.length - max)
      saveSession(this.session)
      this.emit()
    })
  }

  openUndoPicker() {
    if (this.busy) return this.notify("Busy — finish or interrupt the current turn first", true)
    const list = this.session.checkpoints ?? []
    if (!list.length) return this.notify(this.snapshots.disabled ? `No checkpoints: ${this.snapshots.disabled}` : "No checkpoints yet — one is taken before the AI's first change in each turn")
    const items: PickItem[] = [...list].reverse().map((cp, i) => ({
      label: `before “${cp.label}”`,
      detail: `${cp.files.length} file${cp.files.length === 1 ? "" : "s"} · ${relativeTime(cp.at)}${i ? ` · also undoes ${i} later turn${i === 1 ? "" : "s"}` : ""}`,
      value: cp.id,
    }))
    this.openPicker("undo", `Undo · restore files to a checkpoint (${list.length}/${this.settings.maxCheckpoints || 20} kept)`, items)
  }

  async undoTo(id: string) {
    const list = this.session.checkpoints ?? []
    const i = list.findIndex((c) => c.id === id)
    const cp = list[i]
    if (!cp) return
    try {
      await this.cpChain
      const touched = await this.snapshots.restore(cp.tree)
      list.splice(i) // this checkpoint and everything after it are used up
      const shown = touched.slice(0, 8).map((f) => `\`${f}\``).join(", ") + (touched.length > 8 ? ` and ${touched.length - 8} more` : "")
      this.info(`↩ Restored files to before “${cp.label}”${touched.length ? ` — ${shown}` : " (nothing had changed)"}`)
      this.pendingNote = `[oah: the user ran /undo. Files were restored to how they were before the turn that began with “${cp.label}”; changes made from then on are gone. Re-read files before editing them.]`
      saveSession(this.session)
    } catch (e: any) {
      this.error(`Undo failed: ${e?.message ?? e}`)
    }
    this.emit(true)
  }

  private turnDone(messages: ModelMessage[], error: string | undefined, aborted: boolean) {
    this.endReasoning()
    this.busy = null
    this.activity = ""
    this.streamText = null
    if (this.permission) {
      this.permission.resolve("deny")
      this.permission = null
    }
    if (this.question) {
      this.question.resolve("")
      this.question = null
    }
    for (const e of this.session.transcript) if (e.kind === "tool" && e.status === "running") e.status = aborted ? "denied" : "failed"
    this.session.messages.push(...messages)
    this.finishCheckpoint()
    if (error) this.error(error)
    if (aborted) this.info("Interrupted.")
    saveSession(this.session)
    this.emit(true)
    if (!error && !aborted) this.afterTurn()
    else if (this.session.goal && !this.session.goal.done) this.info("Goal paused. Send a message to continue, or /goal clear to stop.")
  }

  private afterTurn() {
    if (this.session.goal && !this.session.goal.done) {
      if (this.needsCompaction()) {
        this.after = { kind: "goal" }
        return this.startCompact("the active goal and remaining work", true)
      }
      return this.continueGoal()
    }
    const next = this.queue.shift()
    if (next !== undefined) return this.sendUser(next)
    if (this.needsCompaction()) {
      this.after = { kind: "none" }
      this.startCompact("", true)
    }
  }

  private continueGoal() {
    const g = this.session.goal
    if (!g || g.done) return
    g.turns++
    if (g.turns > 1000) return this.info("Goal paused after 1000 continuation turns.")
    const queued = this.queue.shift()
    if (queued !== undefined) {
      this.push({ kind: "user", text: queued })
      this.session.messages.push(userMessage(queued))
    } else {
      this.push({ kind: "info", text: `↻ Continuing toward goal · turn ${g.turns}` })
      this.session.messages.push(userMessage("Continue working toward the goal. Check what remains, keep going, and verify your work. Call goal_complete only when the goal is fully achieved."))
    }
    this.startTurn()
  }

  startCompact(focus: string, automatic: boolean) {
    if (!this.session.messages.length) return this.notify("Nothing to compact yet")
    const abort = new AbortController()
    this.busy = { kind: "compact", started: Date.now(), abort }
    this.activity = automatic ? "Auto-compacting conversation" : "Compacting conversation"
    this.emit(true)
    compact({ model: this.settings.model, effort: this.settings.effort, settings: this.settings, instructions: this.instructions(), messages: this.session.messages, focus, signal: abort.signal, spend: (usd) => this.spend(usd) })
      .then((summary) => {
        this.busy = null
        this.activity = ""
        const note = `This conversation was compacted to save context. Summary of everything so far:\n\n${summary.trim()}`
        this.session.messages = [userMessage(note), { role: "assistant", content: "Understood — continuing from the summary." }]
        this.session.contextTokens = Math.round(note.length / 4) + 2000
        this.session.compactions++
        this.push({ kind: "info", text: `⇣ **Conversation compacted** (${this.session.compactions} so far)` })
        saveSession(this.session)
        const after = this.after
        this.after = { kind: "none" }
        if (after.kind === "submit") this.sendUser(after.text)
        else if (after.kind === "goal") this.continueGoal()
      })
      .catch((e) => {
        this.busy = null
        this.activity = ""
        this.after = { kind: "none" }
        if (!abort.signal.aborted) this.error(`Compaction failed: ${e?.message ?? e}`)
        this.emit(true)
      })
  }

  interrupt() {
    if (!this.busy) return false
    this.busy.abort.abort()
    if (this.busy.kind === "login") {
      this.login?.cancel()
      this.login = null
      this.busy = null
      this.info("Sign-in cancelled.")
    } else this.activity = "Interrupting…"
    this.queue = []
    this.emit(true)
    return true
  }

  /** Answer the pending ask_user question ("" dismisses it). */
  answerQuestion(answer: string) {
    const q = this.question
    if (!q) return
    this.question = null
    this.activity = "Thinking"
    q.resolve(answer)
    this.emit(true)
  }

  /** The answer for the current selection (checked options, else the highlighted one). */
  questionChoice(): string {
    const q = this.question
    if (!q || !q.options.length) return ""
    const picked = q.multiSelect && q.checked.size ? [...q.checked].sort((a, b) => a - b) : [q.selected]
    return picked.map((i) => q.options[i]?.label ?? "").filter(Boolean).join(", ")
  }

  answerPermission(r: Reply) {
    const p = this.permission
    if (!p) return
    this.permission = null
    this.activity = "Thinking"
    p.resolve(r)
    this.emit(true)
  }

  // ---- commands ---------------------------------------------------------------

  command(line: string) {
    const body = line.slice(1)
    const sp = body.search(/\s/)
    const name = resolveCommand((sp < 0 ? body : body.slice(0, sp)).toLowerCase())
    const args = sp < 0 ? "" : body.slice(sp + 1).trim()
    switch (name) {
      case "help":
        return this.info(HELP)
      case "exit":
        this.quit = true
        return this.emit(true)
      case "model":
        return args ? this.switchModel(args) : this.openModelPicker()
      case "effort": {
        if (args) return EFFORTS_SET.has(args) ? this.setEffort(args as Effort) : this.notify(`Unknown effort '${args}'`, true)
        const allowed = effortsFor(this.settings.model)
        if (!allowed.length) return this.notify(`${displayName(this.settings.model)} has no reasoning effort setting`)
        return this.openPicker("effort", `Reasoning effort · ${displayName(this.settings.model)}`, allowed.map((e) => ({ label: e, detail: effortInfo[e], value: e, current: e === this.settings.effort })))
      }
      case "permissions": {
        const p = ({ auto: "auto", ask: "ask", "always ask": "ask", allow: "allow", "always allow": "allow", yolo: "allow" } as Record<string, Permission>)[args.toLowerCase()]
        if (p) return this.setPermission(p)
        return this.openPicker("permission", "Permission level", PERMISSIONS.map((x) => ({ label: permissionInfo[x].label, detail: permissionInfo[x].desc, value: x, current: x === this.settings.permission })))
      }
      case "plan":
        return this.setMode("plan")
      case "build":
        return this.setMode("build")
      case "auth":
        if (args === "logout") return this.authAction("logout-chatgpt")
        if (args === "login" || args === "chatgpt") return this.authAction("chatgpt")
        if (args) {
          const a = args.toLowerCase().replace(/[\s.]/g, "")
          const p = KEY_PROVIDERS.find((x) => x === a || providerInfo[x].label.toLowerCase().replace(/[\s.]/g, "") === a || (a === "gemini" && x === "google") || (a === "grok" && x === "xai"))
          if (p) return this.authAction(`key:${p}`)
        }
        return this.openAuthPicker()
      case "goal":
        return this.goalCommand(args)
      case "compact":
        if (this.busy) return this.notify("Busy — wait for the current turn or press esc", true)
        this.after = { kind: "none" }
        return this.startCompact(args, false)
      case "undo":
      case "rewind":
        return this.openUndoPicker()
      case "clear":
        return this.newConversation()
      case "session":
        return args ? this.resume(args) : this.openSessionPicker()
      case "memory":
        return this.memoryCommand(args)
      case "skills":
        if (!this.skills.list.length) return this.info("No skills found. Add folders with a `SKILL.md` to `~/.config/oah/skills` or `./.oah/skills` (`.claude/skills` and `.agents/skills` work too).")
        return this.info("**Skills** — run with /<name>, or the model loads them when relevant:\n" + this.skills.list.map((s) => `- **${s.name}** (${s.scope}) — ${s.description}`).join("\n"))
      case "mcp":
        if (!this.mcp.servers.length) return this.info('No MCP servers configured. Add them to `~/.config/oah/mcp.json`, `./.oah/mcp.json` or `./.mcp.json`:\n```json\n{ "mcpServers": { "name": { "command": "npx", "args": ["-y", "some-mcp-server"] } } }\n```')
        return this.info(
          "**MCP servers**\n" +
            this.mcp.servers
              .map((s) => `- **${s.name}** — ${s.state === "ready" ? `ready · ${s.tools.length} tools` : s.state === "failed" ? `failed: ${s.error}` : "starting…"} _(${s.source})_${s.tools.length ? `\n  ${s.tools.join(", ")}` : ""}`)
              .join("\n"),
        )
      case "tools":
        return this.info("**Built-in tools**\n" + [...TOOLS.map((t) => `- **${t.name}**${t.buildOnly ? " _(build mode)_" : ""}${t.openaiOnly ? " _(GPT models)_" : ""} — ${t.description.split("\n")[0]}`), "- **task** — delegate research to a read-only sub-agent", "- **ask_user** — ask you a question with options instead of assuming"].join("\n") + (this.mcp.readyNames().length ? `\n\nPlus MCP tools from: ${this.mcp.readyNames().join(", ")}` : ""))
      case "init":
        return this.sendUser(
          "Analyze this repository and create (or update) an AGENTS.md at the project root that helps AI coding agents work here. Include a short overview, the layout of important directories, build/test/lint commands, code style conventions, and gotchas you discover. Keep it concise and accurate — verify commands by reading config files.",
          "/init",
        )
      case "copy": {
        const last = this.session.transcript.findLast((e) => e.kind === "assistant")
        if (!last || last.kind !== "assistant") return this.notify("No answer to copy yet", true)
        return this.copy(last.text) ? this.notify(`Copied ${last.text.length} characters`) : this.notify("Clipboard unavailable", true)
      }
      default: {
        const skill = this.skills.get(name)
        const bodyText = skill && this.skills.load(skill.name)
        if (!bodyText) return this.notify(`Unknown command /${name} — type / to see commands`, true)
        const msg = args ? `Use the following skill for this request: ${args}\n\n${bodyText}` : `Use the following skill.\n\n${bodyText}`
        return this.sendUser(msg, `/${skill!.name} ${args}`.trim())
      }
    }
  }

  private goalCommand(args: string) {
    if (!args) {
      const g = this.session.goal
      return this.info(g ? (g.done ? `Last goal completed:\n${g.text}` : `**Active goal** (turn ${g.turns}):\n${g.text}`) : "No goal set. Usage: `/goal <objective>` — oah keeps working until it's complete.")
    }
    if (["clear", "stop", "off", "cancel"].includes(args)) {
      this.session.goal = undefined
      return this.notify("Goal cleared")
    }
    this.session.goal = { text: args, turns: 0, done: false }
    this.push({ kind: "info", text: `◎ **Goal set** — working until complete (esc pauses, /goal clear stops)\n${args}` })
    this.sendUser(`Goal: ${args}`)
  }

  private memoryCommand(args: string) {
    const [sub = "", ...rest] = args.split(/\s+/)
    const text = rest.join(" ")
    try {
      switch (sub) {
        case "add":
          this.memory.add(text)
          return this.notify("Saved to project memory")
        case "global":
          this.memory.add(text, true)
          return this.notify("Saved to global memory")
        case "remove":
        case "forget":
          return this.notify(`Removed ${this.memory.remove(text)} entries`)
      }
    } catch (e: any) {
      return this.notify(e.message, true)
    }
    const files = this.memory.instructionFiles()
    let s = "**Memory**\n"
    s += files.length ? files.map((f) => `- Instructions: \`${shortPath(f)}\``).join("\n") + "\n" : "- Instruction files: none (create AGENTS.md, or run /init)\n"
    const notes = this.memory.render()
    s += notes ? `\n${notes}` : "- Notes: none yet. Add with `#note`, `/memory add <text>` or `/memory global <text>`.\n"
    s += `\nFiles: \`${shortPath(this.memory.projectFile)}\` · \`${shortPath(this.memory.globalFile)}\``
    this.info(s)
  }

  newConversation() {
    this.images.clear()
    this.imageCount = 0
    killAllShells()
    this.busy?.abort.abort()
    this.busy = null
    saveSession(this.session)
    this.session = newSession(this.settings.model, this.settings.effort, this.settings.mode)
    this.grants.clear()
    this.queue = []
    this.after = { kind: "none" }
    this.notify("Started a new conversation")
  }

  resume(id: string) {
    const s = loadSession(id)
    if (!s) return this.notify(`No unique session matches '${id}'`, true)
    this.busy?.abort.abort()
    this.busy = null
    saveSession(this.session)
    this.session = s
    if (s.model) this.settings.model = s.model
    this.settings.effort = s.effort
    this.settings.mode = s.mode
    this.info(`Resumed session ${s.id}`)
  }

  // ---- pickers ----------------------------------------------------------------

  openPicker(kind: PickerKind, title: string, items: PickItem[]) {
    this.picker = { kind, title, items, selected: Math.max(0, items.findIndex((i) => i.current)), filter: "" }
    this.emit(true)
  }

  pickerVisible(): PickItem[] {
    const p = this.picker
    if (!p) return []
    const words = p.filter.toLowerCase().split(/\s+/).filter(Boolean)
    return p.items.filter((i) => {
      const hay = `${i.label} ${i.detail} ${i.group ?? ""}`.toLowerCase()
      return words.every((w) => hay.includes(w))
    })
  }

  pick() {
    const p = this.picker
    const item = this.pickerVisible()[p?.selected ?? 0]
    this.picker = null
    if (!p || !item) return this.emit(true)
    switch (p.kind) {
      case "model":
        return this.setModel(item.value)
      case "effort":
        return this.setEffort(item.value as Effort)
      case "permission":
        return this.setPermission(item.value as Permission)
      case "session":
        return this.resume(item.value)
      case "auth":
        return this.authAction(item.value)
      case "undo":
        return void this.undoTo(item.value)
    }
  }

  openModelPicker(probe = true) {
    if (probe) void this.refreshLocal()
    const avail = auth.available(this.authStatus)
    const providers = avail.length ? avail : (["chatgpt", "anthropic", "openai"] as const).slice()
    const all = modelsFor([...providers])
    const item = (m: (typeof all)[number], group: string): PickItem => ({
      label: m.name,
      detail: `${m.context ? `${fmtTokens(m.context)} ctx · ` : ""}${m.full}`,
      value: m.full,
      current: m.full === this.settings.model,
      group,
    })
    const byId = new Map(all.map((m) => [m.full, m]))
    const recent = [...new Set([this.settings.model, ...(this.settings.recentModels ?? [])])].filter((id) => byId.has(id)).slice(0, 5)
    const items: PickItem[] = [
      ...recent.map((id) => item(byId.get(id)!, "Recent")),
      ...all.filter((m) => !recent.includes(m.full)).map((m) => item(m, auth.label(this.authStatus, m.provider))),
    ]
    if (!items.some((i) => i.current)) items.unshift({ label: displayName(this.settings.model), detail: this.settings.model, value: this.settings.model, current: true, group: "Current" })
    this.openPicker("model", avail.length ? "Model · ctrl+t" : "Model · no credentials yet (/auth)", items)
    // Preselect the previous model so ctrl+t → enter flips between the last two.
    const prev = recent.find((id) => id !== this.settings.model)
    if (this.picker && prev) this.picker.selected = this.picker.items.findIndex((i) => i.value === prev)
  }

  openSessionPicker() {
    const items = listSessions()
      .filter((s) => s.id !== this.session.id)
      .slice(0, 200)
      .map((s) => ({ label: s.title || "(untitled)", detail: `${s.id} · ${relativeTime(s.updated)} · ${shortPath(s.cwd)}`, value: s.id }))
    if (!items.length) return this.notify("No other saved sessions yet")
    this.openPicker("session", "Resume a session", items)
  }

  openAuthPicker() {
    const s = (this.authStatus = auth.status())
    const items: PickItem[] = [
      { label: "Continue with ChatGPT", detail: s.chatgpt ? `signed in${s.chatgpt.email ? ` as ${s.chatgpt.email}` : ""} · uses your plan` : "use your ChatGPT plan (Plus, Pro, Business…)", value: "chatgpt", current: !!s.chatgpt?.planUsage, group: "Sign in" },
      ...KEY_PROVIDERS.map((p) => ({
        label: `${providerInfo[p].label} ${providerInfo[p].billing.includes("plan") ? "plan key" : "API key"}`,
        detail: s.keys[p] ? `set (${s.keys[p]}) · enter to replace` : providerInfo[p].blurb,
        value: `key:${p}`,
        current: !!s.keys[p],
        group: "API keys",
      })),
    ]
    for (const p of LOCAL_PROVIDERS) {
      const n = modelsFor([p]).length
      items.push({ label: providerInfo[p].label, detail: s.keys[p] ? `running · ${n} model${n === 1 ? "" : "s"} · enter to pick one` : `not running — start it, then pick it here (${providerInfo[p].baseURL})`, value: `local:${p}`, current: !!s.keys[p], group: "On this machine" })
    }
    if (s.chatgpt) items.push({ label: "Sign out of ChatGPT", detail: "revoke and remove the saved session", value: "logout-chatgpt", group: "Remove" })
    for (const p of KEY_PROVIDERS) if (s.keys[p] === "saved key") items.push({ label: `Remove ${providerInfo[p].label} key`, detail: "delete the saved key", value: `remove:${p}`, group: "Remove" })
    this.openPicker("auth", "Accounts", items)
    if (this.picker) this.picker.selected = 0
  }

  authAction(action: string) {
    if (action.startsWith("key:")) {
      const p = action.slice(4) as Provider
      this.inputMode = `key:${p}`
      this.setInput("")
      return this.notify(`Paste your ${providerInfo[p].label} key and press enter (esc cancels)`)
    }
    if (action.startsWith("local:")) {
      const p = action.slice(6) as LocalProvider
      this.notify(`Looking for ${providerInfo[p].label}…`)
      void this.refreshLocal().then(() => {
        if (!isRunning(p)) return this.notify(`${providerInfo[p].label} isn't running at ${localBaseURL(p)} — start it and try again`, true)
        if (!modelsFor([p]).length) return this.notify(`${providerInfo[p].label} is running but has no chat models with tool calling`, true)
        this.openModelPicker(false)
        if (this.picker) Object.assign(this.picker, { filter: providerInfo[p].label.toLowerCase(), selected: 0 })
        this.emit(true)
      })
      return
    }
    if (action.startsWith("remove:")) {
      auth.setKey(action.slice(7) as Provider, undefined)
      this.authStatus = auth.status()
      return this.notify("Key removed")
    }
    switch (action) {
      case "chatgpt":
        if (this.busy) return this.notify("Busy — finish or interrupt the current turn first", true)
        this.busy = { kind: "login", started: Date.now(), abort: new AbortController() }
        this.activity = "Waiting for ChatGPT sign-in in your browser"
        this.emit(true)
        auth
          .beginChatGptLogin()
          .then((p) => {
            this.login = p
            const opened = auth.openBrowser(p.url)
            this.info(`**Continue with ChatGPT** — ${opened ? "your browser should open. If it doesn't, open this link:" : "open this link in a browser:"}\n${p.displayUrl}\n\nWaiting for sign-in… press esc to cancel.`)
            return p.done
          })
          .then((s) => {
            this.authStatus = s
            if (s.chatgpt?.planUsage) {
              if (this.provider !== "chatgpt") this.settings.model = pickDefault(["chatgpt"], "")
              this.persistSettings()
              this.info(`Signed in${s.chatgpt.email ? ` as ${s.chatgpt.email}` : ""}. **Using ChatGPT plan** with ${displayName(this.settings.model)} — manage usage at ${auth.USAGE_URL}`)
            } else this.info("Signed in, but ChatGPT plan usage wasn't granted. Run /auth again to enable it, or add an API key.")
          })
          .catch((e) => {
            if (this.busy?.kind === "login") this.error(`Sign-in failed: ${e?.message ?? e}`)
          })
          .finally(() => {
            if (this.busy?.kind === "login") this.busy = null
            this.login = null
            this.activity = ""
            this.emit(true)
          })
        return
      case "logout-chatgpt":
        auth.logoutChatGpt().then((note) => {
          this.authStatus = auth.status()
          this.info(note)
        })
        return
    }
  }

  // ---- lifecycle ------------------------------------------------------------------

  shutdown() {
    killAllShells()
    this.question?.resolve("")
    this.busy?.abort.abort()
    this.permission?.resolve("deny")
    this.persistSettings()
    saveSession(this.session)
  }
}

const EFFORTS_SET = new Set<string>(["minimal", "low", "medium", "high", "xhigh", "max"])
