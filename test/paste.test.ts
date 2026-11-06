import { beforeEach, expect, mock, test } from "bun:test"
import * as real from "../src/images"

// Stand-in clipboard: what the next read returns.
let clip: { image: Awaited<ReturnType<typeof real.clipboardImage>>; text: string | null } = { image: null, text: null }
mock.module("../src/images", () => ({
  ...real,
  clipboardImage: async () => clip.image,
  clipboardText: async () => clip.text,
}))

const { Store } = await import("../src/ui/store")

function bareStore() {
  const s = Object.create(Store.prototype) as InstanceType<typeof Store>
  const calls = { inserted: [] as string[], attached: [] as unknown[], notes: [] as { text: string; error: boolean }[], pasted: [] as string[] }
  Object.assign(s, {
    cwd: process.cwd(),
    insertInput: (t: string) => calls.inserted.push(t),
    attachImage: (img: unknown) => calls.attached.push(img),
    notify: (text: string, error = false) => calls.notes.push({ text, error }),
  })
  return { s, calls }
}

beforeEach(() => {
  clip = { image: null, text: null }
})

test("an image on the clipboard is attached", async () => {
  const { s, calls } = bareStore()
  clip.image = { data: "AAAA", mediaType: "image/png", name: "clipboard", bytes: 3 }
  clip.text = "ignored"
  await s.pasteFromClipboard()
  expect(calls.attached).toHaveLength(1)
  expect(calls.inserted).toEqual([])
})

test("text on the clipboard is pasted into the input", async () => {
  const { s, calls } = bareStore()
  clip.text = "line one\r\nline two"
  await s.pasteFromClipboard()
  expect(calls.inserted).toEqual(["line one\nline two"])
  expect(calls.notes).toEqual([])
})

test("a copied image file path still attaches the file", async () => {
  const { s, calls } = bareStore()
  clip.text = "/tmp/shot.png"
  ;(s as any).pasteText = (t: string) => (calls.pasted.push(t), true)
  await s.pasteFromClipboard()
  expect(calls.pasted).toEqual(["/tmp/shot.png"])
  expect(calls.inserted).toEqual([])
})

test("errors and an empty clipboard are reported", async () => {
  const { s, calls } = bareStore()
  clip.image = "The image on the clipboard is 9.0 MB — images must be under 5 MB"
  await s.pasteFromClipboard()
  clip.image = null
  await s.pasteFromClipboard()
  expect(calls.notes).toEqual([
    { text: "The image on the clipboard is 9.0 MB — images must be under 5 MB", error: true },
    { text: "The clipboard is empty", error: true },
  ])
})
