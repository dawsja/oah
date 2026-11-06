import { afterEach, expect, setSystemTime, test } from "bun:test"
import { newSession } from "../src/session"
import { Store } from "../src/ui/store"
import { fmtMs } from "../src/ui/theme"

// The goal clock lives on Store's busy setter; skip the constructor (catalog fetch, MCP, …).
function bareStore(): Store {
  const s = Object.create(Store.prototype) as Store
  Object.assign(s, { _busy: null, goalRun: null, session: newSession("openai/gpt-5.6", "high", "build") })
  return s
}

const turn = () => ({ kind: "turn" as const, started: Date.now(), abort: new AbortController() })
const compact = () => ({ kind: "compact" as const, started: Date.now(), abort: new AbortController() })
let now = 1_000_000
const at = (ms: number) => setSystemTime(new Date((now = ms)))
const later = (ms: number) => at(now + ms)

afterEach(() => setSystemTime())

test("counts working time, not pauses", () => {
  at(1_000_000)
  const s = bareStore()
  s.session.goal = { text: "ship it", turns: 0, done: false, ms: 0 }
  s.busy = turn()
  later(30_000)
  expect(s.goalRunning).toBe(true)
  expect(s.goalElapsed()).toBe(30_000)
  // turn → compaction → next turn runs continuously
  s.busy = compact()
  later(10_000)
  s.busy = turn()
  later(5_000)
  s.busy = null
  expect(s.goalElapsed()).toBe(45_000)
  expect(s.session.goal.ms).toBe(45_000)
  // paused (esc / error): the clock stops
  later(3_600_000)
  expect(s.goalRunning).toBe(false)
  expect(s.goalElapsed()).toBe(45_000)
  // resumed
  s.busy = turn()
  later(15_000)
  expect(s.goalElapsed()).toBe(60_000)
})

test("sign-ins don't count and completion stops the clock", () => {
  at(1_000_000)
  const s = bareStore()
  s.session.goal = { text: "x", turns: 0, done: false }
  s.busy = { kind: "login", started: Date.now(), abort: new AbortController() }
  later(20_000)
  s.busy = null
  expect(s.goalElapsed()).toBe(0)
  s.busy = turn()
  later(7_000)
  s.session.goal.done = true // goal_complete mid-turn
  s.busy = null
  s.busy = turn()
  later(60_000)
  expect(s.goalElapsed()).toBe(7_000)
})

test("older sessions without a recorded time start from zero", () => {
  at(1_000_000)
  const s = bareStore()
  s.session.goal = { text: "legacy", turns: 12, done: false }
  expect(s.goalElapsed()).toBe(0)
  s.busy = turn()
  later(2_000)
  expect(s.goalElapsed()).toBe(2_000)
})

test("fmtMs covers hours", () => {
  expect(fmtMs(42_000)).toBe("42s")
  expect(fmtMs(12 * 60_000 + 5_000)).toBe("12m05s")
  expect(fmtMs(3_600_000 + 5 * 60_000)).toBe("1h05m")
})
