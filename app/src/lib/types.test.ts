import { describe, expect, it } from "vitest"
import { isRemote, locationLabel, windowsPath } from "./types"

describe("locations", () => {
  it("names where a project is", () => {
    expect(locationLabel({ kind: "local" })).toBe("This computer")
    expect(locationLabel({ kind: "ssh", host: "devbox" })).toBe("devbox")
    expect(locationLabel({ kind: "wsl", distro: "Ubuntu" })).toBe("Ubuntu")
    expect(isRemote({ kind: "wsl", distro: "Ubuntu" })).toBe(true)
    expect(isRemote({ kind: "local" })).toBe(false)
  })

  it("spots Windows drives inside WSL", () => {
    expect(windowsPath("/mnt/c/Users/u/code")).toBe("C:\\Users\\u\\code")
    expect(windowsPath("/mnt/d")).toBe("D:\\")
    expect(windowsPath("/mnt/wsl/x")).toBeNull()
    expect(windowsPath("/home/u")).toBeNull()
  })
})
