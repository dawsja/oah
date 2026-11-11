// Chats open in their agent's own CLI: the CLI runs in a terminal on the chat's session, in the
// chat's place, until the user goes back to the chat or leaves the CLI. The app decides who has
// a chat's session (its driver or the CLI); this is the window's side: the xterm.js terminals,
// kept outside React so a chat that's not on screen keeps its CLI running.

import { toast } from "sonner"
import { create } from "zustand"
import { cli as ipc, onCliTaken } from "@/lib/ipc"
import { runningTasks } from "@/lib/chat"
import { AGENT_LABEL, type AgentKind } from "@/lib/types"
import { disposeXterm, fitXterm, loadXterm, makeXterm, moveXterm, type Xterm } from "@/lib/xterm"
import { useApp } from "./app"
import { useChats } from "./chats"
import { useGit } from "./git"

/** A chat's CLI as the window sees it. `exited`: it ended by itself with an error (its last
 *  output stays to read); `failed`: it never started. */
export type CliSession = { status: "starting" | "running" | "exited" | "failed"; code?: number | null; error?: string }

type CliStore = {
  /** By chat id. A chat in here shows its CLI instead of its transcript and composer. */
  sessions: Record<string, CliSession>
}

export const useCli = create<CliStore>(() => ({ sessions: {} }))

/** Agents whose CLI starts a conversation under an id oah picks, so a chat can begin in the CLI
 *  (as `presets_session` in crates/oah-agents/src/cli.rs). The others need a first message. */
export const STARTS_IN_CLI: AgentKind[] = ["claude", "grok"]

type Instance = Xterm & {
  /** Input from before the CLI is up: answers to Windows' pseudo console asking where the
   *  cursor is, which it waits for. */
  queued: string[]
  started: boolean
  /** Bumped per start, so a late word from an earlier one is ignored. */
  run: number
  disposed: boolean
}

const instances = new Map<string, Instance>()
const hosts = new Map<string, HTMLElement>()
const creating = new Set<string>()
/** Whether each chat's next start may stop an agent working in it (the user said so). */
const forced = new Set<string>()
/** Chats made only to open the CLI in: one left with nothing said isn't kept. */
const fresh = new Set<string>()

function patch(chatId: string, p: CliSession | null) {
  useCli.setState((s) => {
    const { [chatId]: _, ...rest } = s.sessions
    return { sessions: p ? { ...rest, [chatId]: p } : rest }
  })
}

/** Opens the chat in its agent's CLI, where the chat is on screen. An agent working in it is
 *  asked about first: the CLI takes the session over, which stops it. */
export async function enterCli(chatId: string) {
  const chats = useChats.getState()
  const meta = chats.metas.find((m) => m.id === chatId)
  if (!meta || useCli.getState().sessions[chatId]) return
  const t = chats.transcripts[chatId]
  const working = t ? t.running || runningTasks(t).length > 0 : chats.busy.includes(chatId) || chats.background.includes(chatId)
  if (working && !(await confirmStop(meta.agent))) return
  if (working) forced.add(chatId)
  // What's queued behind the turn stays queued: nothing is sent while the CLI has the chat.
  patch(chatId, { status: "starting" })
}

function confirmStop(agent: AgentKind): Promise<boolean> {
  return import("@/components/Prompt").then(({ confirmAction }) =>
    confirmAction({
      title: `${AGENT_LABEL[agent]} is working in this chat`,
      description: "Opening the CLI stops it: the CLI takes the conversation over from where it is now. What it has written so far is kept.",
      confirm: "Stop and open the CLI",
      danger: true,
    }),
  )
}

/** Back to the chat: the CLI ends, and the next message continues from what was said in it. */
export async function leaveCli(chatId: string) {
  const inst = instances.get(chatId)
  if (inst) inst.disposed = true
  await ipc.close(chatId).catch(() => undefined)
  ended(chatId)
}

/** The chat's CLI is over: its terminal goes, and the chat shows what the app now knows. */
function ended(chatId: string) {
  const inst = instances.get(chatId)
  if (inst) {
    inst.disposed = true
    instances.delete(chatId)
    disposeXterm(inst)
  }
  hosts.delete(chatId)
  forced.delete(chatId)
  patch(chatId, null)
  void settle(chatId)
}

async function settle(chatId: string) {
  const chats = useChats.getState()
  await chats.refresh()
  const meta = useChats.getState().metas.find((m) => m.id === chatId)
  if (!meta) return
  // Opened only for the CLI and nothing was said in it: no empty chat is left behind.
  if (fresh.delete(chatId) && !meta.agentSessionId && !useChats.getState().transcripts[chatId]?.items.length) return void chats.remove(chatId)
  // The agent may have changed files there.
  const project = useApp.getState().settings?.projects.find((p) => p.id === meta.projectId)
  if (project) void useGit.getState().refresh({ projectId: project.id, cwd: meta.cwd && meta.cwd !== project.path ? meta.cwd : null })
}

/** A new chat that begins in the agent's CLI (for the agents in `STARTS_IN_CLI`). */
export function startsInCli(chatId: string) {
  fresh.add(chatId)
  return enterCli(chatId)
}

/** Puts the chat's CLI in `host`, starting it the first time. */
export function attachCli(chatId: string, host: HTMLElement) {
  hosts.set(chatId, host)
  const inst = instances.get(chatId)
  if (inst) {
    moveXterm(inst, host)
  } else if (!creating.has(chatId)) {
    creating.add(chatId)
    void build(chatId).finally(() => creating.delete(chatId))
  }
}

/** The host is going away (the CLI stays, to be attached again). */
export function detachCli(chatId: string, host: HTMLElement) {
  if (hosts.get(chatId) === host) hosts.delete(chatId)
}

export function fitCli(chatId: string) {
  const inst = instances.get(chatId)
  if (inst) fitXterm(inst)
}

export function focusCli(chatId: string) {
  instances.get(chatId)?.term.focus()
}

/** Types `text` into the chat's CLI as a paste (a file mention, terminal output to ask about);
 *  false when the chat isn't in its CLI. */
export function pasteIntoCli(chatId: string, text: string): boolean {
  const inst = instances.get(chatId)
  if (!inst || useCli.getState().sessions[chatId]?.status !== "running") return false
  // A bracketed paste: a block with newlines in it lands in the prompt, it isn't sent.
  void ipc.write(chatId, `\x1b[200~${text.replace(/\x1b/g, "")}\x1b[201~`).catch(() => undefined)
  inst.term.focus()
  return true
}

async function build(chatId: string) {
  const modules = await loadXterm()
  const host = hosts.get(chatId)
  if (!host || !useCli.getState().sessions[chatId]) return
  const inst: Instance = Object.assign(makeXterm(modules, host, useApp.getState().info?.platform === "windows"), { queued: [], started: false, run: 0, disposed: false })
  instances.set(chatId, inst)
  inst.term.onData((data) => {
    const status = useCli.getState().sessions[chatId]?.status
    if (status === "exited" || status === "failed") {
      if (data === "\r") void start(chatId, inst)
      return
    }
    if (inst.started) void ipc.write(chatId, data).catch(() => undefined)
    else inst.queued.push(data)
  })
  inst.term.onResize(({ cols, rows }) => {
    if (inst.started) void ipc.resize(chatId, cols, rows).catch(() => undefined)
  })
  inst.term.focus()
  await start(chatId, inst)
}

/** Starts (or starts again) the chat's CLI in its terminal. */
export async function restartCli(chatId: string) {
  const inst = instances.get(chatId)
  if (inst) await start(chatId, inst)
}

async function start(chatId: string, inst: Instance) {
  const run = ++inst.run
  inst.started = false
  inst.queued = []
  patch(chatId, { status: "starting" })
  const current = () => inst.run === run && !inst.disposed
  const open = (force: boolean) =>
    ipc.open(chatId, inst.term.cols, inst.term.rows, force, (e) => {
      if (!current()) return
      if (e.kind === "data") return inst.term.write(e.data)
      inst.started = false
      // Left the usual way (`/exit`, Ctrl+D): straight back to the chat. An error stays to read,
      // unless the chat went with it (deleted, or its project removed).
      const meta = useChats.getState().metas.find((m) => m.id === chatId)
      const there = meta && useApp.getState().settings?.projects.some((p) => p.id === meta.projectId)
      if (e.code === 0 || !there) return ended(chatId)
      patch(chatId, { status: "exited", code: e.code })
      void useChats.getState().refresh()
    })
  try {
    await open(forced.delete(chatId)).catch(async (e) => {
      // An agent started working in the chat since the window last heard (a phone, MCP).
      const meta = useChats.getState().metas.find((m) => m.id === chatId)
      if (!String(e).includes("is working in this chat") || !meta || !(await confirmStop(meta.agent))) throw e
      return open(true)
    })
    if (!current()) return
    // An exit can beat the reply; then there's no CLI to talk to.
    if (useCli.getState().sessions[chatId]?.status !== "starting") return
    inst.started = true
    if (inst.queued.length) void ipc.write(chatId, inst.queued.splice(0).join("")).catch(() => undefined)
    patch(chatId, { status: "running" })
    void useChats.getState().refresh()
  } catch (e) {
    if (!current()) return
    patch(chatId, { status: "failed", error: String(e) })
    // A chat made just for this isn't left behind, empty; the reason is said where it was.
    if (fresh.has(chatId)) {
      toast.error(String(e))
      ended(chatId)
    }
  }
}

/** Once, as the window starts. A CLI the app still runs from before a reload has no terminal
 *  here any more, so its chat goes back to being a chat; and from now on a chat taken back from
 *  its CLI elsewhere (a phone, MCP) shows as a chat here too. */
export async function initCli() {
  await onCliTaken((chatId) => {
    if (!useCli.getState().sessions[chatId]) return
    const meta = useChats.getState().metas.find((m) => m.id === chatId)
    toast.info(`${meta?.title ? `"${meta.title}"` : "A chat"} was taken back from its CLI`, { description: "From a phone or over MCP: the CLI ended, and the chat carries on from what was said in it." })
    ended(chatId)
  })
  for (const id of useChats.getState().cli) if (!useCli.getState().sessions[id]) await ipc.close(id).catch(() => undefined)
  if (useChats.getState().cli.length) void useChats.getState().refresh()
}
