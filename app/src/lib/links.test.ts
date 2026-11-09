import { describe, expect, it } from "vitest"
import { linkTarget } from "./links"

describe("linkTarget", () => {
  it("sends web and mail links to the browser", () => {
    expect(linkTarget("https://example.com/a?b#c")).toEqual({ kind: "url", url: "https://example.com/a?b#c" })
    expect(linkTarget("mailto:me@example.com")).toEqual({ kind: "url", url: "mailto:me@example.com" })
  })
  it("opens project files, with a line when one is given", () => {
    expect(linkTarget("src/app.ts")).toEqual({ kind: "file", path: "src/app.ts" })
    expect(linkTarget("./src/app.ts#L12")).toEqual({ kind: "file", path: "src/app.ts", line: 12 })
    expect(linkTarget("src/app.ts:40")).toEqual({ kind: "file", path: "src/app.ts", line: 40 })
    expect(linkTarget("docs/my%20notes.md")).toEqual({ kind: "file", path: "docs/my notes.md" })
  })
  it("does nothing with anything else", () => {
    for (const h of ["javascript:alert(1)", "file:///etc/passwd", "data:text/html,x", "//evil.example", "#top", "", "../secret", "/etc/hosts", undefined]) {
      expect(linkTarget(h)).toEqual({ kind: "none" })
    }
  })
})
