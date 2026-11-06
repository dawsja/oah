import { expect, test } from "bun:test"
import type { ScrollBoxRenderable } from "@opentui/core"
import { testRender } from "@opentui/react/test-utils"
import { act, useRef, useState } from "react"
import { JumpToBottom, useJumpToBottom } from "../src/ui/App"

let grow: (n: number) => void = () => {}
let box: ScrollBoxRenderable | null = null

// The same shape as App: a sticky-bottom transcript, then the banner above the composer.
function Harness() {
  const scroll = useRef<ScrollBoxRenderable>(null)
  const { away, jump } = useJumpToBottom(scroll)
  const [lines, setLines] = useState(40)
  grow = (n) => setLines((x) => x + n)
  box = scroll.current
  return (
    <box flexDirection="column" width={40} height={12}>
      <scrollbox ref={scroll} flexGrow={1} stickyScroll stickyStart="bottom">
        {Array.from({ length: lines }, (_, i) => (
          <text key={i}>line {i}</text>
        ))}
      </scrollbox>
      {away ? <JumpToBottom onJump={jump} /> : null}
      <box height={3} border>
        <text>input</text>
      </box>
    </box>
  )
}

async function frame(s: Awaited<ReturnType<typeof testRender>>) {
  for (let i = 0; i < 3; i++) await act(async () => void (await s.renderOnce()))
  return s.captureCharFrame()
}

test("banner appears when scrolled up and jumps back to the bottom", async () => {
  const s = await testRender(<Harness />, { width: 40, height: 12 })
  let f = await frame(s)
  expect(f).toContain("line 39")
  expect(f).not.toContain("Jump to bottom")

  // Streaming while at the bottom: still following, no banner.
  await act(async () => grow(5))
  f = await frame(s)
  expect(f).toContain("line 44")
  expect(f).not.toContain("Jump to bottom")

  // Scroll up: banner shows, directly above the input box.
  await act(async () => box!.scrollBy(-10))
  f = await frame(s)
  const rows = f.split("\n")
  const bannerRow = rows.findIndex((r) => r.includes("Jump to bottom"))
  expect(bannerRow).toBeGreaterThan(0)
  expect(rows[bannerRow]).toContain("↓ Jump to bottom · ctrl+end")
  expect(rows[bannerRow + 1]).toMatch(/^[┌╭]/)
  expect(f).not.toContain("line 44")

  // More output arrives while scrolled up: banner stays, view doesn't move.
  await act(async () => grow(5))
  f = await frame(s)
  expect(f).toContain("Jump to bottom")
  expect(f).not.toContain("line 49")

  // Click it: back at the bottom, following again, banner gone.
  const col = rows[bannerRow]!.indexOf("Jump")
  await act(async () => void (await s.mockMouse.click(col, bannerRow)))
  f = await frame(s)
  expect(f).toContain("line 49")
  expect(f).not.toContain("Jump to bottom")
  await act(async () => grow(3))
  f = await frame(s)
  expect(f).toContain("line 52")
  expect(f).not.toContain("Jump to bottom")

  // ctrl+end does the same as clicking.
  await act(async () => box!.scrollBy(-6))
  expect(await frame(s)).toContain("Jump to bottom")
  await act(async () => grow(2))
  await act(async () => s.mockInput.pressKey("END", { ctrl: true }))
  f = await frame(s)
  expect(f).toContain("line 54")
  expect(f).not.toContain("Jump to bottom")

  // Scrolling back down by hand also clears it.
  await act(async () => box!.scrollBy(-4))
  expect(await frame(s)).toContain("Jump to bottom")
  await act(async () => box!.scrollBy(100))
  expect(await frame(s)).not.toContain("Jump to bottom")
  s.renderer.destroy()
})
