import { describe, expect, it, vi } from "vitest"
import { attachmentMime, MAX_IMAGE, readAttachment } from "./attachments"

describe("image attachments", () => {
  it.each(["", "application/octet-stream"])("accepts a PNG with MIME type %j", (type) => {
    expect(attachmentMime({ name: "SCREENSHOT.PNG", type })).toBe("image/png")
  })
  it("recognizes pasted images without filenames and rejects unsupported files", () => {
    expect(attachmentMime({ name: "", type: "image/png" })).toBe("image/png")
    expect(attachmentMime({ name: "image.svg", type: "image/svg+xml" })).toBeNull()
    expect(attachmentMime({ name: "fake.png", type: "text/plain" })).toBeNull()
    expect(attachmentMime({ name: "notes.txt", type: "" })).toBeNull()
  })
  it("sends raw base64 bytes with the inferred image type", async () => {
    vi.stubGlobal("FileReader", class {
      result = "data:application/octet-stream;base64,iVBORw0KGgo="
      onload?: () => void
      readAsDataURL() { this.onload?.() }
    })
    try {
      await expect(readAttachment({ name: "screen.png", type: "", size: 8 } as File)).resolves.toEqual({ name: "screen.png", mime: "image/png", data: "iVBORw0KGgo=" })
    } finally { vi.unstubAllGlobals() }
  })
  it("reports read failures and rejects oversized images", async () => {
    vi.stubGlobal("FileReader", class {
      onerror?: () => void
      readAsDataURL() { this.onerror?.() }
    })
    try {
      await expect(readAttachment({ name: "screen.png", type: "image/png", size: 8 } as File)).rejects.toThrow("Could not read screen.png")
      await expect(readAttachment({ name: "screen.png", type: "image/png", size: MAX_IMAGE + 1 } as File)).rejects.toThrow("oversized")
    } finally { vi.unstubAllGlobals() }
  })
})
