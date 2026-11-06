// The agent loop on the AI SDK: one user turn = `streamText` with tools and a
// multi-step stop condition. Permission prompts happen inside each tool's
// `execute`, so the model only sees the result ("denied" included).
import { APICallError, generateText, jsonSchema, stepCountIs, streamText, tool, type ModelMessage, type ToolSet } from "ai"
import { USAGE_URL } from "./auth"
import { shortPath, splitModel, type Effort, type Mode, type Permission, type Settings } from "./config"
import type { Mcp } from "./mcp"
import { projectRoot } from "./memory"
import { languageModel, providerOptions } from "./providers"
import { oneLine, permissionPreview, risk, title, TOOLS, type Outcome, type Risk, type Todo, type ToolCtx } from "./tools"
import { existsSync } from "node:fs"
import { join } from "node:path"

export type Reply = "once" | "always" | "deny"

export interface Hooks {
  text(delta: string): void
  reasoning(delta: string): void
  reasoningEnd(): void
  toolStart(callId: string, name: string, title: string): void
  toolEnd(callId: string, outcome: Outcome, denied: boolean): void
  ask(req: { title: string; kind: string; preview: string[] }): Promise<Reply>
  todos(items: Todo[]): void
  goalComplete(summary: string): void
  usage(u: { input: number; output: number }): void
  /** Live status for a running tool (sub-agents). */
  toolProgress(callId: string, summary: string, detail: string[]): void
  /** Ask the user a question; resolves with their answer text. */
  question(q: Question): Promise<string>
}

export interface Question {
  question: string
  options: { label: string; description?: string }[]
  multiSelect: boolean
}

/** Tools a research sub-agent may use (read-only). */
const SUBAGENT_TOOLS = new Set(["read_file", "ls", "find", "grep", "web_fetch", "bash"])

export interface TurnOptions {
  model: string
  effort: Effort
  mode: Mode
  permission: Permission
  instructions: string
  messages: ModelMessage[]
  settings: Settings
  ctx: Omit<ToolCtx, "signal" | "mode" | "onTodos" | "onGoalComplete">
  mcp: Mcp
  grants: Set<string>
  goalActive: boolean
  signal: AbortSignal
  hooks: Hooks
  /** Restrict built-in tools to this set (sub-agents); also disables task/ask_user/MCP. */
  only?: Set<string>
}

export interface TurnResult {
  messages: ModelMessage[]
  error?: string
  aborted: boolean
}

const kindLabel: Record<Risk, string> = {
  internal: "internal",
  read: "read",
  "read-outside": "read outside the workspace",
  write: "edit files",
  "safe-exec": "run command",
  exec: "run command",
  external: "external tool",
}

export function needsPermission(p: Permission, r: Risk): boolean {
  if (p === "allow") return false
  if (p === "ask") return r !== "internal"
  return r === "read-outside" || r === "write" || r === "exec" || r === "external"
}

export function grantKey(name: string, args: any): string {
  if (name === "write_file" || name === "edit_file") return "write"
  if (name === "bash") return `bash:${String(args?.command ?? "").trim().split(/\s+/)[0]}`
  if (["read_file", "ls", "find", "grep"].includes(name)) return "read"
  return name
}

function mcpText(result: any): { text: string; isError: boolean } {
  if (typeof result === "string") return { text: result, isError: false }
  const parts: string[] = []
  for (const c of result?.content ?? []) {
    if (c.type === "text") parts.push(c.text)
    else if (c.type === "image") parts.push(`[image ${c.mimeType ?? ""}]`)
    else if (c.type === "resource") parts.push(c.resource?.text ?? `[resource ${c.resource?.uri}]`)
    else parts.push(JSON.stringify(c))
  }
  if (!parts.length && result?.structuredContent) parts.push(JSON.stringify(result.structuredContent))
  return { text: parts.join("\n") || "(no output)", isError: !!result?.isError }
}

export function buildTools(o: TurnOptions): ToolSet {
  const ctx: ToolCtx = { ...o.ctx, signal: o.signal, mode: o.mode, onTodos: o.hooks.todos, onGoalComplete: o.hooks.goalComplete }

  const gated = async (name: string, args: any, callId: string, run: () => Promise<Outcome>): Promise<string> => {
    o.hooks.toolStart(callId, name, title(name, args, ctx.cwd))
    const r = risk(name, args, ctx.cwd, o.settings.allowedCommands)
    const key = grantKey(name, args)
    if (needsPermission(o.permission, r) && !o.grants.has(key)) {
      const reply = await Promise.race([
        o.hooks.ask({ title: title(name, args, ctx.cwd), kind: kindLabel[r], preview: permissionPreview(name, args) }),
        new Promise<Reply>((res) => o.signal.addEventListener("abort", () => res("deny"), { once: true })),
      ])
      if (reply === "deny") {
        const out = { output: "The user denied this tool call. Do not retry it; ask how to proceed if needed.", summary: "denied", detail: [], ok: false }
        o.hooks.toolEnd(callId, out, true)
        return out.output
      }
      if (reply === "always") o.grants.add(key)
    }
    let outcome: Outcome
    try {
      outcome = await run()
    } catch (e: any) {
      outcome = { output: `Error: ${e?.message ?? e}`, summary: e?.message ?? String(e), detail: [], ok: false }
    }
    o.hooks.toolEnd(callId, outcome, false)
    return outcome.output
  }

  const tools: ToolSet = {}
  const hasSkills = o.ctx.skills.list.length > 0
  const openai = splitModel(o.model).provider !== "anthropic"
  for (const def of TOOLS) {
    if (o.only && !o.only.has(def.name)) continue
    if (def.buildOnly && o.mode !== "build") continue
    if (def.openaiOnly && !openai) continue
    if (def.name === "goal_complete" && !o.goalActive) continue
    if (def.name === "load_skill" && !hasSkills) continue
    const description =
      def.name === "bash" && o.mode === "plan" ? "Run a READ-ONLY shell command (plan mode): inspection only, e.g. git log, ls, cat. Commands that modify state are refused." : def.description
    tools[def.name] = tool({
      description,
      inputSchema: jsonSchema(def.parameters),
      execute: (args: any, { toolCallId }) => gated(def.name, args, toolCallId, async () => def.run(args, ctx)),
    })
  }
  if (o.only) return tools

  tools.task = tool({
    description:
      "Delegate a self-contained research question to a sub-agent with a fresh context and read-only tools (read_file, ls, find, grep, web_fetch, read-only bash). Use it to explore a codebase, trace how something works, or gather facts without filling your own context. Give a complete, specific prompt; it can't see this conversation. It returns a concise report. Launch several at once for independent questions.",
    inputSchema: jsonSchema({
      type: "object",
      properties: {
        description: { type: "string", description: "3-6 word label shown to the user" },
        prompt: { type: "string", description: "Everything the sub-agent needs: the question, relevant paths, what to report" },
      },
      required: ["description", "prompt"],
      additionalProperties: false,
    }),
    execute: (args: any, { toolCallId }) =>
      gated("task", args, toolCallId, async () => {
        const report = await runSubagent(o, String(args.prompt ?? ""), toolCallId)
        return report
      }),
  })

  tools.ask_user = tool({
    description:
      "Ask the user a question and wait for the answer. Use it whenever requirements are ambiguous, there are several reasonable approaches, or a decision is the user's to make (naming, scope, trade-offs, destructive steps) — ask instead of assuming. Offer 2-5 concrete options when you can; the user can always type their own answer. Ask one focused question at a time.",
    inputSchema: jsonSchema({
      type: "object",
      properties: {
        question: { type: "string", description: "The question, self-contained" },
        options: {
          type: "array",
          description: "Suggested answers (2-5). Put the recommended one first and say so in its description.",
          items: {
            type: "object",
            properties: { label: { type: "string" }, description: { type: "string" } },
            required: ["label"],
            additionalProperties: false,
          },
        },
        multi_select: { type: "boolean", description: "Allow choosing several options" },
      },
      required: ["question"],
      additionalProperties: false,
    }),
    execute: (args: any, { toolCallId }) =>
      gated("ask_user", args, toolCallId, async () => {
        const q: Question = {
          question: String(args.question ?? ""),
          options: (Array.isArray(args.options) ? args.options : []).slice(0, 8).map((x: any) => ({ label: String(x?.label ?? x), description: x?.description ? String(x.description) : undefined })),
          multiSelect: !!args.multi_select,
        }
        const answer = await Promise.race([
          o.hooks.question(q),
          new Promise<string>((res) => o.signal.addEventListener("abort", () => res(""), { once: true })),
        ])
        if (!answer.trim()) return { output: "The user dismissed the question without answering. Don't assume — ask again later or proceed only with what is certain.", summary: "dismissed", detail: [], ok: false }
        return { output: `The user answered: ${answer}`, summary: `→ ${oneLine(answer, 100)}`, detail: [], ok: true }
      }),
  })

  for (const [name, t] of Object.entries(o.mcp.tools())) {
    const exec = (t as any).execute
    if (!exec) continue
    tools[name] = {
      ...t,
      execute: (args: any, opts: any) =>
        gated(name, args, opts.toolCallId, async () => {
          const { text, isError } = mcpText(await exec(args, opts))
          const lines = text.split("\n")
          return { output: text, summary: isError ? "error" : `${lines.length} lines`, detail: lines.slice(0, 4), ok: !isError }
        }),
    } as any
  }
  return tools
}

/** Run a read-only research sub-agent and return its final report. */
async function runSubagent(parent: TurnOptions, prompt: string, callId: string): Promise<Outcome> {
  const started = Date.now()
  const steps: string[] = []
  let calls = 0
  const progress = () => {
    const secs = Math.round((Date.now() - started) / 1000)
    parent.hooks.toolProgress(callId, `${calls} tool call${calls === 1 ? "" : "s"} · ${secs}s`, steps.slice(-4))
  }
  const sub: TurnOptions = {
    ...parent,
    mode: "plan",
    goalActive: false,
    only: SUBAGENT_TOOLS,
    instructions: `You are a research sub-agent for nur, a coding agent. Another agent delegated one question to you; it can't see your work, only your final message.
- Investigate with your read-only tools (read_file, ls, find, grep, web_fetch, and bash for read-only commands). You cannot modify anything.
- Be efficient: search before reading, read only what you need, run independent lookups in parallel.
- Finish with a concise, specific report that answers the question: key findings, file paths with line numbers, short code excerpts where useful, and anything you couldn't determine.

Working directory: ${parent.ctx.cwd}
Date: ${new Date().toISOString().slice(0, 10)}`,
    hooks: {
      ...parent.hooks,
      text() {},
      reasoning() {},
      reasoningEnd() {},
      todos() {},
      goalComplete() {},
      usage() {},
      toolStart(_id, _name, title) {
        calls++
        steps.push(`◌ ${title}`)
        progress()
      },
      toolEnd(_id, outcome) {
        const i = steps.findLastIndex((x) => x.startsWith("◌ "))
        if (i >= 0) steps[i] = `${outcome.ok ? "✓" : "✗"} ${steps[i]!.slice(2)}`
        progress()
      },
      toolProgress() {},
      question: async () => "",
    },
  }
  progress()
  const model = await languageModel(parent.model, parent.settings)
  const result = streamText({
    model,
    system: sub.instructions,
    messages: [{ role: "user", content: prompt }],
    tools: buildTools(sub),
    stopWhen: stepCountIs(60),
    providerOptions: providerOptions(parent.model, parent.effort),
    abortSignal: parent.signal,
    maxRetries: 3,
  })
  let text = ""
  for await (const part of result.fullStream) {
    if (part.type === "text-delta") text += part.text
    else if (part.type === "finish-step") text = text ? text + "\n" : text
    else if (part.type === "error") throw new Error(describeError(part.error))
  }
  // Keep the final answer: the last step's text.
  const steps2 = await result.steps
  const final = (steps2.at(-1)?.text ?? text).trim() || text.trim()
  const secs = Math.round((Date.now() - started) / 1000)
  return {
    output: final || "(the sub-agent returned no report)",
    summary: `${calls} tool call${calls === 1 ? "" : "s"} · ${secs}s · report ${final.split("\n").length} lines`,
    detail: final.split("\n").filter((l) => l.trim()).slice(0, 4).map((l) => `│ ${oneLine(l, 160)}`),
    ok: !!final,
  }
}

/** Turn provider errors (including ChatGPT plan codes) into readable text. */
export function describeError(e: unknown): string {
  if (APICallError.isInstance(e)) {
    let code = ""
    let message = e.message
    try {
      const body = JSON.parse(e.responseBody ?? "{}")
      const err = body.error ?? body
      code = err.code ?? err.type ?? ""
      message = err.message ?? body.detail ?? message
    } catch {}
    switch (code) {
      case "subscription_sharing_usage_limit_exceeded":
        return `You've reached your ChatGPT plan's usage limit. Manage usage: ${USAGE_URL}`
      case "subscription_sharing_user_not_eligible":
        return "This ChatGPT account isn't eligible to use its plan in nur. Add an API key with /auth."
      case "subscription_sharing_invalid_user":
        return "Your ChatGPT session is no longer valid — run /auth to sign in again."
      case "subscription_sharing_usage_unavailable":
      case "subscription_sharing_user_unavailable":
        return "ChatGPT plan usage is temporarily unavailable. Try again shortly."
      case "chatpass_v2_scope_not_authorized":
      case "chatpass_v2_invalid_authorization_context":
        return "ChatGPT plan permission is missing — run /auth to grant it."
      case "overloaded_error":
        return "The model is overloaded right now. Try again shortly."
    }
    if (e.statusCode === 401) return `Unauthorized — check credentials with /auth. ${message}`
    return message
  }
  if (e instanceof Error) return e.message
  return String(e)
}

export async function runTurn(o: TurnOptions): Promise<TurnResult> {
  const collected: ModelMessage[] = []
  let error: string | undefined
  let aborted = false
  try {
    const model = await languageModel(o.model, o.settings)
    const result = streamText({
      model,
      system: o.instructions,
      messages: o.messages,
      tools: buildTools(o),
      stopWhen: stepCountIs(500),
      providerOptions: providerOptions(o.model, o.effort),
      abortSignal: o.signal,
      maxRetries: 3,
      onStepFinish(step: any) {
        collected.push(...(step.response?.messages ?? []))
      },
    })
    let inReasoning = false
    for await (const part of result.fullStream) {
      switch (part.type) {
        case "text-delta":
          if (inReasoning) {
            inReasoning = false
            o.hooks.reasoningEnd()
          }
          o.hooks.text(part.text)
          break
        case "reasoning-delta":
          inReasoning = true
          o.hooks.reasoning(part.text)
          break
        case "reasoning-end":
          if (inReasoning) o.hooks.reasoningEnd()
          inReasoning = false
          break
        case "finish-step":
          o.hooks.usage({ input: part.usage.inputTokens ?? 0, output: part.usage.outputTokens ?? 0 })
          break
        case "error":
          error = describeError(part.error)
          break
        case "abort":
          aborted = true
          break
      }
    }
  } catch (e) {
    if (o.signal.aborted) aborted = true
    else error = describeError(e)
  }
  return { messages: collected, error, aborted: aborted || o.signal.aborted }
}

/** Summarize the conversation so it can continue in a fresh context. */
export async function compact(o: { model: string; effort: Effort; settings: Settings; instructions: string; messages: ModelMessage[]; focus: string; signal: AbortSignal }): Promise<string> {
  let ask =
    "Your context window is being compacted. Write a detailed handoff summary of the conversation so far so you can continue seamlessly. " +
    "Include: the user's goals and constraints; key decisions; files read or changed (with paths) and what changed; commands run and important results; " +
    "errors and fixes; the current state; and concrete next steps. Do not call tools."
  if (o.focus.trim()) ask += `\nPay special attention to: ${o.focus.trim()}`
  const model = await languageModel(o.model, o.settings)
  const { text } = await generateText({
    model,
    system: o.instructions,
    messages: [...o.messages, { role: "user", content: ask }],
    providerOptions: providerOptions(o.model, "low"),
    abortSignal: o.signal,
    maxRetries: 3,
  })
  return text
}

export function systemPrompt(o: { cwd: string; mode: Mode; goal?: string; skills: string; memory: string; mcp: string[] }): string {
  const git = existsSync(join(projectRoot(o.cwd), ".git"))
  let s = `You are nur, an expert software engineering agent running in the user's terminal. You help with coding tasks by reading and editing files and running commands in the workspace.

# Working style
- Be concise and direct. Use GitHub-flavored Markdown; keep answers short unless detail is requested.
- Explore before changing things: read relevant files, search with grep/glob, and follow existing conventions.
- Make focused, minimal changes. Prefer edit_file over rewriting whole files.
- After changes, verify (build, tests, linters) when possible and report results honestly.
- For multi-step work, keep a todo list with todo_write and update it as you go.
- Never invent file contents or command output. If something fails, investigate the cause.
- Don't run destructive commands (rm -rf, git reset --hard, force pushes) unless the user explicitly asks.
- When the user states a durable preference or fact worth keeping, save it with the memory tool.

# Ask, don't assume
- The user prefers being asked over having you guess. When requirements are ambiguous, several approaches are reasonable, or a choice is theirs to make (names, scope, trade-offs, anything destructive or hard to undo), call ask_user with concrete options — before doing the work, not after.
- Don't ask about things you can find out yourself by reading the code; investigate first, then ask a focused question.

# Tools
- Search with find (files by name/type/age) and grep (contents) before reading files; use ls to look around.
- Use task to delegate self-contained research to sub-agents so your own context stays focused; run several in parallel for independent questions.
- Start servers, watchers and other long-running commands with bash background=true, then check them with bash_output and stop them with bash_kill when done.
- Batch several changes to one file into a single edit_file call with \`edits\`.

# Environment
- Working directory: ${o.cwd} (${shortPath(o.cwd)})
- Git repository: ${git ? "yes" : "no"}
- Platform: ${process.platform} (${process.arch})
- Date: ${new Date().toISOString().slice(0, 10)}
`
  s +=
    o.mode === "plan"
      ? "\n# Mode: PLAN\nYou are in plan mode. Do NOT modify files or run commands that change state. Investigate with read-only tools, then present a clear, numbered implementation plan (files to change, approach, risks, verification). Ask clarifying questions if requirements are ambiguous. The user will switch to build mode to execute.\n"
      : "\n# Mode: BUILD\nYou may edit files and run commands, subject to the user's permission settings.\n"
  if (o.goal)
    s += `\n# Active goal\nThe user set a goal and wants you to keep working autonomously until it is completely done:\n\n${o.goal}\n\nWork in small verified steps. Don't stop for routine confirmation, but do use ask_user when a decision genuinely needs the user. When — and only when — the goal is fully achieved and verified, call goal_complete with a summary. If you end a turn without calling goal_complete you will be asked to continue.\n`
  if (o.mcp.length) s += `\n# MCP\nConnected MCP servers: ${o.mcp.join(", ")}. Their tools are prefixed mcp__<server>__.\n`
  if (o.skills) s += `\n${o.skills}`
  if (o.memory) s += `\n${o.memory}`
  return s
}

export const userMessage = (text: string): ModelMessage => ({ role: "user", content: text })
