import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { phoneImageActions } from "./images"
import { api } from "./rpc"
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock("@tauri-apps/api/core", () => ({ invoke }))
const png = "data:image/png;base64,iVBORw0KGgo="
beforeEach(() => {
  vi.resetAllMocks()
  vi.stubGlobal("FileReader", class {
    result = ""
    onload = () => {}
    async readAsDataURL(blob: Blob) {
      this.result = `data:${blob.type};base64,${btoa(String.fromCharCode(...new Uint8Array(await blob.arrayBuffer())))}`
      this.onload()
    }
  })
  invoke.mockResolvedValue({ saved: true })
})
afterEach(() => vi.unstubAllGlobals())
describe("phone images", () => {
  it("reads agent files from the selected chat through the desktop", async () => {
    invoke.mockResolvedValue(png)
    expect(await api.readImage("chat-a", "/tmp/preview.png")).toBe(png)
    expect(invoke).toHaveBeenCalledWith("rpc", { method: "chat.readImage", params: { chatId: "chat-a", path: "/tmp/preview.png" } })
  })
  it("saves and copies bytes on the phone, without desktop file commands", async () => {
    expect(await phoneImageActions.save({ src: png, name: "/tmp/preview.png" })).toBe(true)
    expect(invoke).toHaveBeenLastCalledWith("image_action", { action: "save", name: "preview.png", mime: "image/png", data: "iVBORw0KGgo=" })
    await phoneImageActions.copy(png)
    expect(invoke).toHaveBeenLastCalledWith("image_action", { action: "copy", name: "Image.png", mime: "image/png", data: "iVBORw0KGgo=" })
  })
  it("preserves cancellation and native errors", async () => {
    invoke.mockResolvedValue({ saved: false })
    expect(await phoneImageActions.save({ src: png, name: "preview" })).toBe(false)
    invoke.mockRejectedValue(new Error("No space"))
    await expect(phoneImageActions.save({ src: png, name: "preview" })).rejects.toThrow("No space")
    await expect(phoneImageActions.copy(png)).rejects.toThrow("No space")
  })
})
