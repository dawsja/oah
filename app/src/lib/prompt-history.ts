/** Session-local navigation. Snapshot the list while browsing so incoming messages cannot shift it. */
export class PromptHistory {
  private entries: string[] = []
  private browsing: { entries: string[]; index: number; draft: string } | null = null

  add(text: string) {
    if (text.trim() && this.entries.at(-1) !== text) this.entries.push(text)
  }

  reset() {
    this.browsing = null
  }

  move(direction: -1 | 1, current: string): string | null {
    if (!this.browsing) {
      if (direction === 1 || !this.entries.length) return null
      this.browsing = { entries: [...this.entries], index: this.entries.length, draft: current }
    }
    const b = this.browsing
    b.index = Math.max(0, Math.min(b.entries.length, b.index + direction))
    if (b.index === b.entries.length) {
      this.reset()
      return b.draft
    }
    return b.entries[b.index]
  }
}

/** Leave selection and multiline cursor movement to the textarea. */
export function historyBoundary(text: string, start: number, end: number, direction: -1 | 1) {
  return start === end && (direction === -1 ? !text.slice(0, start).includes("\n") : !text.slice(end).includes("\n"))
}
