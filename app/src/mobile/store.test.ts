import { beforeEach, describe, expect, it, vi } from "vitest"

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock("@tauri-apps/api/core", () => ({ invoke }))

import { useMobile } from "./store"
import type { AgentInfo } from "./rpc"

const meta = (effort: string | null) => ({ id: "chat-new", projectId: "p1", agent: "claude", model: "opus", effort, title: "New chat" })

beforeEach(() => {
  invoke.mockReset()
  invoke.mockImplementation(async (_cmd: string, args: { method: string; params: Record<string, unknown> }) => {
    switch (args.method) {
      case "chat.create": return meta(null)
      case "chat.setEffort": return meta(args.params.effort as string)
      case "chat.open": return { events: [], busy: false, live: false }
      default: return undefined
    }
  })
})

describe("a new chat from the phone", () => {
  const agent = { kind: "claude", name: "Claude Code" } as AgentInfo

  it("starts with the effort picked before the first message", async () => {
    const id = await useMobile.getState().send(null, "hi", [], { projectId: "p1", agent, model: "opus", effort: "high" })
    expect(id).toBe("chat-new")
    const methods = invoke.mock.calls.map(([, a]) => a.method)
    expect(methods).toEqual(["chat.create", "chat.setEffort", "chat.open", "chat.send"])
    expect(invoke.mock.calls[1][1].params).toEqual({ chatId: "chat-new", effort: "high" })
    expect(useMobile.getState().metas.find((m) => m.id === "chat-new")?.effort).toBe("high")
  })

  it("leaves the agent's default effort alone when none was picked", async () => {
    await useMobile.getState().send(null, "hi", [], { projectId: "p1", agent, model: "opus" })
    expect(invoke.mock.calls.map(([, a]) => a.method)).not.toContain("chat.setEffort")
  })

  it("sends its first message with the permission mode picked before it, and keeps it", async () => {
    const id = await useMobile.getState().send(null, "hi", [], { projectId: "p1", agent, permission: "plan" })
    const send = invoke.mock.calls.find(([, a]) => a.method === "chat.send")!
    expect(send[1].params.permission).toBe("plan")
    expect(useMobile.getState().permission[id!]).toBe("plan")
  })

  it("leaves the agent's default mode to the desktop when none was picked", async () => {
    useMobile.setState({ permission: {} })
    await useMobile.getState().send(null, "hi", [], { projectId: "p1", agent })
    expect(invoke.mock.calls.find(([, a]) => a.method === "chat.send")![1].params.permission).toBeNull()
  })
})
