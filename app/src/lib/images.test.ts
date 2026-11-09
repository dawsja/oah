import { describe, expect, it } from "vitest"
import { imageSource, markdownUrlTransform, remoteHost } from "./images"

describe("Markdown image sources", () => {
  it.each([
    ["/tmp/oah-tux-preview/tux-options.png", "/tmp/oah-tux-preview/tux-options.png"],
    ["./assets/my%20image.png", "./assets/my image.png"],
    ["C:\\Users\\me\\picture.png", "C:\\Users\\me\\picture.png"],
    ["file:///C:/Users/me/my%20picture.png", "C:/Users/me/my picture.png"],
    ["file:///tmp/picture.png", "/tmp/picture.png"],
    ["file://localhost/tmp/picture.png", "/tmp/picture.png"],
    ["file://server/share/picture.png", "//server/share/picture.png"],
  ])("reads %s through the project backend", (src, path) => {
    expect(imageSource(src)).toEqual({ kind: "file", path })
    expect(markdownUrlTransform(src, "src", { tagName: "img" })).toBe(src)
  })

  it.each(["https://example.com/image.png", "http://localhost:3000/image.png", "blob:http://localhost/image", "data:image/png;base64,iVBORw0KGgo="])("keeps browser image URLs: %s", (src) => {
    expect(imageSource(src)).toEqual({ kind: "url", src })
    expect(markdownUrlTransform(src, "src", { tagName: "img" })).toBe(src)
  })

  it.each(["javascript:alert(1)", "data:text/html;base64,PHNjcmlwdD4=", "data:image/svg+xml;base64,PHN2Zz4=", "file:///tmp/%00.png", "file://%", ""])("rejects invalid image sources: %s", (src) => {
    expect(imageSource(src)).toBeNull()
    expect(markdownUrlTransform(src, "src", { tagName: "img" })).toBe("")
  })

  it("keeps link sanitization in place", () => {
    expect(markdownUrlTransform("file:///tmp/image.png", "href", { tagName: "a" })).toBe("")
    expect(markdownUrlTransform("javascript:alert(1)", "href", { tagName: "a" })).toBe("")
    expect(markdownUrlTransform("https://example.com", "href", { tagName: "a" })).toBe("https://example.com")
  })
})

describe("remoteHost", () => {
  it("names the server a remote image would come from", () => {
    expect(remoteHost("https://evil.example/x.png?d=secret")).toBe("evil.example")
    expect(remoteHost("http://10.0.0.2:8080/a.png")).toBe("10.0.0.2:8080")
    expect(remoteHost("//cdn.example/a.png")).toBe("cdn.example")
  })
  it("is null for what isn't fetched from a server", () => {
    for (const s of ["data:image/png;base64,AAAA", "blob:abc", "/home/me/a.png", "C:/a.png", "shot.png"]) expect(remoteHost(s)).toBe(null)
  })
})
