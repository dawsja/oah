import { describe, expect, it } from "vitest"
import { emptyTranscript, reduce, waitingFor, type ApprovalRequest, type ChatMeta } from "./chat"
import { notification } from "./notify"
import { needsYou, waitingOf } from "./chat"

const approval = (kind: ApprovalRequest["kind"]): ApprovalRequest => ({ requestId: "r", itemId: null, kind, title: "Run tests", detail: null, diff: null, canRemember: false })
const meta = (id: string) => ({ id, title: id }) as ChatMeta

describe("what waits on the user", () => {
  it("reads the transcript's first card", () => {
    const t = reduce(emptyTranscript(), { type: "approval", request: approval("plan") })
    expect(waitingFor(t)).toBe("plan")
    const q = reduce(t, { type: "question", request: { requestId: "q", questions: [] } })
    expect(waitingFor(q)).toBe("question")
    expect(waitingFor(reduce(t, { type: "approvalResolved", requestId: "r", decision: "allow" }))).toBeNull()
  })

  it("prefers a loaded transcript to the app's list, which covers chats never opened here", () => {
    const s = {
      metas: [meta("phone"), meta("answered"), meta("idle")],
      // "answered": the list hasn't caught up yet, the live transcript has.
      transcripts: { answered: emptyTranscript() },
      waiting: { phone: "approval" as const, answered: "approval" as const },
    }
    expect(waitingOf(s, "phone")).toBe("approval")
    expect(waitingOf(s, "answered")).toBeNull()
    expect(needsYou(s)).toEqual(["phone"])
  })

  it("marks the notifications that wait, so they get an Answer button", () => {
    expect(notification({ type: "approval", request: approval("command") }, meta("c"))?.waits).toBe(true)
    expect(notification({ type: "turnFinished", turnId: "t", usage: null, costUsd: null, durationMs: null, interrupted: false }, meta("c"))?.waits).toBeUndefined()
  })
})
