import { describe, expect, it } from "vitest"
import { isIOS, phoneName } from "./rpc"

const pixel = "Mozilla/5.0 (Linux; Android 16; Pixel 9 Build/BP2A.250605.031; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/140.0.0.0 Mobile Safari/537.36"
const reduced = "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Mobile Safari/537.36"
const iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148"
const linux = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15"

describe("the phone's name and kind", () => {
  it("names an Android phone by its model", () => {
    expect(phoneName(pixel)).toBe("Pixel 9")
    expect(phoneName(reduced)).toBe("Android phone")
    expect(isIOS(pixel)).toBe(false)
  })

  it("knows an iPhone", () => {
    expect(phoneName(iphone)).toBe("iPhone")
    expect(isIOS(iphone)).toBe(true)
  })

  it("doesn't call anything else Android", () => {
    expect(phoneName(linux)).toBe("Phone")
    expect(isIOS(linux)).toBe(false)
  })
})

describe("an iPhone kept off the local network", () => {
  it("is recognised by how the connection fails", async () => {
    const { needsLocalNetwork } = await import("./rpc")
    expect(needsLocalNetwork("couldn't reach the desktop at 192.168.1.20:7787 (No route to host (os error 65))", true)).toBe(true)
    expect(needsLocalNetwork("oah needs Local Network access to find computers.", true)).toBe(true)
    expect(needsLocalNetwork("192.168.1.20:7787 did not answer in time.", true)).toBe(false)
    expect(needsLocalNetwork(null, true)).toBe(false)
    // Android has no such switch.
    expect(needsLocalNetwork("No route to host (os error 113)", false)).toBe(false)
  })
})
