// Application state and behaviour. React components subscribe to `version`.
import { experimental_createSteering, type ModelMessage, type Experimental_Steering as Steering } from "ai"
import { compact, runTurn, promptParts, userMessage, type Hooks, type Question, type Reply, type TurnOptions } from "../agent"
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
import type { Mcp, McpServer } from "../mcp"
import { importFromClaude } from "../setup"
import { update } from "../update"
import type { Memory } from "../memory"
import { appendHistory, listSessions, loadHistory, loadSession, newId, newSession, relativeTime, saveSession, type Entry, type SessionData } from "../session"
import type { Skills } from "../skills"
import { oneLine, TOOLS, type Outcome, type SetupApi } from "../tools"
import { fmtTokens } from "./theme"
import { Snapshots, type Checkpoint } from "../checkpoints"
import { detectLocal, isRunning, LOCAL_PROVIDERS, localBaseURL, type LocalProvider } from "../local"
import { clipboardImage, clipboardText, imageFromFile, pastedPaths, referenced, userContent, type PastedImage } from "../images"
import { elideImages, estimateContext } from "../context"
import { addUsage } from "../usage"
import { killAllShells, setShellListener } from "../tools/shells"

export type PickerKind = "model" | "effort" | "permission" | "session" | "auth" | "undo" | "skills" | "mcp"

export interface PickItem {
  label: string
  detail: string
  value: string
  current?: boolean
  group?: string
  /** Extra words that match a search (provider names and aliases). */
  keywords?: string
  /** Only listed while searching (older models). */
  searchOnly?: boolean
}

export interface Picker {
  kind: PickerKind
  title: string
  items: PickItem[]
  selected: number
  filter: string
  /** First visible row. */
  scroll?: number
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

  private _busy: Busy | null = null
  /** When the current stretch of goal work began; in memory only, folded into goal.ms when it ends. */
  private goalRun: number | null = null
  activity = ""
  toast: { text: string; error: boolean; at: number } | null = null
  picker: Picker | null = null
  paletteIdx = 0
  permission: PendingPermission | null = null
  question: PendingQuestion | null = null
  /** Prompts from parallel tool calls wait here until the one on screen is answered. */
  private waitingPermissions: PendingPermission[] = []
  private waitingQuestions: PendingQuestion[] = []
  /** The user's answers to the model's questions (context for the safety monitor). */
  private answers: { q: string; a: string }[] = []
  queue: string[] = []
  /** Messages sent mid-turn ("send now") that the model hasn't seen yet. */
  steers: { id: string; text: string }[] = []
  private steering: Steering | null = null
  /** A newer oah release, from the daily check (shown on the welcome screen). */
  updateAvailable: string | null = null
  private updating = false
  /** When ctrl+c was last pressed while idle (a second press soon after exits). */
  private exitArmed = 0
  private after: After = { kind: "none" }
  private streamText: number | null = null
  private streamReasoning: number | null = null
  grants = new Set<string>()
  authStatus = auth.status()
  quit = false
  /** A browser sign-in in progress (ChatGPT or an MCP server): its link, and how to cancel it. */
  private login: { url: string; cancel: () => void } | null = null
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

  /** What's running. Setting it also starts and stops the active goal's clock. */
  get busy(): Busy | null {
    return this._busy
  }
  set busy(b: Busy | null) {
    this.stopGoalClock()
    this._busy = b
    this.startGoalClock()
  }

  private startGoalClock() {
    const g = this.session.goal
    if (this._busy && this._busy.kind !== "login" && g && !g.done) this.goalRun = Date.now()
  }

  private stopGoalClock() {
    if (this.goalRun === null) return
    const g = this.session.goal
    if (g) g.ms = (g.ms ?? 0) + Date.now() - this.goalRun
    this.goalRun = null
  }

  /** Time spent on the goal so far, including work in progress. */
  goalElapsed(): number {
    const g = this.session.goal
    if (!g) return 0
    return (g.ms ?? 0) + (this.goalRun !== null ? Date.now() - this.goalRun : 0)
  }

  /** Whether the goal's clock is ticking right now. */
  get goalRunning(): boolean {
    return this.goalRun !== null
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

  /**
   * ctrl+v (or an empty paste from the terminal): read the clipboard and do the
   * right thing — attach an image, or paste text (image file paths still attach).
   */
  async pasteFromClipboard() {
    const img = await clipboardImage()
    if (typeof img === "string") return this.notify(img, true)
    if (img) return this.attachImage(img)
    const text = (await clipboardText())?.replace(/\r\n?/g, "\n")
    if (!text) return this.notify("The clipboard is empty", true)
    if (!this.pasteText(text)) this.insertInput?.(text)
  }

  /** Attach the clipboard's image, if it holds one (the desktop app's "paste image"). */
  async pasteClipboardImage() {
    const r = await clipboardImage()
    if (typeof r === "string") return this.notify(r, true)
    if (!r) return this.notify("No image on the clipboard", true)
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

  /** `now`: while a turn runs, send the message into it instead of queuing it. */
  submit(text: string, now = false) {
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
    if (!text.trim()) {
      // Enter on an empty input sends waiting messages now (for terminals without ctrl+enter).
      if (this.queue.length) this.sendQueuedNow()
      return
    }
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
    this.sendUser(text, undefined, now)
  }

  sendUser(text: string, display?: string, now = false) {
    if (this.busy) {
      if (now && this.canSteer()) return this.steer(text)
      this.queue.push(text)
      this.notify(`Queued · ${this.queue.length} waiting · ctrl+enter to send now`)
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

  /**
   * Show an approval prompt. Parallel tool calls can ask at the same time: extra
   * requests queue behind the one on screen instead of replacing it (a replaced
   * prompt would leave its tool waiting forever).
   */
  requestPermission(req: { title: string; kind: string; preview: string[] }): Promise<Reply> {
    return new Promise<Reply>((resolve) => {
      const p = { ...req, resolve }
      if (this.permission) {
        this.waitingPermissions.push(p)
        return this.emit()
      }
      this.activity = "Waiting for permission"
      this.permission = p
      this.emit(true)
    })
  }

  /** Show a question from the model; queued like approval prompts. */
  requestAnswer(q: Question): Promise<string> {
    return new Promise<string>((resolve) => {
      const pending = { ...q, selected: 0, checked: new Set<number>(), resolve }
      if (this.question) {
        this.waitingQuestions.push(pending)
        return this.emit()
      }
      this.activity = "Waiting for your answer"
      this.question = pending
      this.emit(true)
    })
  }

  /** Deny / dismiss every prompt on screen or queued (turn ended or interrupted). */
  private dropPrompts() {
    for (const p of [this.permission, ...this.waitingPermissions.splice(0)]) p?.resolve("deny")
    this.permission = null
    for (const q of [this.question, ...this.waitingQuestions.splice(0)]) q?.resolve("")
    this.question = null
  }

  /** What the user has asked for, as the safety monitor sees it. */
  private intentContext(): string {
    const users = this.session.transcript.filter((e) => e.kind === "user").slice(-8)
    let s = `The user's messages in this session (oldest first):\n${users.map((e) => `- ${e.kind === "user" ? oneLine(e.text, 800) : ""}`).join("\n") || "- (none)"}`
    const answers = this.answers.slice(-6)
    if (answers.length) s += `\n\nThe user's answers to the agent's questions:\n${answers.map((x) => `- Q: ${oneLine(x.q, 200)}\n  A: ${oneLine(x.a, 400)}`).join("\n")}`
    return s
  }

  /** Activity line while a turn runs: the newest still-running tool, else thinking. */
  private workingActivity(): string {
    const running = this.session.transcript.filter((e) => e.kind === "tool" && e.status === "running")
    const last = running.at(-1)
    if (!last || last.kind !== "tool") return "Thinking"
    return `Running ${oneLine(last.title, 60)}${running.length > 1 ? ` +${running.length - 1} more` : ""}`
  }

  /** Approval prompts / questions queued behind the one on screen. */
  get waitingPrompts(): number {
    return this.waitingPermissions.length + this.waitingQuestions.length
  }

  private canSteer() {
    return this.busy?.kind === "turn" && !!this.steering
  }

  /** Hand a message to the running turn; the model sees it after the current step. */
  private steer(text: string) {
    const id = newId()
    this.steers.push({ id, text })
    this.steering!.steer({ role: "user", content: userContent(text, this.images) }, { id })
    this.notify("Sending now · the model sees it after its current step")
  }

  /** Send every queued message into the running turn (or start a turn with them). */
  sendQueuedNow() {
    if (!this.queue.length) return
    if (!this.busy) return this.afterTurn()
    if (!this.canSteer()) return this.notify("Queued messages are sent when this finishes", true)
    for (const text of this.queue.splice(0)) this.steer(text)
  }

  /** ctrl+c while idle: the first press arms, a second within 2s confirms the exit. */
  confirmExit(): boolean {
    if (Date.now() - this.exitArmed < 2000) return true
    this.exitArmed = Date.now()
    this.notify("Press ctrl+c again to exit")
    return false
  }

  /** Put unsent messages back into the input box so nothing typed is lost. */
  private restoreToInput(texts: string[]) {
    if (!texts.length) return
    this.setInput([...texts, this.input].filter((t) => t.trim()).join("\n\n"))
    this.notify(`${texts.length} unsent message${texts.length === 1 ? "" : "s"} moved back to the input`)
  }

  private compactedTokens = -1

  needsCompaction() {
    return this.session.contextTokens !== this.compactedTokens && this.settings.autoCompact && this.session.contextTokens > this.contextWindow() * this.settings.autoCompactAt
  }

  instructions() {
    const goal = this.session.goal && !this.session.goal.done ? this.session.goal.text : undefined
    return promptParts({ cwd: this.cwd, mode: this.settings.mode, goal, skills: this.skills.promptSection(), memory: this.memory.promptSection(), mcp: this.mcp.readyNames() }).system
  }

  private fileVersions = new Map<string, string | null>()

  startTurn() {
    if (!auth.has(this.authStatus, this.provider)) {
      this.error(`No credentials for ${providerInfo[this.provider].label}. Run /auth, or pick another model with /model.`)
      return
    }
    const goal = this.session.goal && !this.session.goal.done ? this.session.goal.text : undefined
    const { context } = promptParts({ cwd: this.cwd, mode: this.settings.mode, goal, skills: this.skills.promptSection(), memory: this.memory.promptSection(), mcp: this.mcp.readyNames() })
    if (context !== this.session.runtimeContext) {
      this.session.messages.push(userMessage(context))
      this.session.runtimeContext = context
    }
    const abort = new AbortController()
    this.busy = { kind: "turn", started: Date.now(), abort }
    const steering = (this.steering = experimental_createSteering())
    this.steers = []
    this.activity = "Thinking"
    this.streamText = null
    this.streamReasoning = null
    this.emit(true)
    const model = this.settings.model
    const session = this.session
    const current = () => this.session === session && this.busy?.abort === abort
    const options: TurnOptions = {
      sessionId: this.session.id,
      verification: this.session.verification,
      model,
      effort: this.settings.effort,
      mode: this.settings.mode,
      permission: this.settings.permission,
      instructions: this.instructions(),
      messages: [...this.session.messages],
      settings: this.settings,
      ctx: { cwd: this.cwd, skills: this.skills, memory: this.memory, setup: this.setupApi, fileVersions: this.fileVersions },
      mcp: this.mcp,
      grants: this.grants,
      goalActive: !!this.session.goal && !this.session.goal.done,
      signal: abort.signal,
      steering,
      hooks: {
        reviewing: (callId, on) => {
          if (this.permission || this.question) return
          const e = this.session.transcript.findLast((x) => x.kind === "tool" && x.callId === callId)
          this.activity = on ? `Safety check · ${e?.kind === "tool" ? oneLine(e.title, 60) : "reviewing proposed actions"}` : this.workingActivity()
          this.emit()
        },
        intent: () => this.intentContext(),
        steered: (id) => {
          const i = this.steers.findIndex((x) => x.id === id)
          if (i < 0) return
          const [m] = this.steers.splice(i, 1)
          this.endReasoning()
          this.streamText = null
          this.push({ kind: "user", text: m!.text })
          this.emit(true)
        },
        text: (d) => this.onText(d, model),
        reasoning: (d) => this.onReasoning(d),
        reasoningEnd: () => this.endReasoning(),
        toolStart: (callId, name, title) => {
          this.endReasoning()
          this.streamText = null
          if (!this.permission && !this.question) this.activity = `Running ${oneLine(title, 60)}`
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
          if (!this.permission && !this.question) this.activity = this.workingActivity()
          this.emit()
        },
        ask: (req) => (this.headless ? Promise.resolve(this.headless.ask(req)) : this.requestPermission(req)),
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
        question: (q) => (this.headless ? Promise.resolve(this.headless.question(q)) : this.requestAnswer(q)),
        beforeChange: () => this.takeCheckpoint(),
        spend: (usd) => this.spend(usd),
        verification: (state) => { this.session.verification = structuredClone(state) },
        usage: (u) => {
          this.session.lastUsage = u
          this.session.usage = addUsage(this.session.usage, u)
          this.session.contextTokens = u.input + u.output
          this.session.totalTokens += u.input + u.output
          this.emit()
        },
      },
    }
    const hooks = options.hooks
    options.hooks = Object.fromEntries(Object.entries(hooks).map(([name, fn]) => [name, (...args: any[]) => current() ? (fn as Function)(...args) : name === "ask" ? "deny" : name === "question" || name === "intent" ? "" : undefined])) as unknown as Hooks
    runTurn(options).then((r) => { if (current()) this.turnDone(r.messages, r.error, r.aborted, r.history) })
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
    const session = this.session
    const snapshots = this.snapshots
    if (!this.turnCheckpoint) {
      const label = this.session.transcript.findLast((e) => e.kind === "user")?.text ?? "turn"
      this.turnCheckpoint = (this.cpChain = this.cpChain.then(async () => {
        const tree = await snapshots.snapshot()
        if (!tree) return null
        const cp: Checkpoint = { id: newId(), tree, label: oneLine(label, 80), at: Date.now(), files: [] }
        ;(session.checkpoints ??= []).push(cp)
        return cp
      })) as Promise<Checkpoint | null>
    }
    return this.turnCheckpoint.then(() => {})
  }

  /** After a turn: record which files it changed, drop no-op checkpoints, keep the newest N. */
  private finishCheckpoint() {
    const session = this.session
    const snapshots = this.snapshots
    const pending = this.turnCheckpoint
    this.turnCheckpoint = null
    if (!pending) return
    this.cpChain = this.cpChain.then(async () => {
      const cp = await pending
      if (!cp) return
      const now = await snapshots.snapshot()
      cp.files = now ? await snapshots.diff(cp.tree, now) : []
      const list = (session.checkpoints ??= [])
      if (!cp.files.length) list.splice(list.indexOf(cp), 1)
      const max = Math.max(1, this.settings.maxCheckpoints || 20)
      if (list.length > max) list.splice(0, list.length - max)
      saveSession(session)
      if (this.session === session) this.emit()
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
      this.fileVersions.clear()
      this.session.verification = { revision: (this.session.verification?.revision ?? 0) + 1, status: "unverified", commands: [] }
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

  private turnDone(messages: ModelMessage[], error: string | undefined, aborted: boolean, history?: ModelMessage[]) {
    this.endReasoning()
    this.busy = null
    this.activity = ""
    this.streamText = null
    this.steering = null
    // Messages sent mid-turn that never reached the model: send them next, or
    // give them back if the turn stopped early.
    const unsent = this.steers.splice(0).map((x) => x.text)
    if (error || aborted) this.restoreToInput(unsent)
    else this.queue.unshift(...unsent)
    this.dropPrompts()
    for (const e of this.session.transcript) if (e.kind === "tool" && e.status === "running") e.status = aborted ? "denied" : "failed"
    if (history) this.session.messages = history
    else this.session.messages.push(...messages)
    this.session.messages = elideImages(this.session.messages)
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
    if (this.needsCompaction()) {
      this.after = { kind: "none" }
      return this.startCompact("", true)
    }
    const next = this.queue.shift()
    if (next !== undefined) return this.sendUser(next)
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
    const session = this.session
    const abort = new AbortController()
    const current = () => this.session === session && this.busy?.abort === abort
    this.busy = { kind: "compact", started: Date.now(), abort }
    this.activity = automatic ? "Auto-compacting conversation" : "Compacting conversation"
    this.emit(true)
    this.cpChain.then(() => {
      abort.signal.throwIfAborted()
      if (!current()) throw new Error("Compaction superseded")
      return compact({ model: this.settings.model, effort: this.settings.effort, settings: this.settings, instructions: this.instructions(), messages: this.session.messages, focus: `${focus}\nKnown state: ${JSON.stringify({ todos: this.session.todos, changedFiles: this.session.checkpoints?.flatMap((c) => c.files), failures: this.session.transcript.filter((e) => e.kind === "tool" && e.status === "failed").slice(-8) })}`, signal: abort.signal, spend: (usd) => this.spend(usd) })
    })
      .then((messages) => {
        if (!current()) return
        this.busy = null
        this.activity = ""
        this.session.messages = messages
        this.session.contextTokens = estimateContext(messages)
        this.compactedTokens = this.session.contextTokens
        this.session.compactions++
        this.push({ kind: "info", text: `⇣ **Conversation compacted** (${this.session.compactions} so far)` })
        saveSession(this.session)
        const after = this.after
        this.after = { kind: "none" }
        if (after.kind === "submit") this.sendUser(after.text)
        else if (after.kind === "goal") this.continueGoal()
        else {
          const queued = this.queue.shift()
          if (queued !== undefined) this.sendUser(queued)
        }
        this.emit(true)
      })
      .catch((e) => {
        if (!current()) return
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
    this.restoreToInput(this.queue.splice(0))
    this.emit(true)
    return true
  }

  // ---- MCP & self-setup -------------------------------------------------------------

  private mcpCommand(args: string) {
    const [sub = "", ...rest] = args.trim().split(/\s+/)
    const name = rest.join(" ")
    if (sub === "login" || sub === "signin") return name ? this.mcpSignIn(name) : this.notify("Usage: /mcp login <server>", true)
    if (sub === "logout" || sub === "signout") {
      if (!name) return this.notify("Usage: /mcp logout <server>", true)
      return void this.mcp.signOut(name).then((ok) => (ok ? this.info(`Signed out of **${name}**`) : this.notify(`Not signed in to ${name}`, true)))
    }
    if (sub === "reload") return void this.setupApi.reloadMcp().then((r) => this.info(r))
    if (!this.mcp.servers.length)
      return this.info(
        'No MCP servers yet. Ask oah to add one ("add the Linear MCP server"), run **/import** to bring over Claude Code\'s, or add them to `~/.config/oah/mcp.json`, `./.oah/mcp.json` or `./.mcp.json`:\n```json\n{ "mcpServers": { "docs": { "url": "https://example.com/mcp" }, "fs": { "command": "npx", "args": ["-y", "some-mcp-server"] } } }\n```',
      )
    this.openPicker(
      "mcp",
      "MCP servers · enter: sign in / details / retry",
      this.mcp.servers.map((x) => ({ label: x.name, detail: `${mcpState(x)}${x.signedIn && x.state === "ready" ? " · signed in" : ""} · ${x.source}`, value: x.name })),
    )
  }

  private mcpPicked(name: string) {
    const x = this.mcp.get(name)
    if (!x) return
    if (x.state === "needs-auth") return this.mcpSignIn(name)
    if (x.state === "failed") {
      this.notify(`Reconnecting ${name}…`)
      return void this.mcp.reconnect(name).then(() => this.notify(`${name}: ${mcpState(this.mcp.get(name)!)}`, this.mcp.get(name)?.state !== "ready"))
    }
    if (x.state === "starting") return this.notify(`${name} is still connecting`)
    this.info(`**${name}** — ${x.tools.length} tools _(${x.source})_${x.signedIn ? " · signed in (/mcp logout " + name + ")" : ""}\n${x.tools.join(", ")}`)
  }

  /** /mcp: browser sign-in to a server that needs authentication. */
  mcpSignIn(name: string) {
    if (!this.mcp.get(name)) return this.notify(`No MCP server named ${name}`, true)
    if (this.busy) return this.notify("Busy — finish or interrupt the current turn first", true)
    const abort = new AbortController()
    this.busy = { kind: "login", started: Date.now(), abort }
    this.activity = `Waiting for ${name} sign-in in your browser · c to copy link`
    this.emit(true)
    this.runMcpSignIn(name, abort.signal)
      .then((msg) => this.info(msg))
      .catch((e) => {
        if (!abort.signal.aborted) this.error(`Sign-in to ${name} failed: ${e?.message ?? e}`)
      })
      .finally(() => {
        if (this.busy?.kind === "login") this.busy = null
        this.activity = ""
        this.emit(true)
      })
  }

  private async runMcpSignIn(name: string, signal: AbortSignal): Promise<string> {
    if (!this.mcp.get(name)) throw new Error(`no MCP server named '${name}'`)
    const ctl = new AbortController()
    const stop = () => ctl.abort()
    signal.addEventListener("abort", stop, { once: true })
    const timer = setTimeout(stop, 5 * 60_000)
    try {
      await this.mcp.signIn(name, {
        signal: ctl.signal,
        onUrl: (url) => {
          this.login = { url, cancel: stop }
          const opened = auth.openBrowser(url)
          this.info(`**Sign in to ${name}** — ${opened ? "your browser should open. If it doesn't, open this link:" : "open this link in a browser:"}\n${url}\n\nWaiting for sign-in… press **c** to copy the link, esc to cancel.`)
        },
      })
      const x = this.mcp.get(name)
      return x?.state === "ready" ? `Signed in to **${name}** · ${x.tools.length} tools` : `Signed in to ${name}, but it is ${x ? mcpState(x) : "gone"}`
    } catch (e: any) {
      if (ctl.signal.aborted && !signal.aborted) throw new Error("timed out waiting for the browser sign-in")
      throw e
    } finally {
      clearTimeout(timer)
      signal.removeEventListener("abort", stop)
      this.login = null
    }
  }

  /** What the oah_setup tool calls into. */
  readonly setupApi: SetupApi = {
    status: () => {
      const servers = this.mcp.servers.map((x) => `- ${x.name}: ${mcpState(x)}${x.signedIn ? " (signed in)" : ""} — ${x.source}`)
      const skills = this.skills.list.map((x) => `${x.name} (${x.scope})`)
      return [
        `MCP servers (${servers.length}):`,
        ...(servers.length ? servers : ["- none"]),
        `Config files: ${this.mcp.files().map((f) => f.replace(this.cwd, ".")).join(", ")} (later wins)`,
        "",
        `Skills (${skills.length}): ${skills.join(", ") || "none"}`,
        "Skill folders: ~/.config/oah/skills (global), ./.oah/skills (project); also read: ./.claude/skills, ./.agents/skills, ~/.claude/skills, ~/.agents/skills",
      ].join("\n")
    },
    reloadMcp: async () => {
      const r = await this.mcp.reload()
      const lines = [...r.added, ...r.changed].map((n) => {
        const x = this.mcp.get(n)
        return `${n}: ${x ? mcpState(x) : "?"}${x?.state === "needs-auth" ? " — run oah_setup mcp_sign_in (or the user can pick it in /mcp)" : ""}`
      })
      if (r.removed.length) lines.push(`removed: ${r.removed.join(", ")}`)
      this.emit()
      return lines.join("\n") || "No MCP config changes."
    },
    reloadSkills: () => {
      this.skills.reload()
      this.emit()
    },
    signInMcp: (name, signal) => this.runMcpSignIn(name, signal),
  }

  /** /update: install the latest release over the running binary. */
  private async selfUpdate() {
    if (this.updating) return this.notify("Already updating…")
    this.updating = true
    try {
      const r = await update({ onStatus: (s) => this.notify(`${s[0]!.toUpperCase()}${s.slice(1)}…`) })
      if (r.status === "current") this.info(`oah ${r.version} is the latest version.`)
      else {
        this.updateAvailable = null
        this.info(`**Updated oah ${r.from} → ${r.to}.** Restart oah to use it — this conversation is saved: \`oah -c\` continues it.`)
      }
    } catch (e: any) {
      this.error(`Update failed: ${e?.message ?? e}`)
    } finally {
      this.updating = false
    }
  }

  /** The full ChatGPT sign-in link while a sign-in is waiting. */
  get loginUrl(): string | undefined {
    return this.login?.url
  }

  copyLoginUrl() {
    const url = this.loginUrl
    if (!url) return
    this.copy(url) ? this.notify("Copied sign-in link") : this.notify("Clipboard unavailable", true)
  }

  /** Answer the pending ask_user question ("" dismisses it). */
  answerQuestion(answer: string) {
    const q = this.question
    if (!q) return
    this.answers.push({ q: q.question, a: answer || "(dismissed)" })
    this.question = this.waitingQuestions.shift() ?? null
    this.activity = this.question ? "Waiting for your answer" : this.permission ? "Waiting for permission" : this.workingActivity()
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
    this.permission = this.waitingPermissions.shift() ?? null
    this.activity = this.permission ? "Waiting for permission" : this.question ? "Waiting for your answer" : this.workingActivity()
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
        return this.openPicker(
          "skills",
          `Skills · ${this.skills.list.length} — enter inserts /<name>; the model also loads them when relevant`,
          [...this.skills.list]
            .sort((a, b) => (a.scope === b.scope ? a.name.localeCompare(b.name) : a.scope === "project" ? -1 : 1))
            .map((s) => ({ label: s.name, detail: "", value: s.name, group: s.scope === "project" ? "Project" : "User", keywords: s.description })),
        )
      case "mcp":
        return this.mcpCommand(args)
      case "update":
        return void this.selfUpdate()
      case "import": {
        const valid = ["mcp", "skills", "sessions"] as const
        const words = args.toLowerCase().split(/[\s,]+/).filter(Boolean)
        if (words.some((w) => !(valid as readonly string[]).includes(w))) return this.notify("Usage: /import [mcp] [skills] [sessions]", true)
        const what = new Set((words.length ? words : valid) as (typeof valid)[number][])
        const r = importFromClaude({ cwd: this.cwd, what, model: this.settings.model, effort: this.settings.effort, mode: this.settings.mode })
        if (r.skillsChanged) this.skills.reload()
        this.info("**Imported from Claude Code**\n" + r.lines.join("\n"))
        if (r.mcpChanged)
          void this.mcp.reload().then(() => {
            const need = this.mcp.servers.filter((x) => x.state === "needs-auth").map((x) => x.name)
            if (need.length) this.info(`${need.join(", ")} need${need.length === 1 ? "s" : ""} authentication — sign in from /mcp`)
            this.emit(true)
          })
        return
      }
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
      this.stopGoalClock()
      this.session.goal = undefined
      return this.notify("Goal cleared")
    }
    this.stopGoalClock()
    this.session.goal = { text: args, turns: 0, done: false, ms: 0 }
    this.startGoalClock()
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
    this.fileVersions = new Map()
    this.turnCheckpoint = null
    this.compactedTokens = -1
    this.images.clear()
    this.imageCount = 0
    killAllShells()
    this.busy?.abort.abort()
    this.dropPrompts()
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
    this.dropPrompts()
    this.busy = null
    saveSession(this.session)
    this.session = s
    this.fileVersions = new Map()
    this.turnCheckpoint = null
    this.compactedTokens = -1
    if (s.model) this.settings.model = s.model
    this.settings.effort = s.effort
    this.settings.mode = s.mode
    this.info(`Resumed session ${s.id}`)
  }

  // ---- pickers ----------------------------------------------------------------

  openPicker(kind: PickerKind, title: string, items: PickItem[]) {
    this.picker = { kind, title, items, selected: Math.max(0, items.findIndex((i) => i.current)), filter: "", scroll: 0 }
    this.emit(true)
  }

  pickerVisible(): PickItem[] {
    const p = this.picker
    if (!p) return []
    const words = p.filter.toLowerCase().split(/\s+/).filter(Boolean)
    if (!words.length) return p.items.filter((i) => !i.searchOnly)
    return p.items.filter((i) => {
      const hay = `${i.label} ${i.detail} ${i.group ?? ""} ${i.keywords ?? ""}`.toLowerCase()
      return words.every((w) => hay.includes(w))
    })
  }

  /** Items only shown while searching, when the search box is empty. */
  pickerHidden(): number {
    const p = this.picker
    return p && !p.filter.trim() ? p.items.filter((i) => i.searchOnly).length : 0
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
      case "skills":
        return this.setInput(`/${item.value} `)
      case "mcp":
        return this.mcpPicked(item.value)
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
      keywords: `${m.provider} ${providerInfo[m.provider].label} ${PROVIDER_ALIASES[m.provider] ?? ""}`,
    })
    const byId = new Map(all.map((m) => [m.full, m]))
    const recent = [...new Set([this.settings.model, ...(this.settings.recentModels ?? [])])].filter((id) => byId.has(id)).slice(0, 5)
    // Unfiltered, show only the newest few per provider; typing searches everything.
    const shown = new Map<string, number>()
    const items: PickItem[] = [
      ...recent.map((id) => item(byId.get(id)!, "Recent")),
      ...all
        .filter((m) => !recent.includes(m.full))
        .map((m) => {
          const n = (shown.get(m.provider) ?? 0) + 1
          shown.set(m.provider, n)
          return { ...item(m, auth.label(this.authStatus, m.provider)), searchOnly: n > MODELS_PER_PROVIDER }
        }),
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
        this.activity = "Waiting for ChatGPT sign-in in your browser · c to copy link"
        this.emit(true)
        auth
          .beginChatGptLogin()
          .then((p) => {
            this.login = p
            const opened = auth.openBrowser(p.url)
            this.info(`**Continue with ChatGPT** — ${opened ? "your browser should open. If it doesn't, open this link:" : "open this link in a browser:"}\n${p.displayUrl}\n\nWaiting for sign-in… press **c** to copy the link, esc to cancel.`)
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
    this.busy?.abort.abort()
    this.dropPrompts()
    this.persistSettings()
    this.stopGoalClock()
    saveSession(this.session)
  }
}

/** Models listed per provider in /model before you start typing. */
const MODELS_PER_PROVIDER = 6

/** Other names people search providers by. */
const PROVIDER_ALIASES: Partial<Record<Provider, string>> = {
  chatgpt: "openai gpt plan subscription",
  openai: "gpt",
  anthropic: "claude",
  google: "gemini gemma",
  xai: "grok x.ai",
  kimi: "moonshot",
  "kimi-code": "moonshot kimi code plan",
  zai: "z.ai glm zhipu",
  "zai-coding": "z.ai zai glm zhipu coding plan",
  minimax: "mini max",
  "minimax-coding": "mini max coding plan",
  ollama: "local",
  lmstudio: "lm studio local",
  openrouter: "open router",
}

const EFFORTS_SET = new Set<string>(["minimal", "low", "medium", "high", "xhigh", "max"])

export function mcpState(x: McpServer): string {
  switch (x.state) {
    case "ready":
      return `ready · ${x.tools.length} tool${x.tools.length === 1 ? "" : "s"}`
    case "needs-auth":
      return "needs authentication"
    case "failed":
      return `failed: ${oneLine(x.error ?? "", 80)}`
    default:
      return "connecting…"
  }
}
