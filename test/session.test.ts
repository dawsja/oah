import { afterAll, beforeEach, expect, test } from "bun:test"
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { listSessions, loadSession, newSession, saveSession, type SessionData } from "../src/session"

const prevHome = process.env.OAH_HOME
let home = ""

beforeEach(() => {
  if (home) rmSync(home, { recursive: true, force: true })
  home = mkdtempSync(join(tmpdir(), "oah-sessions-"))
  process.env.OAH_HOME = home
})

afterAll(() => {
  if (home) rmSync(home, { recursive: true, force: true })
  if (prevHome === undefined) delete process.env.OAH_HOME
  else process.env.OAH_HOME = prevHome
})

const sessionsDir = () => join(home, "sessions")
const metaPath = (id: string) => join(sessionsDir(), "meta", `${id}.json`)

function make(title: string, cwd = "/proj"): SessionData {
  const s = newSession("openai/gpt-5.6", "high", "build")
  s.title = title
  s.cwd = cwd
  s.messages = [{ role: "user", content: "hi" }]
  return s
}

test("saveSession writes a meta summary and listSessions returns it", () => {
  const s = make("first")
  saveSession(s)
  expect(existsSync(metaPath(s.id))).toBe(true)
  expect(listSessions()).toEqual([{ id: s.id, title: "first", cwd: "/proj", updated: s.updated }])
})

test("listing reads the meta file, not the full session", () => {
  const s = make("real title")
  saveSession(s)
  // A meta that still matches the session file's mtime/size is trusted as-is.
  const meta = JSON.parse(readFileSync(metaPath(s.id), "utf8"))
  writeFileSync(metaPath(s.id), JSON.stringify({ ...meta, title: "from meta" }))
  expect(listSessions()[0]!.title).toBe("from meta")
})

test("sessions without meta (older saves, /import) are parsed and backfilled", () => {
  const s = make("legacy")
  saveSession(s)
  rmSync(metaPath(s.id))
  expect(listSessions()[0]!.title).toBe("legacy")
  expect(existsSync(metaPath(s.id))).toBe(true)
})

test("a session file changed behind the meta's back is re-read", () => {
  const s = make("old")
  saveSession(s)
  const file = join(sessionsDir(), `${s.id}.json`)
  writeFileSync(file, JSON.stringify({ ...s, title: "renamed elsewhere" }))
  expect(statSync(file).size).toBeGreaterThan(0)
  expect(listSessions()[0]!.title).toBe("renamed elsewhere")
})

test("newest first, corrupt files skipped, prefix resume ignores the meta folder", () => {
  const a = make("a")
  saveSession(a)
  const b = make("b")
  saveSession(b)
  b.updated = a.updated + 1000
  writeFileSync(join(sessionsDir(), `${b.id}.json`), JSON.stringify(b))
  writeFileSync(join(sessionsDir(), "broken.json"), "{not json")
  expect(listSessions().map((x) => x.title)).toEqual(["b", "a"])
  expect(loadSession(a.id.slice(0, 6))?.title).toBe("a")
})
