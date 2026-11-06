import { describe, expect, test } from "bun:test"
import { compareVersions, target } from "../src/update"

describe("update", () => {
  test("compares versions numerically", () => {
    expect(compareVersions("0.10.0", "0.9.3")).toBe(1)
    expect(compareVersions("v1.2.0", "1.2")).toBe(0)
    expect(compareVersions("1.2.3", "1.2.4")).toBe(-1)
    expect(compareVersions("2.0.0", "10.0.0")).toBe(-1)
  })
  test("names this platform like the release archives", () => {
    expect(target()).toMatch(/^(linux|darwin|windows)-(x64|arm64)$/)
  })
})
