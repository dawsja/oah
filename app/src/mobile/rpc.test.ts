import { describe, expect, it } from "vitest"
import { phoneName } from "./rpc"

const pixel = "Mozilla/5.0 (Linux; Android 16; Pixel 9 Build/BP2A.250605.031; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/140.0.0.0 Mobile Safari/537.36"
const reduced = "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Mobile Safari/537.36"
const linux = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15"

describe("the phone's name", () => {
  it("names an Android phone by its model", () => {
    expect(phoneName(pixel)).toBe("Pixel 9")
    expect(phoneName(reduced)).toBe("Android phone")
  })

  it("doesn't call anything else Android", () => {
    expect(phoneName(linux)).toBe("Phone")
  })
})
