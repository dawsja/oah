import { expect, test } from "bun:test"
import { Verification, isVerificationCommand } from "../src/verification"
test("only checks after the latest mutation count, and repaired failures clear", () => {
  const v = new Verification()
  v.checked("bun test", true, 0)
  v.wrote()
  expect(v.pending).toBe(true)
  v.checked("bun test", true, 0)
  expect(v.pending).toBe(true)
  v.checked("bun test", false, 1)
  expect(v.state.status).toBe("failed")
  v.checked("bun test", true, 1)
  expect(v.pending).toBe(false)
  expect(new Verification(v.state).state).toEqual(v.state)
})
test("verification commands are actual invocations, not incidental text", () => {
  for (const cmd of ["bun test", "bun run typecheck", "npm test", "cargo check", "git diff --check", "pytest -q"]) expect(isVerificationCommand(cmd)).toBe(true)
  for (const cmd of ["ls", "echo bun test", "bun test; echo bypass", "bun test || true", "bun test &", "cat test.ts"]) expect(isVerificationCommand(cmd)).toBe(false)
})

test("help, generators and modifying check modes do not count as verification", () => {
  for (const cmd of ["tsc --init", "ruff format src", "eslint --fix src", "bun test --help", "pytest --version", "bun test -u", "bun test --update-snapshots"]) expect(isVerificationCommand(cmd)).toBe(false)
  expect(isVerificationCommand("ruff check src")).toBe(true)
})
test("a different passing check cannot mask a failure on the current revision", () => {
  const v = new Verification()
  v.wrote()
  v.checked("bun test", false, 1)
  v.checked("git diff --check", true, 1)
  expect(v.state.status).toBe("failed")
  v.wrote()
  v.checked("bun test", true, 2)
  expect(v.state.status).toBe("checks-passed")
})

test("AND-only check chains and stderr merging count; mixed chains cannot mask mutations", () => {
  expect(isVerificationCommand("bun test && bun run typecheck")).toBe(true)
  expect(isVerificationCommand("bun test 2>&1")).toBe(true)
  for (const cmd of ["bun test && rm file", "bun test || true", "bun test && bun -e 'writeFileSync()'", "bun test && bun run lint --fix", "bun test &&"]) expect(isVerificationCommand(cmd)).toBe(false)
})
