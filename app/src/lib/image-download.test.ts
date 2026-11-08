import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { git } from "./git"
import { copyImageLocally, imageBlob, saveImageLocally } from "./image-download"

const { save, invoke } = vi.hoisted(() => ({ save: vi.fn(), invoke: vi.fn() }))
vi.mock("@tauri-apps/plugin-dialog", () => ({ save }))
vi.mock("@tauri-apps/api/core", () => ({ invoke }))

const png = "data:image/png;base64,iVBORw0KGgo="
const bytes = [137, 80, 78, 71, 13, 10, 26, 10]
const desktop = "C:\\Users\\dawso\\Desktop\\picture.png"

beforeEach(() => {
  vi.resetAllMocks()
  vi.stubGlobal("window", { __TAURI_INTERNALS__: { invoke: (cmd: string, args: unknown) => invoke(cmd, args) } })
  save.mockResolvedValue(desktop)
  invoke.mockImplementation(async (cmd) => cmd === "file_read_image" ? png : undefined)
  vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")))
})
afterEach(() => vi.unstubAllGlobals())

describe("saving images on the viewing device", () => {
  it.each([
    ["local", "C:\\work\\picture.png"],
    ["wsl", "/tmp/picture.png"],
    ["ssh", "/home/user/picture.png"],
  ])("saves an image loaded from a %s project to the chosen Windows path without fetching it again", async (projectId, path) => {
    const src = await git.readImage({ projectId, cwd: null }, path)
    expect(invoke).toHaveBeenCalledWith("file_read_image", { projectId, cwd: null, path })
    await expect(saveImageLocally({ src, name: path })).resolves.toBe(true)
    expect(save).toHaveBeenCalledWith({ defaultPath: "picture.png", title: "Save image" })
    expect(invoke).toHaveBeenLastCalledWith("save_bytes", { path: desktop, data: bytes })
    expect(fetch).not.toHaveBeenCalled()
  })

  it("keeps binary bytes and MIME type intact for copy and save", async () => {
    const blob = await imageBlob("data:image/png;base64,iVBORw0K\nGgo=")
    expect(blob.type).toBe("image/png")
    expect(Array.from(new Uint8Array(await blob.arrayBuffer()))).toEqual(bytes)
    expect(fetch).not.toHaveBeenCalled()
  })

  it("copies PNG bytes to the local clipboard even when the webview blocks fetch", async () => {
    await copyImageLocally(png)
    expect(invoke).toHaveBeenCalledWith("plugin:clipboard-manager|write_image", { image: new Uint8Array(bytes) })
    expect(fetch).not.toHaveBeenCalled()
  })

  it("reports clipboard write errors instead of claiming success", async () => {
    invoke.mockRejectedValue(new Error("Clipboard unavailable"))
    await expect(copyImageLocally(png)).rejects.toThrow("Clipboard unavailable")
  })

  it("adds the image extension to descriptive names and respects a local Linux destination", async () => {
    save.mockResolvedValue("/home/me/Desktop/preview.png")
    await saveImageLocally({ src: png, name: "Preview" })
    expect(save).toHaveBeenCalledWith({ defaultPath: "Preview.png", title: "Save image" })
    expect(invoke).toHaveBeenCalledWith("save_bytes", { path: "/home/me/Desktop/preview.png", data: bytes })
  })

  it("does not load or write anything when the dialog is cancelled", async () => {
    save.mockResolvedValue(null)
    await expect(saveImageLocally({ src: png, name: "image.png" })).resolves.toBe(false)
    expect(invoke).not.toHaveBeenCalled()
    expect(fetch).not.toHaveBeenCalled()
  })

  it("does not write corrupt data and propagates local disk errors", async () => {
    await expect(saveImageLocally({ src: "data:image/png;base64,???", name: "image.png" })).rejects.toThrow()
    expect(invoke).not.toHaveBeenCalled()
    invoke.mockRejectedValue(new Error("Access denied"))
    await expect(saveImageLocally({ src: png, name: "image.png" })).rejects.toThrow("Access denied")
  })
})
