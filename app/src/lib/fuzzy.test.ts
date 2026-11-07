import { describe, expect, it } from "vitest"
import { fuzzyPaths, mention, mentionAt } from "./fuzzy"

const paths = ["src/cart.js", "src/cart.test.js", "README.md", "app/src/components/chat/Composer.tsx", "docs/carting-guide.md"]

describe("fuzzyPaths", () => {
  it("ranks file-name matches first", () => {
    expect(fuzzyPaths(paths, "cart")[0]).toBe("src/cart.js")
    expect(fuzzyPaths(paths, "composer")).toEqual(["app/src/components/chat/Composer.tsx"])
  })
  it("matches subsequences and every term", () => {
    expect(fuzzyPaths(paths, "cmpsr")).toContain("app/src/components/chat/Composer.tsx")
    expect(fuzzyPaths(paths, "cart test")).toEqual(["src/cart.test.js"])
    expect(fuzzyPaths(paths, "zzz")).toEqual([])
  })
})

describe("mentions", () => {
  it("finds the @query before the caret", () => {
    expect(mentionAt("look at @src/ca", 15)).toEqual({ start: 8, query: "src/ca" })
    expect(mentionAt("email a@b", 9)).toBeNull()
    expect(mentionAt("@", 1)).toEqual({ start: 0, query: "" })
  })
  it("quotes paths with spaces", () => {
    expect(mention("my file.txt")).toBe('@"my file.txt"')
    expect(mention("a/b.ts")).toBe("@a/b.ts")
  })
})
