import { describe, expect, it } from "vitest"
import { fitImage, fitSize, jpegName } from "./photos"

describe("photos too large to attach", () => {
  it("scales the longer edge down, keeping the shape", () => {
    expect(fitSize(8064, 6048, 3072)).toEqual({ width: 3072, height: 2304 })
    expect(fitSize(3024, 4032, 2048)).toEqual({ width: 1536, height: 2048 })
  })

  it("never scales up", () => {
    expect(fitSize(800, 600, 3072)).toEqual({ width: 800, height: 600 })
    expect(fitSize(1, 5000, 1440)).toEqual({ width: 1, height: 1440 })
  })

  it("names the result a JPEG", () => {
    expect(jpegName("IMG_0042.HEIC")).toBe("IMG_0042.jpg")
    expect(jpegName("shot.final.png")).toBe("shot.final.jpg")
    expect(jpegName("")).toBe("photo.jpg")
  })

  it("leaves images that fit, and GIFs, as they are", async () => {
    const small = new File([new Uint8Array(10)], "a.png", { type: "image/png" })
    expect(await fitImage(small)).toBe(small)
    const gif = new File([new Uint8Array(64)], "a.gif", { type: "image/gif" })
    expect(await fitImage(gif, 16)).toBe(gif)
  })
})
