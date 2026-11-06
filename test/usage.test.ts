import { expect, test } from "bun:test"
import { addUsage, cacheHitPercent } from "../src/usage"

test("weighted cache rate and missing provider usage", () => {
  let t = addUsage(undefined, { input: 100, output: 10, inputTokenDetails: { noCacheTokens: 20, cacheReadTokens: 70, cacheWriteTokens: 10 } })
  t = addUsage(t, { input: 300, output: 20, inputTokenDetails: { noCacheTokens: 270, cacheReadTokens: 30, cacheWriteTokens: 0 } })
  t = addUsage(t, { input: 100, output: 0 })
  expect(t).toMatchObject({ input: 500, output: 30, cacheRead: 100, cacheWrite: 10, cacheReportedInput: 400, requests: 3 })
  expect(cacheHitPercent(t)).toBe(25)
  expect(cacheHitPercent(addUsage(undefined, { input: 100, output: 0 }))).toBeUndefined()
})
