import { expect, test } from "bun:test"
import { mkdtempSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { imageFromFile, pastedPaths, referenced, sniff, userContent, type PastedImage } from "../src/images"

const PNG = Uint8Array.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0])

test("dragged-in paths: quoted, escaped, file:// and several at once", () => {
  expect(pastedPaths("'/Users/me/Screen Shot 1.png'")).toEqual(["/Users/me/Screen Shot 1.png"])
  expect(pastedPaths("/Users/me/Screen\\ Shot\\ 1.png")).toEqual(["/Users/me/Screen Shot 1.png"])
  expect(pastedPaths("file:///tmp/a%20b.png")).toEqual(["/tmp/a b.png"])
  expect(pastedPaths("/a.png /b.jpg")).toEqual(["/a.png", "/b.jpg"])
})

test("only real image files attach; anything else pastes as text", () => {
  const dir = mkdtempSync(join(tmpdir(), "oah-img-"))
  writeFileSync(join(dir, "shot.png"), PNG)
  writeFileSync(join(dir, "notes.txt"), "hi")
  const img = imageFromFile(join(dir, "shot.png"), dir)
  expect(typeof img === "object" && img?.mediaType).toBe("image/png")
  expect(imageFromFile("shot.png", dir)).not.toBeNull()
  expect(imageFromFile(join(dir, "notes.txt"), dir)).toBeNull()
  expect(imageFromFile(join(dir, "missing.png"), dir)).toBeNull()
  expect(sniff(PNG)).toBe("image/png")
})

test("[Image #N] markers become image parts in place", () => {
  const images = new Map<number, PastedImage>([[1, { n: 1, data: "QUJD", mediaType: "image/png", name: "a.png", bytes: 3 }]])
  expect(userContent("no images here", images)).toBe("no images here")
  expect(userContent("what's wrong in [Image #1] and [Image #9]?", images)).toEqual([
    { type: "text", text: "what's wrong in [Image #1]" },
    { type: "image", image: "QUJD", mediaType: "image/png" },
    { type: "text", text: " and [Image #9]?" },
  ])
  expect(referenced("[Image #1] [Image #1] [Image #2]", images)).toEqual([1])
})
